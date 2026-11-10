//! Read the nodes two trees differ by into the local archive.

use dialog_artifacts::Index;
use dialog_capability::Provider;
use dialog_common::Blake3Hash as NodeHash;
use dialog_common::{ConditionalSync, Holds};
use dialog_effects::archive::prelude::CatalogScope;
use dialog_effects::archive::{Get, Load};
use dialog_search_tree::{DialogSearchTreeError, TreeDifference};

/// Materialize the search-tree nodes by which `base` and `current`
/// differ in `catalog`, by reading them through `store`, whose load
/// answers a node locally or hydrates it from the remote and caches it.
///
/// The differential prunes identical subtrees by hash without reading
/// them, so this walk touches (and therefore fetches) only the changed
/// paths plus their spines, then every node `current` holds that `base`
/// does not. The number of reads follows the size of the change, never
/// the size of the trees.
pub(crate) async fn hydrate_divergence<S>(
    catalog: &CatalogScope,
    base: &NodeHash,
    current: &NodeHash,
    store: &S,
) -> Result<(), DialogSearchTreeError>
where
    S: Provider<Load> + Provider<Get> + Holds + ConditionalSync,
{
    let base_tree = Index::at(catalog.clone(), base.clone());
    let current_tree = Index::at(catalog.clone(), current.clone());
    // The compute itself performs every read: each node it expands passes
    // through the caching store, so by the time it returns, the local
    // store holds the divergent paths.
    TreeDifference::compute(&base_tree, &current_tree, store, store).await?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use async_trait::async_trait;
    use dialog_artifacts::tree::ArtifactTreeExt as _;
    use dialog_artifacts::{ArchiveDelta, Artifact, Instruction, Value};
    use dialog_capability::Capability;
    use dialog_common::{Buffer, Held};
    use dialog_effects::archive::ArchiveError;
    use dialog_effects::archive::prelude::GetExt as _;
    use dialog_search_tree::helpers::Blocks;
    use futures_util::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A small stand-in for a hydrating env: a load reads locally first,
    /// then remote, caching a remote hit locally on the way back — and
    /// counting every remote hit, so a test can bound how much the repair
    /// replicated. A `Get` is the local archive's own word, and it holds
    /// nothing, so every node read is a load.
    #[derive(Clone)]
    struct CachingStore {
        local: Blocks,
        remote: Blocks,
        remote_reads: Arc<AtomicUsize>,
    }

    impl Holds for CachingStore {
        fn held(&self, _key: &str) -> Option<Held> {
            None
        }

        fn hold(&self, _key: String, _handle: Held) {}
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Provider<Load> for CachingStore {
        async fn execute(&self, load: Load) -> Result<Option<Buffer>, ArchiveError> {
            if let Some(block) = self.local.get(&load.digest) {
                return Ok(Some(block));
            }
            let Some(block) = self.remote.get(&load.digest) else {
                return Ok(None);
            };
            self.remote_reads.fetch_add(1, Ordering::Relaxed);
            self.local.store(block.clone());
            Ok(Some(block))
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Provider<Get> for CachingStore {
        async fn execute(&self, get: Capability<Get>) -> Result<Option<Vec<u8>>, ArchiveError> {
            Ok(self.local.get(get.digest()).map(Buffer::into_vec))
        }
    }

    /// Seed `count` items starting at `offset` into `tree`, storing the
    /// flushed nodes in `remote`, and return how many blocks were stored.
    async fn seed(
        tree: &mut Index,
        remote: &Blocks,
        offset: usize,
        count: usize,
    ) -> anyhow::Result<usize> {
        let mut delta = ArchiveDelta::zero();
        let instructions = (offset..offset + count).map(|index| {
            Instruction::Assert(
                Artifact {
                    the: "item/title".parse().unwrap(),
                    of: format!("item:{index}").parse().unwrap(),
                    is: Value::String(format!("Item {index}")),
                    cause: None,
                },
                dialog_artifacts::Pick::All,
            )
        });
        tree.apply(remote, &mut delta, stream::iter(instructions))
            .await?;
        let mut stored = 0;
        for block in delta.flush_blocks() {
            remote.store(block);
            stored += 1;
        }
        Ok(stored)
    }

    /// The repair hydrates enough for push's local-only differential to
    /// succeed, while fetching only the divergent paths — not the whole
    /// tree, which is what the previous full-scan repair replicated.
    #[dialog_common::test]
    async fn it_hydrates_only_the_divergent_paths() -> anyhow::Result<()> {
        let remote = Blocks::new();
        let catalog = remote.catalog().clone();
        let mut tree = Index::empty_in(catalog.clone());

        // A wide base the two heads share, then a single-item divergence:
        // the shape of a lazily adopted branch pushing one commit.
        let base_blocks = seed(&mut tree, &remote, 0, 2000).await?;
        let base_root = tree.root().clone();
        let novel_blocks = seed(&mut tree, &remote, 2000, 1).await?;
        let current_root = tree.root().clone();

        let remote_reads = Arc::new(AtomicUsize::new(0));
        let local = Blocks::in_catalog(catalog.clone());
        hydrate_divergence(
            &catalog,
            &base_root,
            &current_root,
            &CachingStore {
                local: local.clone(),
                remote,
                remote_reads: remote_reads.clone(),
            },
        )
        .await?;

        let base = Index::at(catalog.clone(), base_root);
        let current = Index::at(catalog, current_root);
        if let Err(error) = TreeDifference::compute(&base, &current, &local, &local).await {
            panic!("push's local-only tree differential must succeed after hydration: {error:?}");
        }

        // The single-commit divergence touches one spine of each head; the
        // shared bulk must stay remote. Half the store is a generous bound
        // that still fails loudly if the repair regresses to a full scan.
        let fetched = remote_reads.load(Ordering::Relaxed);
        let total = base_blocks + novel_blocks;
        assert!(
            fetched < total / 2,
            "hydration should fetch only divergent paths: fetched {fetched} of {total} blocks"
        );
        Ok(())
    }
}
