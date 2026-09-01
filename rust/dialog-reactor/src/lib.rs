//! [`Reactor`] — a reactive layer over dialog branches.
//!
//! See `reactor-spec.md` next to the crate's `Cargo.toml` for the
//! full design rationale. The short version: consumers mutate
//! branches through a chain (`reactor.repository(r).branch(b)
//! .transaction().assert(…).commit().perform(&op).await?`); the
//! leaf effect's `perform` re-evaluates every subscription on the
//! branch and broadcasts the result to whoever is listening,
//! deduplicating by hash so unchanged results don't fire spurious
//! broadcasts.
//!
//! The reactor caches `Repository` and `Branch` handles — first
//! reference opens, subsequent references reuse — so per-request
//! load+open overhead is paid once per repo/branch lifetime.
//! Subscriptions live on the [`BranchEntry`] itself, so once a
//! branch is acquired its subscription operations skip the
//! reactor's name-keyed lookup entirely.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_operator::Profile;
use parking_lot::{Mutex, RwLock};

mod branch;
mod command;
mod env;
mod error;
mod export;
mod formula;
mod import;
mod overlay;
mod pull;
mod push;
mod query;
mod repository;
mod subscribe;
mod subscription;
mod transaction;

pub use branch::{BranchReference, BranchSession, BranchState};
pub use command::{CommandHandler, CommandRegistry, Decode, EntityFacts, RunFuture, TypedCommand};
pub use env::{
    BranchOpenProvider, CommitProvider, GetPutProvider, LoadProvider, PullProvider, PushProvider,
    SelectProvider,
};
pub use error::ReactorError;
// `PendingSubscription` is declared in this module; re-exported here for
// symmetry with the other public reactor types.
pub use export::{Export, ExportError};
pub use formula::{FormulaError, resolve_formula};
pub use import::{Import, ImportError};
pub use overlay::{OverlayBuilder, OverlayWrite};
pub use pull::Pull;
pub use push::Push;
pub use query::QueryEffect;
pub use repository::{RepositoryReference, RepositoryState};
pub use subscribe::Subscribe;
pub use subscription::{QueryHash, Subscriber, SubscriptionPoll, SubscriptionReference};
/// On-the-wire `Conclusion` and `Query` — re-exported from
/// [`tonk_schema`] so consumers (browser clients, the consumer
/// elements) can deserialize without depending on this crate.
pub use tonk_schema::conclusion::{Conclusion, Frame, project};
pub use tonk_schema::query::Query;
pub use transaction::{Commit, TransactionBuilder};

/// A reactive layer over dialog branches. Owned by the consumer's
/// application state (e.g. the worker's `TonkState`).
pub struct Reactor {
    profile: Profile,
    repos: RwLock<HashMap<String, Arc<RepositoryState>>>,
    /// Cached `RepositoryState` for the profile-as-repository.
    /// Lazily populated on first `profile_repository().acquire()`
    /// call; lives outside `repos` because the profile is a
    /// singleton with no name in the routing namespace.
    profile_repo: RwLock<Option<Arc<RepositoryState>>>,
    /// Branches whose subscriptions need re-evaluation but haven't
    /// been polled yet. A mutation that changes query results —
    /// a durable [`Commit`] or a session-overlay write — schedules
    /// the affected branch here (sync, no env) instead of polling
    /// inline; the request's command dispatcher drains this with
    /// [`run_scheduled_polls`](Self::run_scheduled_polls) once after
    /// its providers run, so the env lives at one drain point and
    /// several writes on a branch in one turn coalesce into a single
    /// poll. Pointer identity (`Arc::ptr_eq`) dedups, so the same
    /// branch scheduled twice polls once.
    pending_polls: Mutex<Vec<Arc<BranchState>>>,
    /// Subscriptions registered against a branch that does not exist
    /// yet, keyed by `(repository, branch)`.
    ///
    /// A repo this device has not replicated, or a branch not created
    /// yet, is an ABSENCE — the empty set — not an error. The subscriber
    /// is registered here and answered with an empty frame, and when
    /// [`BranchReference::acquire`] later materializes that exact
    /// `(repo, branch)` it adopts every pending subscription onto the
    /// real [`BranchState`] and polls once, so the standing query
    /// delivers its first real frame.
    ///
    /// Nothing polls or retries: registration is passive, and the
    /// hand-off is driven by the branch coming into existence. Both
    /// absences behave identically — a space joined in another tab and a
    /// branch created later both arrive through `acquire`.
    pending_subscriptions: Mutex<HashMap<(String, String), Vec<PendingSubscription>>>,
}

