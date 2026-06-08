//! [`TonkReactor`] — the worker's reactive layer over branches.
//!
//! See `reactor-spec.md` next to the crate's `Cargo.toml` for the
//! full design rationale. The short version: routes mutate
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
use parking_lot::Mutex;

mod branch;
mod command;
mod env;
mod error;
mod pull;
mod push;
mod repository;
mod subscribe;
mod subscription;
mod transaction;

pub use branch::{BranchReference, BranchSession, BranchState};
pub use command::{CommandHandler, CommandRegistry, Decode, EntityFacts, Env, TypedCommand};
pub use env::{
    BranchOpenProvider, CommitProvider, LoadProvider, PullProvider, PushProvider, SelectProvider,
};
pub use error::ReactorError;
pub use pull::Pull;
pub use push::Push;
pub use repository::{RepositoryReference, RepositoryState};
pub use subscribe::Subscribe;
pub use subscription::{QueryHash, Subscriber, SubscriptionPoll, SubscriptionReference};
/// On-the-wire `Conclusion` and `Query` — re-exported from
/// [`tonk_schema`] so consumers (browser clients, the
/// `<tonk-concept>` element) can deserialize without depending
/// on this crate.
pub use tonk_schema::conclusion::Conclusion;
pub use tonk_schema::query::Query;
pub use transaction::{Commit, TransactionBuilder};

/// The worker's reactive layer. Owned by `TonkState`.
pub struct TonkReactor {
    profile: Profile,
    repos: Mutex<HashMap<String, Arc<RepositoryState>>>,
    /// Cached `RepositoryState` for the profile-as-repository.
    /// Lazily populated on first `profile_repository().acquire()`
    /// call; lives outside `repos` because the profile is a
    /// singleton with no name in the routing namespace.
    profile_repo: Mutex<Option<Arc<RepositoryState>>>,
}

impl TonkReactor {
    /// Construct a reactor over the given profile. The reactor
    /// doesn't own an operator — every effect takes one at
    /// `perform` time, matching dialog's command/perform pattern.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            repos: Mutex::new(HashMap::new()),
            profile_repo: Mutex::new(None),
        }
    }

    /// Drop every cached handle and every active SSE subscriber
    /// so open response bodies finish. Called from the SW upgrade
    /// path so the old worker can be replaced.
    ///
    /// Walking the cache and explicitly dropping subscriber
    /// senders is the load-bearing step: the
    /// [`BranchState`](crate::reactor::BranchState) `Arc`s are
    /// shared with `SubscriptionPoll` futures still holding a
    /// reference, so removing the cache entry alone isn't enough.
    /// Clearing each branch's subscriber map drops every
    /// `mpsc::Sender`, which surfaces `None` on the receiver side
    /// and ends the SSE response stream regardless of who else
    /// holds the state.
    pub fn shutdown(&self) {
        let repos = {
            let mut map = self.repos.lock();
            std::mem::take(&mut *map)
        };
        // The profile-as-repository lives in its own slot, not in
        // `repos`. The Hub subscribes to its meta branch
        // (`/api/profile/branch/meta/query` SSE), so it must be drained
        // too — otherwise that one stream stays open and pins the
        // outgoing worker in `waiting` on every update.
        let profile = self.profile_repo.lock().take();
        for repo in repos.into_values().chain(profile) {
            let branches = {
                let mut map = repo.branches().lock();
                std::mem::take(&mut *map)
            };
            for (_, branch) in branches {
                branch.clear_subscribers();
            }
        }
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
    pub fn repos(&self) -> &Mutex<HashMap<String, Arc<RepositoryState>>> {
        &self.repos
    }

    /// Snapshot the cached profile-as-repository state, if any.
    /// Used by `RepositoryReference::Profile::acquire` for the
    /// fast-path branch.
    pub fn profile_repo_state(&self) -> Option<Arc<RepositoryState>> {
        self.profile_repo.lock().clone()
    }

    /// Install the profile-as-repository state into the cache.
    /// Returns the resident value — if another caller raced and
    /// installed first, theirs wins (state is fungible).
    pub fn set_profile_repo_state(&self, state: Arc<RepositoryState>) -> Arc<RepositoryState> {
        let mut slot = self.profile_repo.lock();
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
