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
    pub async fn perform<Env>(self, env: &Env) -> Result<Option<Revision>, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + PullProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;
        let revision = cached.handle().pull().perform(env).await?;
        cached.poll(env).await;
        Ok(revision)
    }
}
