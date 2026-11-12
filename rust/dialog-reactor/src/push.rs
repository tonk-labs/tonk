//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;
use dialog_capability::Provider;
use dialog_common::Blake3Hash as NodeHash;
use dialog_common::ConditionalSync;
use dialog_corpus::Index;
use dialog_effects::archive::prelude::CatalogScope;
use dialog_repository::{PushError, Revision, Upstream};
use dialog_search_tree::{DialogSearchTreeError, TreeDifference};

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
/// the divergent paths between `base` and `current` in `catalog` — by
/// reading them through `store`, whose load answers a node locally or
/// hydrates it from the remote and caches it.
///
/// The differential prunes identical subtrees by hash without reading
/// them, so this walk touches (and therefore fetches) only the changed
/// paths plus their spines: the same node set the retried local diff
/// reads. Streaming the whole tree here instead — the previous repair —
/// re-replicated the entire space one authorized round trip per block to
/// satisfy a diff that needed a handful of nodes.
async fn hydrate_divergence<S>(
    catalog: &CatalogScope,
    base: &NodeHash,
    current: &NodeHash,
    store: &S,
) -> Result<(), DialogSearchTreeError>
where
    S: Provider<dialog_search_tree::nodes::Read<dialog_corpus::tree::CorpusNode>>
        + Provider<dialog_search_tree::nodes::Preload<dialog_corpus::tree::CorpusNode>>
        + ConditionalSync,
{
    let base_tree = Index::at(catalog.clone(), base.clone());
    let current_tree = Index::at(catalog.clone(), current.clone());
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

        let Some(base) = cached
            .handle()
            .pushes()
            .iter()
            .find_map(|upstream| match upstream {
                // A branch never synced with its upstream has no
                // divergence to hydrate: the retry would fail the same.
                Upstream::Remote {
                    tree: Some(tree), ..
                } => Some(tree.clone()),
                _ => None,
            })
        else {
            return Err(error.into());
        };
        let Some(revision) = cached.handle().revision() else {
            return Err(error.into());
        };
        // The diff can traverse either side, so both sides hydrate through
        // the env from the remote the open branch tracks; only tree nodes
        // are cached — referenced blob payloads are still transferred by
        // push's normal shipment phase.
        hydrate_divergence(
            &cached.handle().archive().index(),
            &NodeHash::from(*base.hash()),
            &NodeHash::from(*revision.tree.hash()),
            env,
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
    use dialog_capability::Capability;
    use dialog_common::{Buffer, Held, Holds};
    use dialog_corpus::tree::CorpusExt as _;
    use dialog_corpus::{ArchiveDelta, Fact, Instruction, Value};
    use dialog_effects::archive::ArchiveError;
    use dialog_effects::archive::prelude::GetExt as _;
    use dialog_effects::archive::{Get, Load};
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

    #[dialog_capability::provider]
    impl Provider<Load> for CachingStore {
        async fn execute(&self, load: Load) -> Result<Option<Buffer>, ArchiveError> {
            if let Some(block) = self.local.get(&load.digest) {
                return Ok(Some(block));
            }
            if load.local {
                return Ok(None);
            }
            let Some(block) = self.remote.get(&load.digest) else {
                return Ok(None);
            };
            self.remote_reads.fetch_add(1, Ordering::Relaxed);
            self.local.store(block.clone());
            Ok(Some(block))
        }
    }

    #[dialog_capability::provider]
    impl Provider<Get> for CachingStore {
        async fn execute(&self, get: Capability<Get>) -> Result<Option<Vec<u8>>, ArchiveError> {
            Ok(self.local.get(get.digest()).map(Buffer::into_vec))
        }
    }

    dialog_search_tree::reads_nodes_through_blocks!(CachingStore);

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
                Fact {
                    the: "item/title".parse().unwrap(),
                    of: format!("item:{index}").parse().unwrap(),
                    is: Value::String(format!("Item {index}")),
                    cause: None,
                },
                dialog_corpus::Pick::All,
            )
        });
        tree.apply(remote, &mut delta, stream::iter(instructions))
            .await?;
        let mut stored = 0;
        for node in delta.flush_nodes() {
            remote.store(node.buffer().clone());
            stored += 1;
        }
        Ok(stored)
    }

    /// The repair hydrates enough for push's local-only differential to
    /// succeed, while fetching only the divergent paths — not the whole
    /// tree, which is what the previous full-scan repair replicated.
    #[dialog_common::test]
    async fn it_hydrates_only_the_divergent_paths_before_push() -> anyhow::Result<()> {
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
