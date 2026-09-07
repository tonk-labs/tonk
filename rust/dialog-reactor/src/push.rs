//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;
use dialog_artifacts::Index;
use dialog_artifacts::tree::TreeStorageBridge;
use dialog_common::Blake3Hash as NodeHash;
use dialog_repository::{
    NetworkedIndex, PushError, RepositoryArchiveExt as _, RepositoryMemoryExt as _, Upstream,
};
use dialog_search_tree::{
    ContentAddressedStorage as TreeStorage, DialogSearchTreeError, TreeDifference,
};
use dialog_storage::{Blake3Hash, DialogStorageError, StorageBackend};

const MISSING_LOCAL_TREE_NODE: &str = "Blob not found in storage:";

fn is_missing_local_tree_node(error: &PushError) -> bool {
    matches!(
        error,
        PushError::Tree(DialogSearchTreeError::Node(message))
            if message.starts_with(MISSING_LOCAL_TREE_NODE)
    )
}

/// Materialize the search-tree nodes push's novelty diff will visit —
/// the divergent paths between `base` and `current` — through a storage
/// backend that fetches and caches remote misses.
///
/// The differential prunes identical subtrees by hash without reading
/// them, so this walk touches (and therefore fetches) only the changed
/// paths plus their spines: the same node set the retried local diff
/// reads. Streaming the whole tree here instead — the previous repair —
/// re-replicated the entire space one authorized round trip per block to
/// satisfy a diff that needed a handful of nodes.
async fn hydrate_divergence<S>(
    base: &NodeHash,
    current: &NodeHash,
    store: S,
) -> Result<(), DialogSearchTreeError>
where
    S: StorageBackend<Key = Blake3Hash, Value = Vec<u8>, Error = DialogStorageError>
        + Clone
        + dialog_common::ConditionalSync,
{
    let storage = TreeStorage::new(TreeStorageBridge(store));
    let base_tree = Index::from_hash(base.clone());
    let current_tree = Index::from_hash(current.clone());
    // The compute itself performs every read: each node it expands passes
    // through the caching backend, so by the time it returns, the local
    // store holds the divergent paths and the retry's local-only diff
    // cannot miss.
    TreeDifference::compute(&base_tree, &current_tree, &storage, &storage).await?;
    Ok(())
}

/// Push-to-upstream effect.
pub struct Push<'a> {
    /// The branch to push from.
    pub branch: BranchReference<'a>,
}

