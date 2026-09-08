//! Worker policy receipts, independent of the reactor's handle cache.
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use dialog_reactor::{BranchState, RepositoryState};
use dialog_repository::{Revision, Upstream};
use parking_lot::Mutex;

use crate::worker::TonkState;

#[derive(Default)]
pub(crate) struct AdmissionCache {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    #[cfg(test)]
    pub(super) observations: Observations,
}

impl AdmissionCache {
    pub(super) fn entry(&self, key: &str) -> Arc<Entry> {
        let normalized = super::space_subject(key)
            .map(|did| did.to_string())
            .unwrap_or_else(|| key.to_owned());
        self.entries.lock().entry(normalized).or_default().clone()
    }

    /// A writer never takes the admission mutex: reconciliation itself writes.
    pub(crate) fn mutation(&self, key: &str) -> Mutation {
        let entry = self.entry(key);
        {
            let mut state = entry.state.lock();
            state.generation += 1;
            state.writers += 1;
            state.receipt = None;
        }
        Mutation(entry)
    }
}

pub(crate) struct Mutation(Arc<Entry>);

impl Drop for Mutation {
    fn drop(&mut self) {
        let mut state = self.0.state.lock();
        state.generation += 1;
        state.writers -= 1;
        state.receipt = None;
    }
}

#[derive(Default)]
pub(super) struct Entry {
    state: Mutex<State>,
    pub(super) slow: tokio::sync::Mutex<()>,
    #[cfg(test)]
    pub(super) gate: TestGate,
    #[cfg(test)]
    pub(super) membership_gate: TestGate,
}

#[cfg(test)]
type TestGate = Mutex<
    Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
>;

#[derive(Default)]
struct State {
    generation: u64,
    writers: usize,
    receipt: Option<Receipt>,
}

/// Weak identity, not ownership: removing a repo must release all its handles.
#[derive(Clone)]
pub(super) struct Stamp {
    start: Start,
    repository: Weak<RepositoryState>,
    meta: Weak<BranchState>,
    meta_revision: Option<Revision>,
}

/// Profile membership is read before opening a mounted repository. Capture
/// its freshness first, then extend the stamp with validated mount handles.
#[derive(Clone)]
pub(super) struct Start {
    generation: u64,
    profile: Weak<BranchState>,
    revision: Option<Revision>,
}

impl Start {
    fn same(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.profile.ptr_eq(&other.profile)
            && self.revision == other.revision
    }
}

struct Receipt {
    stamp: Stamp,
    upstreams: Vec<(String, String, String)>,
}

impl Stamp {
    pub(super) fn started_at(&self, start: Option<&Start>) -> bool {
        start.is_some_and(|start| self.start.same(start))
    }

    fn same(&self, other: &Self) -> bool {
        self.start.same(&other.start)
            && self.repository.ptr_eq(&other.repository)
            && self.meta.ptr_eq(&other.meta)
            && self.meta_revision == other.meta_revision
    }
}

impl Entry {
    #[cfg(test)]
    pub(super) async fn pause_if_requested(&self) {
        let gate = self.gate.lock().take();
        if let Some((entered, release)) = gate {
            let _ = entered.send(());
            let _ = release.await;
        }
    }

    #[cfg(test)]
    pub(super) async fn pause_membership_if_requested(&self) {
        let gate = self.membership_gate.lock().take();
        if let Some((entered, release)) = gate {
            let _ = entered.send(());
            let _ = release.await;
        }
    }

    pub(super) fn forget(&self) {
        self.state.lock().receipt = None;
    }

    pub(super) fn start(&self, tonk: &TonkState) -> Option<Start> {
        let state = self.state.lock();
        if state.writers != 0 {
            return None;
        }
        self.start_at(tonk, state.generation)
    }

    fn start_at(&self, tonk: &TonkState, generation: u64) -> Option<Start> {
        let profile = tonk.reactor.profile_repo_state()?;
        let main = profile
            .branches()
            .read()
            .get(tonk_account::MAIN_BRANCH)?
            .clone();
        Some(Start {
            generation,
            profile: Arc::downgrade(&main),
            revision: main.branch.revision(),
        })
    }

    /// Pure cache lookups; no acquisition, storage, overlay epoch or content head.
    fn stamp_at(&self, tonk: &TonkState, key: &str, generation: u64) -> Option<Stamp> {
        let start = self.start_at(tonk, generation)?;
        let repository = tonk.reactor.repos().read().get(key)?.clone();
        let meta = repository
            .branches()
            .read()
            .get(super::super::repository::META_BRANCH)?
            .clone();
        Some(Stamp {
            start,
            repository: Arc::downgrade(&repository),
            meta: Arc::downgrade(&meta),
            meta_revision: meta.branch.revision(),
        })
    }

    pub(super) fn stamp(&self, tonk: &TonkState, key: &str) -> Option<Stamp> {
        let state = self.state.lock();
        if state.writers != 0 {
            return None;
        }
        self.stamp_at(tonk, key, state.generation)
    }

    pub(super) fn valid(&self, tonk: &TonkState, key: &str) -> bool {
        let state = self.state.lock();
        if state.writers != 0 {
            return false;
        }
        let Some(receipt) = &state.receipt else {
            return false;
        };
        let Some(current) = self.stamp_at(tonk, key, state.generation) else {
            return false;
        };
        if !receipt.stamp.same(&current) {
            return false;
        }
        let Some(repo) = current.repository.upgrade() else {
            return false;
        };
        let branches = repo.branches().read();
        receipt.upstreams.iter().all(|(name, remote, target)| {
            branches.get(name).is_some_and(|branch| {
                matches!(branch.branch.upstream(),
                    Some(Upstream::Remote { remote: current_remote, branch: current_branch, .. })
                        if current_remote == *remote && current_branch == *target
                )
            })
        })
    }

    pub(super) fn install(
        &self,
        tonk: &TonkState,
        key: &str,
        before: Option<Stamp>,
        upstreams: Vec<(String, String, String)>,
    ) {
        let Some(before) = before else {
            return;
        };
        let mut state = self.state.lock();
        if state.writers != 0 {
            return;
        }
        let Some(after) = self.stamp_at(tonk, key, state.generation) else {
            return;
        };
        if before.same(&after) {
            state.receipt = Some(Receipt {
                stamp: after,
                upstreams,
            });
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct Observations {
    pub slow: std::sync::atomic::AtomicUsize,
    pub directory: std::sync::atomic::AtomicUsize,
    pub configuration: std::sync::atomic::AtomicUsize,
    pub fail_directory: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl Observations {
    pub fn counts(&self) -> (usize, usize, usize) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.slow.load(Relaxed),
            self.directory.load(Relaxed),
            self.configuration.load(Relaxed),
        )
    }
}
