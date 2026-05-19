//! [`TransactionBuilder`] and [`Commit`] — accumulate
//! assert/retract pairs and commit them through the reactor so
//! subscriptions re-poll on success.
//!
//! Routes call this instead of `Branch::transaction()` so they
//! can't forget to notify subscribers — the wrapper does it
//! automatically. The builder is lazy; nothing touches the
//! branch until `Commit::perform`.
//!
//! [`Commit::perform`] runs the effects evaluator
//! ([`super::effects::evaluate_effects`]) between the user's
//! changes and the durable write, then strips transient facts
//! ([`super::effects::retract_transients`]) before commit so they
//! never reach durable storage.

use dialog_artifacts::{Changes, Statement};
use dialog_repository::Revision;

use super::BranchReference;
use super::effects::{evaluate_effects, retract_transients};
use super::env::{BranchOpenProvider, CommitProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// Builder — accumulates assertions and retractions into a
/// [`Changes`] batch. Chain off `TonkBranch::transaction`.
pub struct TransactionBuilder<'a> {
    /// The branch the transaction will commit to.
    pub branch: BranchReference<'a>,
    /// Accumulated assert/retract claims.
    pub changes: Changes,
}

impl<'a> TransactionBuilder<'a> {
    /// Build a new transaction with no claims accumulated yet.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            changes: Changes::new(),
        }
    }

    /// Add an assertion.
    pub fn assert<S: Statement>(mut self, claim: S) -> Self {
        self.changes.assert(claim);
        self
    }

    /// Add a retraction.
    pub fn retract<S: Statement>(mut self, claim: S) -> Self {
        self.changes.retract(claim);
        self
    }

    /// Wrap the accumulated changes into a [`Commit`] effect.
    pub fn commit(self) -> Commit<'a> {
        Commit {
            branch: self.branch,
            changes: self.changes,
        }
    }
}

/// Commit effect — performs the dialog commit, then re-polls
/// every subscription on the branch so changed query results
/// fan out to subscribers.
pub struct Commit<'a> {
    /// The branch the commit applies to.
    pub branch: BranchReference<'a>,
    /// The accumulated claim changes to commit atomically.
    pub changes: Changes,
}

impl Commit<'_> {
    /// Execute the commit. On success, every subscription on
    /// the branch is re-evaluated and (if its result changed)
    /// broadcasts to its subscribers.
    ///
    /// The user's [`Changes`] are loaded into a dialog
    /// [`Transaction`](dialog_repository::Transaction); the
    /// effects evaluator runs against it (firing inductive rules
    /// whose body matches the transaction's overlay view), then
    /// transient facts are retracted in-place so they cancel at
    /// commit. Whatever's left lands durably.
    pub async fn perform<Env>(self, env: &Env) -> Result<Revision, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;
        let branch = cached.handle();

        let mut txn = branch.transaction().integrate(self.changes);
        txn = evaluate_effects(branch, txn, env).await?;
        txn = retract_transients(branch, txn, env).await?;

        let revision = txn.commit().perform(env).await?;
        cached.poll(env).await;
        Ok(revision)
    }
}
