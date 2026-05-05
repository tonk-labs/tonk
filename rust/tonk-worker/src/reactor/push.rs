//! [`Push`] — wrap [`dialog_repository::Branch::push`].
//!
//! No subscription poll on success: push doesn't change local
//! branch state, so any subscription's query result is the
//! same after the push as before.

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, PushProvider};
use super::error::ReactorError;

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
        cached.handle().push().perform(env).await?;
        Ok(())
    }
}
