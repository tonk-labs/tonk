//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;
use super::hydrate::hydrate_divergence;
use dialog_common::Blake3Hash as NodeHash;
use dialog_repository::{PushError, Revision, Upstream};
use dialog_search_tree::DialogSearchTreeError;

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
