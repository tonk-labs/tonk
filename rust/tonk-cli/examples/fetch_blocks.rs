//! Fetches tree blocks of a joined space from its upstream by hash.
//!
//! Reads base58 block hashes, one per line, from a file and writes each
//! block the upstream's archive holds to `<out>/<hash>`. With `--closure`
//! it also fetches every node the fetched index nodes link to, so the
//! whole tree under each fetched node lands on disk. The upstream keeps
//! every block ever pushed, superseded ones included, so this recovers a
//! replica's history that a lazily fetched copy never downloaded.
//!
//! ```sh
//! cargo run --release -p tonk-cli --example fetch_blocks -- \
//!   <space> <hashes.txt> <out> [--closure]
//! ```
use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Context as _;
use base58::{FromBase58, ToBase58};
use dialog_repository::Upstream;
use dialog_search_tree::{NodeBody, PersistentNode};
use futures_util::StreamExt as _;

type Node = PersistentNode<dialog_artifacts::Key, dialog_artifacts::State<dialog_artifacts::Datum>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let name = args
        .next()
        .context("usage: fetch_blocks <space> <hashes> <out> [--closure]")?;
    let list = PathBuf::from(args.next().context("missing hashes file")?);
    let out = PathBuf::from(args.next().context("missing output directory")?);
    let closure = args.next().as_deref() == Some("--closure");
    std::fs::create_dir_all(&out)?;

    let store = tonk_cli::space::SpaceStore::open()?;
    let resolved = store.resolve(Some(&name), None, None)?;
    let binding = tonk_cli::connections::binding_at(&resolved.site)?
        .context("the space has no connection binding")?;
    let site = tonk_cli::connections::open_bound(&resolved.site, &binding, store).await?;
    let session = site.branch().await?;
    let upstreams = session.handle().upstreams();
    let remote = upstreams
        .iter()
        .find_map(|upstream| match upstream {
            Upstream::Remote { remote, .. } => Some(remote.clone()),
            _ => None,
        })
        .context("the branch has no remote upstream")?;
    let archive = remote.archive();
    let index = archive.index();

    let mut pending: Vec<String> = std::fs::read_to_string(&list)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    let mut seen: HashSet<String> = pending.iter().cloned().collect();
    let (mut fetched, mut missing, mut bytes) = (0usize, 0usize, 0usize);
    let started = std::time::Instant::now();
    while !pending.is_empty() {
        let batch: Vec<String> = std::mem::take(&mut pending);
        let results: Vec<(String, anyhow::Result<Option<Vec<u8>>>)> =
            futures_util::stream::iter(batch.into_iter().map(|name| {
                let index = &index;
                let operator = &site.operator;
                let target = out.join(&name);
                async move {
                    if let Ok(existing) = std::fs::read(&target) {
                        return (name, Ok(Some(existing)));
                    }
                    let result = async {
                        let raw = name
                            .from_base58()
                            .map_err(|error| anyhow::anyhow!("{name} is not base58: {error:?}"))?;
                        let hash: [u8; 32] = raw
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("{name} is not a 32-byte hash"))?;
                        let block = index
                            .get(dialog_storage::Blake3Hash::from(hash))
                            .perform(operator)
                            .await
                            .map_err(|error| anyhow::anyhow!("fetch {name}: {error:?}"))?;
                        if let Some(block) = &block {
                            std::fs::write(&target, block)?;
                        }
                        Ok(block)
                    }
                    .await;
                    (name, result)
                }
            }))
            .buffer_unordered(32)
            .collect()
            .await;
        for (name, result) in results {
            match result {
                Ok(Some(block)) => {
                    fetched += 1;
                    bytes += block.len();
                    if closure
                        && let Ok(node) = Node::try_from(dialog_common::Buffer::from(block))
                        && let NodeBody::Index(index) = node.body()
                    {
                        for at in 0..index.len() {
                            let child = index.hash_at(at)?.as_bytes().to_base58();
                            if seen.insert(child.clone()) {
                                pending.push(child);
                            }
                        }
                    }
                }
                Ok(None) => {
                    missing += 1;
                    eprintln!("{name}: not in the upstream archive");
                }
                Err(error) => {
                    missing += 1;
                    eprintln!("{error:#}");
                }
            }
        }
        eprintln!(
            "{fetched} blocks ({:.1} MiB), {missing} missing, {} queued, {:.0}s",
            bytes as f64 / (1024.0 * 1024.0),
            pending.len(),
            started.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