/// A subscription waiting for its branch to exist. Holds everything
/// needed to attach it to the real [`BranchState`] once one appears.
pub struct PendingSubscription {
    /// The query to install when the branch materializes.
    pub query: dialog_query::ConceptQuery,
    /// The client this subscriber serves, for stale-client pruning.
    pub client: Option<String>,
    /// Sender the adopted subscription broadcasts into — already wired
    /// to the consumer's open SSE stream, which is why the hand-off is
    /// invisible to the page: it just starts receiving frames.
    pub sender: tokio::sync::mpsc::UnboundedSender<bytes::Bytes>,
}

impl Reactor {
    /// Construct a reactor over the given profile. The reactor
    /// doesn't own an operator — every effect takes one at
    /// `perform` time, matching dialog's command/perform pattern.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            repos: RwLock::new(HashMap::new()),
            profile_repo: RwLock::new(None),
            pending_polls: Mutex::new(Vec::new()),
            pending_subscriptions: Mutex::new(HashMap::new()),
        }
    }

    /// Register a subscription against a branch that does not exist yet.
    ///
    /// Passive: nothing polls, nothing retries. The entry sits until
    /// [`Self::adopt_pending`] is called with a matching `(repo, branch)`
    /// — which [`BranchReference::acquire`] does the moment that branch
    /// materializes, whether because a space was joined elsewhere or a
    /// branch was created.
    pub fn register_pending(&self, repo: &str, branch: &str, pending: PendingSubscription) {
        self.pending_subscriptions
            .lock()
            .entry((repo.to_owned(), branch.to_owned()))
            .or_default()
            .push(pending);
    }

    /// Take every subscription registered against `(repo, branch)`.
    ///
    /// Called from [`BranchReference::acquire`] once the branch exists.
    /// Draining (rather than copying) means each pending subscription is
    /// adopted exactly once; a consumer that has since gone away is
    /// pruned by the ordinary dead-subscriber sweep after adoption.
    pub fn take_pending(&self, repo: &str, branch: &str) -> Vec<PendingSubscription> {
        self.pending_subscriptions
            .lock()
            .remove(&(repo.to_owned(), branch.to_owned()))
            .unwrap_or_default()
    }

    /// Whether any subscription is waiting on `(repo, branch)`. Lets
    /// `acquire` skip the drain entirely in the overwhelmingly common
    /// case where nothing was waiting.
    pub fn has_pending(&self, repo: &str, branch: &str) -> bool {
        self.pending_subscriptions
            .lock()
            .contains_key(&(repo.to_owned(), branch.to_owned()))
    }

    /// Schedule a poll of `state`'s subscriptions. Called by mutating
    /// effects (a durable commit, a session-overlay write) so the change
    /// propagates without the writer holding an env or polling inline.
    /// The scheduled branch is re-evaluated when the request's dispatcher
    /// calls [`run_scheduled_polls`](Self::run_scheduled_polls). Cheap and
    /// sync — just an `Arc` push; dedup happens at drain time.
    pub fn schedule_poll(&self, state: Arc<BranchState>) {
        self.pending_polls.lock().push(state);
    }

    /// Drain the scheduled-poll set: re-evaluate every distinct branch's
    /// subscriptions once, broadcasting changed results to subscribers.
    /// Called by the command dispatcher after its providers run, so all
    /// the polls a turn scheduled fire together with the env in hand.
    /// Pointer identity dedups, so a branch scheduled by both its commit
    /// and an overlay write polls a single time.
    pub async fn run_scheduled_polls<'a, Env: SelectProvider>(&'a self, env: &'a Env) {
        let scheduled = {
            let mut pending = self.pending_polls.lock();
            std::mem::take(&mut *pending)
        };
        let mut unique: Vec<Arc<BranchState>> = Vec::new();
        for state in scheduled {
            if !unique.iter().any(|seen| Arc::ptr_eq(seen, &state)) {
                unique.push(state);
            }
        }
        for state in unique {
            state.poll(env).await;
        }
    }

    /// Drop every cached handle and every active SSE subscriber
    /// so open response bodies finish. Called from the SW upgrade
    /// path so the old worker can be replaced.
    ///
    /// Walking the cache and explicitly dropping subscriber
    /// senders is the load-bearing step: the
    /// [`BranchState`](crate::BranchState) `Arc`s are
    /// shared with `SubscriptionPoll` futures still holding a
    /// reference, so removing the cache entry alone isn't enough.
    /// Clearing each branch's subscriber map drops every
    /// `mpsc::Sender`, which surfaces `None` on the receiver side
    /// and ends the SSE response stream regardless of who else
    /// holds the state.
    pub fn shutdown(&self) {
        let repos = {
            let mut map = self.repos.write();
            std::mem::take(&mut *map)
        };
        // The profile-as-repository lives in its own slot, not in
        // `repos`. The Hub subscribes to its main branch
        // (`/api/profile/branch/main/query` SSE), so it must be drained
        // too — otherwise that one stream stays open and pins the
        // outgoing worker in `waiting` on every update.
        let profile = self.profile_repo.write().take();
        for repo in repos.into_values().chain(profile) {
            let branches = {
                let mut map = repo.branches().write();
                std::mem::take(&mut *map)
            };
            for (_, branch) in branches {
                branch.clear_subscribers();
            }
        }
    }

    /// Drop one repository's cached handles and active subscribers —
    /// the per-repo analog of [`shutdown`](Self::shutdown). Used when a
    /// space is removed: the background sync sweep builds its repo set
    /// from this cache, so eviction is what actually stops the space
    /// from syncing, and clearing each branch's subscriber map ends its
    /// SSE streams (see `shutdown` for why removing the cache entry
    /// alone isn't enough). No-op when the repo isn't cached.
    pub fn evict(&self, name: &str) {
        let Some(repo) = self.repos.write().remove(name) else {
            return;
        };
        let branches = {
            let mut map = repo.branches().write();
            std::mem::take(&mut *map)
        };
        for (_, branch) in branches {
            branch.clear_subscribers();
        }
    }

    /// Reconcile the cached handle for `repo`/`branch` with durable
    /// storage after its upstream/remote wiring changed on a *separate*
    /// repository handle.
    ///
    /// A [`Branch`](dialog_repository::Branch) captures its `upstream`
    /// cell when first opened — e.g. during the standard-library seed,
    /// before any remote is attached. A later `set_upstream` on a
    /// freshly loaded handle publishes to durable storage but never
    /// touches that cached cell, so sync — which reads through this
    /// cache — sees no upstream and fails with `BranchHasNoUpstream`.
    /// Re-opening re-resolves the upstream from durable storage, so the
    /// fresh handle reflects the change; we swap it into the cache and
    /// rebind the existing subscriptions to it so live SSE streams keep
    /// updating. Rebinding sends a fresh snapshot on the next poll because
    /// the discarded engine's retained result cannot be used as a delta base
    /// for the new branch handle.
    ///
    /// No-op when the repo or branch isn't cached: the next `acquire`
    /// opens a fresh handle that already reflects durable state.
    pub async fn refresh_branch<Env>(
        &self,
        repo: &str,
        branch: &str,
        env: &Env,
    ) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        // Only a cached repository can hold a stale cached branch.
        let Some(repo_state) = self.repos.read().get(repo).map(Arc::clone) else {
            return Ok(());
        };
        // An uncached branch opens fresh on next acquire — already
        // current, nothing to reconcile.
        if !repo_state.branches().read().contains_key(branch) {
            return Ok(());
        }

        // Re-open outside the lock (open is async). This re-resolves the
        // branch's upstream cell from durable storage, so the fresh
        // handle reflects a `set_upstream` performed elsewhere.
        let handle = repo_state
            .repository()
            .branch(branch)
            .open()
            .perform(env)
            .await
            .map_err(|e| ReactorError::BranchNotFound {
                repo: repo.to_owned(),
                branch: branch.to_owned(),
                reason: e.to_string(),
            })?;
        let fresh = Arc::new(BranchState::new(handle));

        // Swap under the branches lock so a concurrent subscribe can't
        // land on the discarded state between the adopt and the insert.
        {
            let mut branches = repo_state.branches().write();
            if let Some(old) = branches.get(branch) {
                fresh.adopt_subscriptions_from(old);
            }
            branches.insert(branch.to_owned(), Arc::clone(&fresh));
        }
        // The adopted subscriptions need that fresh snapshot DELIVERED:
        // nothing else is going to poll on their behalf. The rebind used
        // to leave them waiting for whatever commit happened to come
        // next — and on a quiet branch none does, so a live view sat on
        // its loading state indefinitely after its space was wired to a
        // remote. The caller's next `run_scheduled_polls` drains this.
        self.schedule_poll(fresh);
        Ok(())
    }

    /// Snapshot every cached branch state, across named repositories
    /// and the profile-as-repository. Used by liveness sweeps that
    /// reconcile per-client state (overlay facts, tagged subscribers)
    /// against the set of live clients — only cached branches can
    /// hold any, since both live on the in-memory [`BranchState`].
    pub fn cached_branch_states(&self) -> Vec<Arc<BranchState>> {
        let repos: Vec<Arc<RepositoryState>> = {
            let map = self.repos.read();
            map.values().cloned().collect()
        };
        let profile = self.profile_repo.read().clone();
        repos
            .into_iter()
            .chain(profile)
            .flat_map(|repo| {
                let branches = repo.branches().read();
                branches.values().cloned().collect::<Vec<_>>()
            })
            .collect()
    }

    /// Begin a chain scoped to the named repository.
    pub fn repository<'a>(&'a self, name: &'a str) -> RepositoryReference<'a> {
        RepositoryReference::Named {
            reactor: self,
            name,
        }
    }

    /// Begin a chain scoped to the profile-as-repository. The
    /// profile lives outside the named-repo namespace; everything
    /// downstream (branch/transaction/sync) reuses the same chain
    /// surface as a named repository.
    pub fn profile_repository(&self) -> RepositoryReference<'_> {
        RepositoryReference::Profile { reactor: self }
    }

    /// Borrow the cache map. Public so the chain handles
    /// (`RepositoryReference::acquire`, `BranchReference::acquire`)
    /// can run their lookup-and-open logic directly without
    /// indirecting through helper methods.
    pub fn repos(&self) -> &RwLock<HashMap<String, Arc<RepositoryState>>> {
        &self.repos
    }

    /// Snapshot the cached profile-as-repository state, if any.
    /// Used by `RepositoryReference::Profile::acquire` for the
    /// fast-path branch.
    pub fn profile_repo_state(&self) -> Option<Arc<RepositoryState>> {
        self.profile_repo.read().clone()
    }

    /// Install the profile-as-repository state into the cache.
    /// Returns the resident value — if another caller raced and
    /// installed first, theirs wins (state is fungible).
    pub fn set_profile_repo_state(&self, state: Arc<RepositoryState>) -> Arc<RepositoryState> {
        let mut slot = self.profile_repo.write();
        if let Some(existing) = slot.clone() {
            existing
        } else {
            *slot = Some(Arc::clone(&state));
            state
        }
    }

    /// Borrow the profile so chain handles can open
    /// repositories on cache miss.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}
