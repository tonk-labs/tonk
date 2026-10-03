//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;
use dialog_artifacts::Index;
use dialog_capability::Provider;
use dialog_common::Blake3Hash as NodeHash;
use dialog_common::ConditionalSync;
use dialog_repository::{NetworkedIndex, PushError, Revision, Upstream};
use dialog_search_tree::{DialogSearchTreeError, LoadBlock, TreeDifference};

// Dialog reports a tree node it cannot load only by this message; see the
// follow-up to give it an error variant of its own.
const MISSING_LOCAL_TREE_NODE: &str = "Block not found in storage:";

fn is_missing_local_tree_node(error: &PushError) -> bool {
    matches!(
        error,
        PushError::Tree(DialogSearchTreeError::Node(message))
            if message.starts_with(MISSING_LOCAL_TREE_NODE)
    )
}

/// Materialize the search-tree nodes push's novelty diff will visit —
/// the divergent paths between `base` and `current` — through `store`,
/// which loads a node locally or fetches and caches it from the remote.
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
    store: &S,
) -> Result<(), DialogSearchTreeError>
where
    S: Provider<LoadBlock> + ConditionalSync,
{
    let base_tree = Index::from_hash(base.clone());
    let current_tree = Index::from_hash(current.clone());
    // The compute itself performs every read: each node it expands passes
    // through the caching store, so by the time it returns, the local
    // store holds the divergent paths and the retry's local-only diff
    // cannot miss.
    TreeDifference::compute(&base_tree, &current_tree, store, store).await?;
    Ok(())
}

/// Push-to-upstream effect.
pub struct Push<'a> {
    /// The branch to push from.
    pub branch: BranchReference<'a>,
    /// Whether to confirm where upstream stands before pushing.
    confirm_upstream: bool,
}

impl<'a> Push<'a> {
    /// Build a new `Push` effect.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            confirm_upstream: true,
        }
    }

    /// Push without first confirming where upstream stands, for a
    /// caller that just read it. See
    /// [`dialog_repository::Push::assuming_upstream`] for what that
    /// gives up: the novelty ships before a doomed push is refused, and
    /// the refusal is a version mismatch rather than a
    /// non-fast-forward.
    pub fn assuming_upstream(mut self) -> Self {
        self.confirm_upstream = false;
        self
    }

    /// Execute the push.
    ///
    /// Answers with the revision upstream now stands at, or `None` when
    /// there was nothing to push. A caller that would otherwise read the
    /// upstream head back — to colour a status, to compare against local
    /// — takes it from here rather than paying another round trip for a
    /// cell this call just settled.
    pub async fn perform<Env>(self, env: &Env) -> Result<Option<Revision>, ReactorError>
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
        let push = || {
            let push = cached.handle().push();
            if self.confirm_upstream {
                push
            } else {
                push.assuming_upstream()
            }
        };

        let error = match push().perform(env).await {
            Ok(pushed) => return Ok(pushed),
            Err(error) if is_missing_local_tree_node(&error) => error,
            Err(error) => return Err(error.into()),
        };

        let Some((remote, base)) =
            cached
                .handle()
                .pushes()
                .iter()
                .find_map(|upstream| match upstream {
                    // A branch never synced with its upstream has no
                    // divergence to hydrate: the retry would fail the same.
                    Upstream::Remote {
                        remote,
                        tree: Some(tree),
                        ..
                    } => Some((remote.clone(), tree.clone())),
                    _ => None,
                })
        else {
            return Err(error.into());
        };
        let Some(revision) = cached.handle().revision() else {
            return Err(error.into());
        };
        let store = NetworkedIndex::new(env, cached.handle().archive().index(), Some(remote));
        // The diff can traverse either side, so both sides hydrate through
        // the networked index; only tree nodes are cached — referenced blob
        // payloads are still transferred by push's normal shipment phase.
        hydrate_divergence(
            &NodeHash::from(*base.hash()),
            &NodeHash::from(*revision.tree.hash()),
            &store,
        )
        .await
        .map_err(PushError::from)?;

        // The retry confirms upstream regardless: the first attempt
        // failed on a shape that means our local view was incomplete,
        // so this is no longer the caller's "I just read it" case.
        Ok(cached.handle().push().perform(env).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dialog_artifacts::tree::ArtifactTreeExt as _;
    use dialog_artifacts::{ArchiveDelta, Artifact, Instruction, Value};
    use dialog_common::Buffer;
    use dialog_search_tree::MemoryBlocks;
    use futures_util::stream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A small stand-in for `NetworkedIndex`: local reads first, then remote,
    /// caching a remote hit locally on the way back — and counting every
    /// remote hit, so a test can bound how much the repair replicated.
    #[derive(Clone)]
    struct CachingStore {
        local: MemoryBlocks,
        remote: MemoryBlocks,
        remote_reads: Arc<AtomicUsize>,
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Provider<LoadBlock> for CachingStore {
        async fn execute(
            &self,
            LoadBlock { hash }: LoadBlock,
        ) -> Result<Option<Buffer>, DialogSearchTreeError> {
            if let Some(block) = self.local.get(&hash) {
                return Ok(Some(block));
            }
            let Some(block) = self.remote.get(&hash) else {
                return Ok(None);
            };
            self.remote_reads.fetch_add(1, Ordering::Relaxed);
            self.local.store(block.clone());
            Ok(Some(block))
        }
    }

    /// Seed `count` items starting at `offset` into `tree`, storing the
    /// flushed nodes in `remote`, and return how many blocks were stored.
    async fn seed(
        tree: &mut Index,
        remote: &MemoryBlocks,
        offset: usize,
        count: usize,
    ) -> anyhow::Result<usize> {
        let mut delta = ArchiveDelta::zero();
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
    async fn it_hydrates_only_the_divergent_paths_before_push() -> anyhow::Result<()> {
        let remote = MemoryBlocks::new();
        let mut tree = Index::empty();

        // A wide base the two heads share, then a single-item divergence:
        // the shape of a lazily adopted branch pushing one commit.
        let base_blocks = seed(&mut tree, &remote, 0, 2000).await?;
        let base_root = tree.root().clone();
        let novel_blocks = seed(&mut tree, &remote, 2000, 1).await?;
        let current_root = tree.root().clone();

        let remote_reads = Arc::new(AtomicUsize::new(0));
        let local = MemoryBlocks::new();
        hydrate_divergence(
            &base_root,
            &current_root,
            &CachingStore {
                local: local.clone(),
                remote,
                remote_reads: remote_reads.clone(),
            },
        )
        .await?;

        let base = Index::from_hash(base_root);
        let current = Index::from_hash(current_root);
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

    #[test]
    fn it_only_retries_missing_local_tree_nodes() {
        assert!(is_missing_local_tree_node(&PushError::Tree(
            DialogSearchTreeError::Node("Block not found in storage: blake3#missing".to_owned())
        )));
        assert!(!is_missing_local_tree_node(&PushError::Tree(
            DialogSearchTreeError::Operation("tree is invalid".to_owned())
        )));
    }
}
