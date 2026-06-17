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
use tonk_evaluator::effects::TransactionExt;
use tonk_schema::claim::{Claim, PredicateApplication};
use tonk_schema::transact::application_plan_from_predicate;

use super::BranchReference;
use super::env::{BranchOpenProvider, CommitProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// Builder — accumulates assertions and retractions into a
/// [`Changes`] batch. Chain off `TonkBranch::transaction`.
///
/// Claims are bucketed by durability: durable claims go into
/// [`Self::changes`] and flow through to the dialog commit
/// untouched; transient claims go into [`Self::transients`] so
/// [`Commit::perform`] can retract them inside the same
/// transaction before the durable write (the
/// assert+retract-cancels-at-commit pattern from
/// `plan/effects.md`). Routes that don't yet carry transient
/// classification (e.g. the legacy `/claim/*` path) keep using
/// the raw [`Self::assert`] / [`Self::retract`] entries, which
/// default to the durable bucket.
pub struct TransactionBuilder<'a> {
    /// The branch the transaction will commit to.
    pub branch: BranchReference<'a>,
    /// Accumulated durable claims.
    pub changes: Changes,
    /// Accumulated transient claims — facts asserted at the
    /// current timestep that must not survive into durable
    /// storage. Seeded by [`Self::apply`] for transient-typed
    /// [`Claim`]s; consumed by [`Commit::perform`] which
    /// integrates them into the transaction overlay (so effects
    /// see them) then emits a matching retract before commit.
    pub transients: Changes,
}

impl<'a> TransactionBuilder<'a> {
    /// Build a new transaction with no claims accumulated yet.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            changes: Changes::new(),
            transients: Changes::new(),
        }
    }

    /// Add an assertion to the durable bucket. Used by callers
    /// that operate on raw dialog [`Statement`] values (legacy
    /// `/claim/*` and tests) without a durability
    /// classification — assume durable.
    pub fn assert<S: Statement>(mut self, claim: S) -> Self {
        claim.assert(&mut self.changes);
        self
    }

    /// Add a retraction to the durable bucket. Same fallback
    /// semantics as [`Self::assert`].
    pub fn retract<S: Statement>(mut self, claim: S) -> Self {
        claim.retract(&mut self.changes);
        self
    }

    /// Apply a typed [`Claim`] from the wire format, routing into
    /// the durable or transient bucket based on the predicate's
    /// classification. Each application is planned into raw EAV
    /// facts (same emitter the asserted-notation planner uses) and
    /// added to the appropriate batch.
    pub fn apply(self, claim: Claim) -> Self {
        match claim {
            Claim::Assert(application) => self.apply_assert(application),
            Claim::Retract(application) => self.apply_retract(application),
        }
    }

    /// Assert a [`PredicateApplication`]. Transient predicates
    /// land in the transient bucket; durable predicates land in
    /// the durable bucket.
    pub fn apply_assert(mut self, application: PredicateApplication) -> Self {
        let bucket = if application.is_transient() {
            &mut self.transients
        } else {
            &mut self.changes
        };
        application_plan_from_predicate(application).assert(bucket);
        self
    }

    /// Retract a [`PredicateApplication`]. Routed by the same
    /// rule as [`Self::apply_assert`] — a retraction of a
    /// transient predicate goes to the transient bucket so it
    /// pairs with whatever assertion the same predicate emitted
    /// earlier in the document.
    pub fn apply_retract(mut self, application: PredicateApplication) -> Self {
        let bucket = if application.is_transient() {
            &mut self.transients
        } else {
            &mut self.changes
        };
        application_plan_from_predicate(application).retract(bucket);
        self
    }

    /// Wrap the accumulated changes into a [`Commit`] effect.
    pub fn commit(self) -> Commit<'a> {
        Commit {
            branch: self.branch,
            changes: self.changes,
            transients: self.transients,
        }
    }
}

/// Commit effect — performs the dialog commit, then re-polls
/// every subscription on the branch so changed query results
/// fan out to subscribers.
pub struct Commit<'a> {
    /// The branch the commit applies to.
    pub branch: BranchReference<'a>,
    /// Durable claims — integrated into the transaction and
    /// kept through to the dialog commit.
    pub changes: Changes,
    /// Transient claims — integrated into the transaction so
    /// effects' deductive saturation can see them, then matched
    /// by inline retractions before the durable write so they
    /// cancel at commit. See `plan/effects.md`.
    pub transients: Changes,
}

impl Commit<'_> {
    /// Execute the commit. On success, every subscription on
    /// the branch is re-evaluated and (if its result changed)
    /// broadcasts to its subscribers.
    ///
    /// The user's durable and transient [`Changes`] are loaded
    /// into a dialog
    /// [`Transaction`](dialog_repository::Transaction); the
    /// effects evaluator runs against it (firing inductive
    /// rules whose body matches the transaction's overlay
    /// view), then transient facts are retracted in-place so
    /// they cancel at commit. Whatever's left lands durably.
    pub async fn perform<Env>(self, env: &Env) -> Result<Revision, ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;
        let branch = cached.handle();

        let t_induce = web_time::Instant::now();
        let txn = branch
            .transaction()
            .integrate(self.changes)
            .integrate(self.transients.clone())
            .induce(self.transients)
            .perform(env)
            .await?;
        let induce_ms = t_induce.elapsed().as_millis();

        let t_commit = web_time::Instant::now();
        let revision = txn.commit().perform(env).await?;
        let commit_ms = t_commit.elapsed().as_millis();

        let t_poll = web_time::Instant::now();
        cached.poll(env).await;
        let poll_ms = t_poll.elapsed().as_millis();

        dialog_common::log!(
            "reactor commit timing: induce {induce_ms}ms | commit {commit_ms}ms | poll {poll_ms}ms"
        );
        Ok(revision)
    }
}
