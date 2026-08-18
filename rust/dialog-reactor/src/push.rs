//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;
use dialog_artifacts::tree::TreeStorageBridge;
use dialog_artifacts::{Index, Key};
use dialog_common::Blake3Hash as NodeHash;
use dialog_repository::{
    NetworkedIndex, PushError, RepositoryArchiveExt as _, RepositoryMemoryExt as _, Upstream,
};
use dialog_search_tree::{ContentAddressedStorage as TreeStorage, DialogSearchTreeError};
use dialog_storage::{Blake3Hash, DialogStorageError, StorageBackend};
use futures_util::TryStreamExt as _;

const MISSING_LOCAL_TREE_NODE: &str = "Blob not found in storage:";

fn is_missing_local_tree_node(error: &PushError) -> bool {
    matches!(
        error,
        PushError::Tree(DialogSearchTreeError::Node(message))
            if message.starts_with(MISSING_LOCAL_TREE_NODE)
    )
}

/// Materialize every search-tree node reachable from `roots` through a
/// storage backend that may fetch and cache remote misses.
async fn hydrate_tree_roots<S>(roots: &[NodeHash], store: S) -> Result<(), DialogSearchTreeError>
where
    S: StorageBackend<Key = Blake3Hash, Value = Vec<u8>, Error = DialogStorageError>
        + Clone
        + dialog_common::ConditionalSync,
{
    let storage = TreeStorage::new(TreeStorageBridge(store));
    for root in roots {
        let tree = Index::from_hash(root.clone());
        let entries = tree.stream_range(Key::min()..=Key::max(), &storage);
        tokio::pin!(entries);
        while entries.try_next().await?.is_some() {}
    }
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

        // Dialog's push novelty diff currently reads through a local-only
        // index. A branch adopted from a remote may therefore have a valid
        // head while some untouched search-tree nodes are still remote-only.
        // Keep the normal path cheap and repair that one typed failure on
        // demand. Uploads before the failure are content-addressed, so retrying
        // after hydration is idempotent.
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
        let roots = [
            NodeHash::from(*base.hash()),
            NodeHash::from(*revision.tree.hash()),
        ];
        // The diff can traverse either side. Walking both roots through the
        // networked index caches only tree nodes; referenced blob payloads are
        // still transferred by push's normal shipment phase.
        hydrate_tree_roots(&roots, store)
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
    /// caching a remote hit locally on the way back.
    #[derive(Clone)]
    struct CachingStore {
        local: Memory,
        remote: Memory,
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
            let mut local = self.local.clone();
            local.set(*key, bytes.clone()).await?;
            Ok(Some(bytes))
        }
    }

    #[dialog_common::test]
    async fn it_hydrates_a_lazy_tree_into_local_storage_before_push() -> anyhow::Result<()> {
        let mut remote = Memory::default();
        let mut delta = Delta::zero();
        let mut tree = Index::empty();
        let instructions = (0..300).map(|index| {
            Instruction::Assert(Artifact {
                the: "item/title".parse().unwrap(),
                of: format!("item:{index}").parse().unwrap(),
                is: Value::String(format!("Item {index}")),
                cause: None,
            })
        });
        tree.apply(&mut remote, &mut delta, stream::iter(instructions))
            .await?;
        for (_, buffer) in delta.flush() {
            remote
                .set(*buffer.blake3_hash().as_bytes(), buffer.as_ref().to_vec())
                .await?;
        }

        let base_root = tree.root().clone();
        let mut delta = Delta::zero();
        let instructions = (300..600).map(|index| {
            Instruction::Assert(Artifact {
                the: "item/title".parse().unwrap(),
                of: format!("item:{index}").parse().unwrap(),
                is: Value::String(format!("Item {index}")),
                cause: None,
            })
        });
        tree.apply(&mut remote, &mut delta, stream::iter(instructions))
            .await?;
        for (_, buffer) in delta.flush() {
            remote
                .set(*buffer.blake3_hash().as_bytes(), buffer.as_ref().to_vec())
                .await?;
        }

        let current_root = tree.root().clone();
        let local = Memory::default();
        hydrate_tree_roots(
            &[base_root.clone(), current_root.clone()],
            CachingStore {
                local: local.clone(),
                remote,
            },
        )
        .await?;

        let storage = TreeStorage::new(TreeStorageBridge(local));
        let base = Index::from_hash(base_root);
        let current = Index::from_hash(current_root);
        if let Err(error) = TreeDifference::compute(&base, &current, &storage, &storage).await {
            panic!("push's local-only tree differential must succeed after hydration: {error:?}");
        }
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
