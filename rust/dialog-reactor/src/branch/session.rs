//! [`BranchSession`] — what `BranchReference::acquire` returns.
//!
//! Carries an `Arc<BranchState>`, so subscription operations on
//! the branch don't have to round-trip through the reactor's
//! name-keyed lookup. The dialog [`Branch`] handle is exposed
//! via [`Self::handle`] for direct dialog API use.

use std::future::Future;
use std::sync::Arc;

use dialog_query::ConceptQuery;
use dialog_repository::Branch;

use crate::BranchState;
use crate::env::SelectProvider;
use crate::error::ReactorError;
use crate::subscription::{QueryHash, Subscriber, SubscriptionReference};

/// Resolved branch handle paired with the cache entry that owns
/// the subscriptions registered against it.
pub struct BranchSession {
    /// Cache entry — owns the dialog branch and the subscription
    /// table.
    pub state: Arc<BranchState>,
}

impl BranchSession {
    /// Borrow the underlying dialog branch handle.
    pub fn handle(&self) -> &Branch {
        &self.state.branch
    }

    /// The per-branch transaction lock. A transaction takes it
    /// (`transactor().lock().await`) around its commit so concurrent
    /// transactions on this branch serialize instead of racing the head CAS.
    /// Sync does not take it — see [`BranchState::transactor`].
    pub fn transactor(&self) -> &tokio::sync::Mutex<()> {
        self.state.transactor()
    }

    /// Re-poll every subscription on this branch.
    ///
    /// `impl Future + 'a` (not `async fn`) so the env lifetime stays
    /// named — see [`SubscriptionPoll::perform`](crate::SubscriptionPoll).
    pub fn poll<'a, Env: SelectProvider>(&'a self, env: &'a Env) -> impl Future<Output = ()> + 'a {
        self.state.poll(env)
    }

    /// Register a fresh subscriber for `query`. Returns a
    /// [`Subscriber`] carrying the subscription's hash and the
    /// receiver to read broadcast bytes from.
    pub fn subscribe(&self, query: ConceptQuery) -> Result<Subscriber, ReactorError> {
        self.state.subscribe(query)
    }

    /// Reference a single subscription on this branch by its
    /// hash. Chain `.poll().perform(&env)` to re-evaluate.
    pub fn subscription(&self, hash: QueryHash) -> SubscriptionReference<'_> {
        SubscriptionReference {
            state: &self.state,
            hash,
        }
    }
}
