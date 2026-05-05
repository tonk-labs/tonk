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
mod conclusion;
mod env;
mod error;
mod pull;
mod push;
mod query;
mod repository;
mod subscribe;
mod subscription;
mod transaction;

pub use branch::{BranchReference, BranchSession, BranchState};
pub use conclusion::Conclusion;
pub use env::{
    BranchOpenProvider, CommitProvider, LoadProvider, PullProvider, PushProvider, SelectProvider,
};
pub use error::ReactorError;
pub use pull::Pull;
pub use push::Push;
pub use query::Query;
pub use repository::{RepositoryReference, RepositoryState};
pub use subscribe::Subscribe;
pub use subscription::{QueryHash, Subscriber, SubscriptionPoll, SubscriptionReference};
pub use transaction::{Commit, TransactionBuilder};

/// The worker's reactive layer. Owned by `TonkState`.
pub struct TonkReactor {
    profile: Profile,
    repos: Mutex<HashMap<String, Arc<RepositoryState>>>,
}

impl TonkReactor {
    /// Construct a reactor over the given profile. The reactor
    /// doesn't own an operator — every effect takes one at
    /// `perform` time, matching dialog's command/perform pattern.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            repos: Mutex::new(HashMap::new()),
        }
    }

    /// Drop every cached handle so open SSE response bodies
    /// finish. Called from the SW upgrade path so the old worker
    /// can be replaced.
    pub fn shutdown(&self) {
        self.repos.lock().clear();
    }

    /// Begin a chain scoped to the named repository.
    pub fn repository<'a>(&'a self, name: &'a str) -> RepositoryReference<'a> {
        RepositoryReference {
            reactor: self,
            name,
        }
    }

    /// Borrow the cache map. Public so the chain handles
    /// (`RepositoryReference::acquire`, `BranchReference::acquire`)
    /// can run their lookup-and-open logic directly without
    /// indirecting through helper methods.
    pub fn repos(&self) -> &Mutex<HashMap<String, Arc<RepositoryState>>> {
        &self.repos
    }

    /// Borrow the profile so chain handles can open
    /// repositories on cache miss.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}
