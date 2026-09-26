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
use serde::{Deserialize, Serialize};

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

/// One branch's session overlay, addressed by where the reactor caches it, so
/// another process (a successor service worker) can restore it into the same
/// branch. The overlay otherwise lives only in this process's memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlaySnapshot {
    /// The named repository, or `None` for the profile-as-repository.
    pub repository: Option<String>,
    /// The branch within that repository.
    pub branch: String,
    /// The branch's session facts, asserts and retracts alike.
    pub changes: Changes,
}

impl crate::Reactor {
    /// Snapshot the session overlay of every cached branch that has one.
    /// Only cached branches can hold overlay facts, since the overlay lives
    /// on the cached branch handle.
    pub fn export_overlays(&self) -> Vec<OverlaySnapshot> {
        let named: Vec<(Option<String>, Arc<crate::RepositoryState>)> = self
            .repos()
            .read()
            .iter()
            .map(|(name, repository)| (Some(name.clone()), Arc::clone(repository)))
            .collect();
        let profile = self
            .profile_repo_state()
            .map(|repository| (None, repository));
        named
            .into_iter()
            .chain(profile)
            .flat_map(|(repository, state)| {
                let branches = state.branches().read();
                branches
                    .iter()
                    .map(|(branch, cached)| OverlaySnapshot {
                        repository: repository.clone(),
                        branch: branch.clone(),
                        changes: cached.branch.overlay().export(),
                    })
                    .filter(|snapshot| !snapshot.changes.is_empty())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Restore overlays [exported](Self::export_overlays) by another process,
    /// opening each branch as needed and scheduling a poll so its subscribers
    /// see the restored facts. A branch that cannot be opened here is
    /// skipped. Returns how many overlays were restored.
    pub async fn import_overlays<Env>(&self, snapshots: Vec<OverlaySnapshot>, env: &Env) -> usize
    where
        Env: LoadProvider + BranchOpenProvider,
    {
        let mut restored = 0;
        for snapshot in snapshots {
            let repository = match &snapshot.repository {
                Some(name) => self.repository(name),
                None => self.profile_repository(),
            };
            match repository.branch(&snapshot.branch).acquire(env).await {
                Ok(session) => {
                    session.state.assert_overlay(snapshot.changes);
                    self.schedule_poll(Arc::clone(&session.state));
                    restored += 1;
                }
                Err(error) => {
                    dialog_common::log!(
                        "overlay restore skipped {:?}/{}: {error}",
                        snapshot.repository,
                        snapshot.branch
                    );
                }
            }
        }
        restored
    }
}
