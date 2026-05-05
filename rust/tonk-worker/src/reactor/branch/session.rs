//! [`BranchSession`] — what `BranchReference::acquire` returns.
//!
//! Carries an `Arc<BranchState>`, so subscription operations on
//! the branch don't have to round-trip through the reactor's
//! name-keyed lookup. The dialog [`Branch`] handle is exposed
//! via [`Self::handle`] for direct dialog API use.

use std::sync::Arc;

use dialog_query::ConceptQuery;
use dialog_repository::Branch;

use crate::reactor::BranchState;
use crate::reactor::env::SelectProvider;
use crate::reactor::error::ReactorError;
use crate::reactor::subscription::{QueryHash, Subscriber, SubscriptionReference};

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

    /// Re-poll every subscription on this branch.
    pub async fn poll<Env: SelectProvider>(&self, env: &Env) {
        self.state.poll(env).await;
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
