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
}

impl<'a> Pull<'a> {
    /// Build a new `Pull` effect.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self { branch }
    }

    /// Execute the pull. On success, every subscription on the
    /// branch is re-evaluated.
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

        // Fetch + rebase + persist blocks — no cell writes, no lock.
        let prepared = cached.handle().pull().prepare(env).await?;

        // Advance the cells under the transactor lock, so this serializes with
        // commits on the same branch instead of racing the head CAS.
        let revision = {
            let _advancing = cached.state.transactor().lock().await;
            prepared.commit(env).await?
        };

        cached.poll(env).await;
        Ok(revision)
    }
}
