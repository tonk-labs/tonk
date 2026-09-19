//! [`OverlayBuilder`] and [`OverlayWrite`] — accumulate assert/retract
//! pairs and apply them to a branch's **state layer**.
//!
//! The state-layer counterpart to [`TransactionBuilder`](crate::TransactionBuilder):
//! same `.assert(…)` / `.retract(…)` / `.perform(&env)` shape, but the changes
//! land in the process's ephemeral state layer above the branch (never
//! committed, never replicated) rather than the durable branch tree. Like a
//! commit, a successful write **schedules** a poll of the branch
//! ([`Reactor::schedule_poll`]) so subscribers are notified of the change —
//! callers don't hand-drive the poll. The request's dispatcher drains the
//! scheduled set once per turn ([`Reactor::run_scheduled_polls`]), so a state
//! write and a commit on the same branch coalesce into a single re-evaluation.
//!
//! Use this for per-request state (the tab's `tonk:site`, the sync status)
//! instead of [`BranchState::write`](crate::BranchState::write) + a manual
//! `schedule_poll`/`run_scheduled_polls` pair. A write that must *replace* an
//! entity's facts rather than merge into them chains
//! [`forget`](OverlayBuilder::forget) first: the forget and the asserts land
//! in one commit, so no reader sees the layer between them.
//!
//! [`Reactor::schedule_poll`]: crate::Reactor::schedule_poll
//! [`Reactor::run_scheduled_polls`]: crate::Reactor::run_scheduled_polls

use std::sync::Arc;

use dialog_artifacts::{Changes, Entity, Statement};

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider};
use super::error::ReactorError;

/// Builder — accumulates state-layer assertions and retractions into a
/// [`Changes`] batch. Chain off
/// [`BranchReference::overlay`](crate::BranchReference::overlay).
/// Lazy: nothing touches the branch until [`OverlayWrite::perform`].
pub struct OverlayBuilder<'a> {
    /// The branch whose state layer the write targets.
    pub branch: BranchReference<'a>,
    /// Accumulated changes (asserts and retracts).
    pub changes: Changes,
    /// Entities whose state-layer facts the write drops first.
    pub forgotten: Vec<Entity>,
}

impl<'a> OverlayBuilder<'a> {
    /// Begin an empty write.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            changes: Changes::new(),
            forgotten: Vec::new(),
        }
    }

    /// Add an assertion to the batch.
    pub fn assert<S: Statement>(mut self, claim: S) -> Self {
        claim.assert(&mut self.changes);
        self
    }

    /// Add a retraction to the batch.
    pub fn retract<S: Statement>(mut self, claim: S) -> Self {
        claim.retract(&mut self.changes);
        self
    }

    /// Drop every state-layer fact of `entity` before the batch lands,
    /// in the same commit.
    pub fn forget(mut self, entity: Entity) -> Self {
        self.forgotten.push(entity);
        self
    }

    /// Finish the builder; chain `.perform(&env)` to apply.
    pub fn write(self) -> OverlayWrite<'a> {
        OverlayWrite {
            branch: self.branch,
            changes: self.changes,
            forgotten: self.forgotten,
        }
    }
}

/// The applicable write — `.perform(&env)` commits the accumulated
/// changes into the branch's state layer and schedules a poll.
pub struct OverlayWrite<'a> {
    branch: BranchReference<'a>,
    changes: Changes,
    forgotten: Vec<Entity>,
}

impl OverlayWrite<'_> {
    /// Apply the forgets and the accumulated changes to the branch's state
    /// layer and schedule a poll so subscribers are notified. The poll is
    /// scheduled (not run inline) — the request dispatcher drains it once
    /// per turn.
    pub async fn perform<Env>(self, env: &Env) -> Result<(), ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider,
    {
        let cached = self.branch.acquire(env).await?;
        // `Changes` keeps both asserts and retracts in the batch, so this
        // applies the whole write (the new site facts and any retract of a
        // prior one) in one stack commit.
        cached
            .state
            .apply(self.forgotten, self.changes, env)
            .await?;
        self.branch
            .reactor()
            .schedule_poll(Arc::clone(&cached.state));
        Ok(())
    }
}
