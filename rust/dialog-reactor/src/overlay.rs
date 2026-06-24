//! [`OverlayBuilder`] and [`OverlayWrite`] — accumulate assert/retract
//! pairs and apply them to a branch's **session overlay**.
//!
//! The overlay counterpart to [`TransactionBuilder`](crate::TransactionBuilder):
//! same `.assert(…)` / `.retract(…)` / `.perform(&env)` shape, but the changes
//! land in the in-memory session overlay (ephemeral, never committed, never
//! replicated) rather than the durable branch tree. Like a commit, a successful
//! overlay write **schedules** a poll of the branch ([`Reactor::schedule_poll`])
//! so subscribers are notified of the change — callers don't hand-drive the
//! poll. The request's dispatcher drains the scheduled set once per turn
//! ([`Reactor::run_scheduled_polls`]), so an overlay write and a commit on the
//! same branch coalesce into a single re-evaluation.
//!
//! Use this for per-request overlay state (the tab's `tonk:site`, the sync
//! status) instead of `BranchState::assert_overlay` + a manual
//! `schedule_poll`/`run_scheduled_polls` pair.
//!
//! [`Reactor::schedule_poll`]: crate::Reactor::schedule_poll
//! [`Reactor::run_scheduled_polls`]: crate::Reactor::run_scheduled_polls

use std::sync::Arc;

use dialog_artifacts::{Changes, Statement};

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider};
use super::error::ReactorError;

/// Builder — accumulates overlay assertions and retractions into a [`Changes`]
/// batch. Chain off [`BranchReference::overlay`](crate::BranchReference::overlay).
/// Lazy: nothing touches the branch until [`OverlayWrite::perform`].
pub struct OverlayBuilder<'a> {
    /// The branch whose overlay the write targets.
    pub branch: BranchReference<'a>,
    /// Accumulated overlay changes (asserts and retracts).
    pub changes: Changes,
}

impl<'a> OverlayBuilder<'a> {
    /// Begin an empty overlay write.
    pub fn new(branch: BranchReference<'a>) -> Self {
        Self {
            branch,
            changes: Changes::new(),
        }
    }

    /// Add an assertion to the overlay batch.
    pub fn assert<S: Statement>(mut self, claim: S) -> Self {
        claim.assert(&mut self.changes);
        self
    }

    /// Add a retraction to the overlay batch.
    pub fn retract<S: Statement>(mut self, claim: S) -> Self {
        claim.retract(&mut self.changes);
        self
    }

    /// Finish the builder; chain `.perform(&env)` to apply.
    pub fn write(self) -> OverlayWrite<'a> {
        OverlayWrite {
            branch: self.branch,
            changes: self.changes,
        }
    }
}

/// The applicable overlay write — `.perform(&env)` writes the accumulated
/// changes into the branch's session overlay and schedules a poll.
pub struct OverlayWrite<'a> {
    branch: BranchReference<'a>,
    changes: Changes,
}

impl OverlayWrite<'_> {
    /// Apply the accumulated changes to the branch's session overlay and
    /// schedule a poll so subscribers are notified. The poll is scheduled (not
    /// run inline) — the request dispatcher drains it once per turn.
    pub async fn perform<Env>(self, env: &Env) -> Result<(), ReactorError>
    where
        Env: LoadProvider + BranchOpenProvider,
    {
        let cached = self.branch.acquire(env).await?;
        // `Changes::assert` preserves both asserts and retracts in the batch, so
        // this applies the whole overlay write (the new site facts and any
        // retract of a prior one) in one exclusive-lock write.
        cached.state.assert_overlay(self.changes);
        self.branch
            .reactor()
            .schedule_poll(Arc::clone(&cached.state));
        Ok(())
    }
}
