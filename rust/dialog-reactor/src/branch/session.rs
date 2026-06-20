//! [`BranchSession`] — what `BranchReference::acquire` returns.
//!
//! Carries an `Arc<BranchState>`, so subscription operations on
//! the branch don't have to round-trip through the reactor's
//! name-keyed lookup. The dialog [`Branch`] handle is exposed
//! via [`Self::handle`] for direct dialog API use.

use std::sync::Arc;

use dialog_artifacts::Changes;
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

    /// A clone of this branch's session overlay, to fold into a read
    /// query via [`QueryLayer::with`](dialog_repository::QueryLayer).
    pub fn overlay(&self) -> Changes {
        self.state.overlay()
    }

    /// The inverse of this branch's overlay — retracts every overlay
    /// fact. Integrate before a commit so a read that folded the overlay
    /// in does not persist the ephemeral facts. See
    /// [`BranchState::overlay_retraction`](crate::BranchState::overlay_retraction).
    pub fn overlay_retraction(&self) -> Changes {
        self.state.overlay_retraction()
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
