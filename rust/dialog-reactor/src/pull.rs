//! [`Pull`] — wrap [`dialog_repository::Branch::pull`] so
//! subscriptions re-poll once remote artifacts have been
//! merged into the branch.

use dialog_repository::Revision;

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PullProvider, SelectProvider};
use super::error::ReactorError;

/// Pull-from-upstream effect.
pub struct Pull<'a> {
    /// The branch to pull into.
    pub branch: BranchReference<'a>,
    /// Materialize the adopted revision locally after the pull.
    download: bool,
}

impl<'a> Pull<'a> {
    /// Build a new `Pull` effect.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            download: false,
        }
    }

    /// Materialize every block and blob the adopted revision references
    /// after the pull. Required for authorization-bearing branches (the
    /// access branch, the account): their walks read entirely locally at
    /// session open, so a head adopted by reference with blocks still
    /// remote bricks the next boot.
    pub fn download(mut self) -> Self {
        self.download = true;
        self
    }

    /// Execute the pull. Subscriptions are re-evaluated **only when the pull
    /// actually moved the branch tree** — a pull that finds no upstream change
    /// (the common case for a periodic background sync) leaves every query
    /// result identical, so re-polling would be pure waste. Skipping it is the
    /// difference between an idle sync being invisible and it re-running every
    /// subscription on the branch on every tick.
    ///
    /// Two phases: the network-bound fetch + rebase
    /// ([`prepare`](dialog_repository::PreparedPull)) runs lock-free, then the
    /// instant cell advance ([`commit`](dialog_repository::PreparedPull::commit))
    /// runs under the per-branch transactor lock — the same lock commits take.
    /// So a sync's advance and a commit serialize cleanly on the one contended
    /// step, while the expensive merge stays concurrent. The transactor lock is
    /// held only for the microsecond cell writes, so a transaction never waits
    /// on a sync's network round trip.
    pub async fn perform<Env>(self, env: &Env) -> Result<Option<Revision>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + PullProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;

        // The tree we're at before the pull. Compared against the post-commit
        // revision to decide whether anything actually changed.
        let before = cached.handle().revision().map(|r| r.tree);

        // Fetch + rebase + persist blocks — no cell writes, no lock.
        let prepared = cached.handle().pull().prepare(env).await?;

        // Advance the cells under the transactor lock, so this serializes with
        // commits on the same branch instead of racing the head CAS.
        let revision = {
            let _advancing = cached.state.transactor().lock().await;
            prepared.commit(env).await?
        };

        // Re-poll subscriptions only if the tree moved. A `None` revision (a
        // no-op pull) or a merge that netted the same tree changes no query
        // result, so the poll is skipped — an idle background sync does no
        // subscription work at all.
        let changed = match &revision {
            Some(after) => before.as_ref() != Some(&after.tree),
            None => false,
        };
        if changed {
            cached.poll(env).await;
        }

        if self.download {
            cached.handle().download().perform(env).await?;
        }

        Ok(revision)
    }
}
