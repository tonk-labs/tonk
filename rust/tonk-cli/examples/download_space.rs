//! Downloads every block and blob a joined space's head references.
//!
//! A joined space adopts its upstream's head by reference and fetches
//! tree nodes as reads need them, so its archive holds only what has been
//! read. This materializes the whole head tree and its spilled values in
//! the local store, for tools that read the store directly, such as
//! dialog's `single_file_census history`.
//!
//! ```sh
//! cargo run --release -p tonk-cli --example download_space -- <space>
//! ```
use anyhow::Context as _;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let name = std::env::args()
        .nth(1)
        .context("usage: download_space <space>")?;
    let store = tonk_cli::space::SpaceStore::open()?;
    let resolved = store.resolve(Some(&name), None, None)?;
    let binding = tonk_cli::connections::binding_at(&resolved.site)?
        .context("the space has no connection binding")?;
    let site = tonk_cli::connections::open_bound(&resolved.site, &binding, store).await?;
    let session = site.branch().await?;
    let started = std::time::Instant::now();
    session
        .handle()
        .download()
        .perform(&site.operator)
        .await
        .map_err(|error| anyhow::anyhow!("download: {error:?}"))?;
    eprintln!(
        "downloaded head of {name} into {} in {:.1}s",
        resolved.site.display(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