impl<'a> Push<'a> {
    /// Build a new `Push` effect.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self { branch }
    }

    /// Execute the push.
    pub async fn perform<Env>(self, env: &Env) -> Result<(), ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + PushProvider,
    {
        let cached = self.branch.acquire(env).await?;

        // Dialog's push novelty diff reads through a local-only index with
        // a boundary-tolerant missing policy, so a lazily adopted branch
        // normally pushes without any hydration. A shape it cannot absorb
        // still surfaces as one typed failure; keep the normal path cheap
        // and repair exactly that on demand, by hydrating only the
        // divergent paths the diff visits. Uploads before the failure are
        // content-addressed, so retrying after hydration is idempotent.
        let error = match cached.handle().push().perform(env).await {
            Ok(_) => return Ok(()),
            Err(error) if is_missing_local_tree_node(&error) => error,
            Err(error) => return Err(error.into()),
        };

        let (remote_name, base) = match cached.handle().upstream() {
            Some(Upstream::Remote { remote, tree, .. }) => (remote, tree),
            _ => return Err(error.into()),
        };
        let Some(revision) = cached.handle().revision() else {
            return Err(error.into());
        };
        let remote = cached
            .handle()
            .subject()
            .remote(remote_name)
            .load()
            .perform(env)
            .await
            .map_err(PushError::from)?;
        let store = NetworkedIndex::new(env, cached.handle().archive().index(), Some(remote));
        // The diff can traverse either side, so both sides hydrate through
        // the networked index; only tree nodes are cached — referenced blob
        // payloads are still transferred by push's normal shipment phase.
        hydrate_divergence(
            &NodeHash::from(*base.hash()),
            &NodeHash::from(*revision.tree.hash()),
            store,
        )
        .await
        .map_err(PushError::from)?;

        cached.handle().push().perform(env).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dialog_artifacts::tree::ArtifactTreeExt as _;
    use dialog_artifacts::{Artifact, Instruction, Value};
    use dialog_search_tree::{Delta, TreeDifference};
    use dialog_storage::MemoryStorageBackend;
    use futures_util::stream;

    type Memory = MemoryStorageBackend<Blake3Hash, Vec<u8>>;

    /// A small stand-in for `NetworkedIndex`: local reads first, then remote,
    /// caching a remote hit locally on the way back — and counting every
    /// remote hit, so a test can bound how much the repair replicated.
    #[derive(Clone)]
    struct CachingStore {
        local: Memory,
        remote: Memory,
        remote_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl StorageBackend for CachingStore {
        type Key = Blake3Hash;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            self.local.set(key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            if let Some(bytes) = self.local.get(key).await? {
                return Ok(Some(bytes));
            }
            let Some(bytes) = self.remote.get(key).await? else {
                return Ok(None);
            };
            self.remote_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut local = self.local.clone();
            local.set(*key, bytes.clone()).await?;
            Ok(Some(bytes))
        }
    }

    /// Seed `count` items starting at `offset` into `tree`, persisting the
    /// flushed nodes into `remote`, and return how many blocks were stored.
    async fn seed(
        tree: &mut Index,
        remote: &mut Memory,
        offset: usize,
        count: usize,
    ) -> anyhow::Result<usize> {
        let mut delta = Delta::zero();
        let instructions = (offset..offset + count).map(|index| {
            Instruction::Assert(Artifact {
                the: "item/title".parse().unwrap(),
                of: format!("item:{index}").parse().unwrap(),
                is: Value::String(format!("Item {index}")),
                cause: None,
            })
        });
        tree.apply(remote, &mut delta, stream::iter(instructions))
            .await?;
        let mut stored = 0;
        for (_, buffer) in delta.flush() {
            remote
                .set(*buffer.blake3_hash().as_bytes(), buffer.as_ref().to_vec())
                .await?;
            stored += 1;
        }
        Ok(stored)
    }

    /// The repair hydrates enough for push's local-only differential to
    /// succeed, while fetching only the divergent paths — not the whole
    /// tree, which is what the previous full-scan repair replicated.
    #[dialog_common::test]
    async fn it_hydrates_only_the_divergent_paths_before_push() -> anyhow::Result<()> {
        let mut remote = Memory::default();
        let mut tree = Index::empty();

        // A wide base the two heads share, then a single-item divergence:
        // the shape of a lazily adopted branch pushing one commit.
        let base_blocks = seed(&mut tree, &mut remote, 0, 2000).await?;
        let base_root = tree.root().clone();
        let novel_blocks = seed(&mut tree, &mut remote, 2000, 1).await?;
        let current_root = tree.root().clone();

        let remote_reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let local = Memory::default();
        hydrate_divergence(
            &base_root,
            &current_root,
            CachingStore {
                local: local.clone(),
                remote,
                remote_reads: remote_reads.clone(),
            },
        )
        .await?;

        let storage = TreeStorage::new(TreeStorageBridge(local));
        let base = Index::from_hash(base_root);
        let current = Index::from_hash(current_root);
        if let Err(error) = TreeDifference::compute(&base, &current, &storage, &storage).await {
            panic!("push's local-only tree differential must succeed after hydration: {error:?}");
        }

        // The single-commit divergence touches one spine of each head; the
        // shared bulk must stay remote. Half the store is a generous bound
        // that still fails loudly if the repair regresses to a full scan.
        let fetched = remote_reads.load(std::sync::atomic::Ordering::Relaxed);
        let total = base_blocks + novel_blocks;
        assert!(
            fetched < total / 2,
            "hydration should fetch only divergent paths: fetched {fetched} of {total} blocks"
        );
        Ok(())
    }

    #[test]
    fn it_only_retries_missing_local_tree_nodes() {
        assert!(is_missing_local_tree_node(&PushError::Tree(
            DialogSearchTreeError::Node("Blob not found in storage: blake3#missing".to_owned())
        )));
        assert!(!is_missing_local_tree_node(&PushError::Tree(
            DialogSearchTreeError::Operation("tree is invalid".to_owned())
        )));
    }
}
