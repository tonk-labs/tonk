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
use crate::env::SettleProvider;
use crate::subscription::Subscriber;

/// Resolved branch handle paired with the cache entry that owns
/// the connections streaming its subscriptions.
pub struct BranchSession {
    /// Cache entry — owns the dialog branch, its stack and the
    /// connections.
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

    /// Re-evaluate the branch's subscriptions: schedule its stack's poll
    /// (for a write the stack did not see, a pull or an import through
    /// the branch handle) and settle it, sending every connection what it
    /// was delivered ([`BranchState::settle`]).
    ///
    /// `impl Future + 'a` (not `async fn`) so the env lifetime stays
    /// named.
    pub fn poll<'a, Env: SettleProvider>(&'a self, env: &'a Env) -> impl Future<Output = ()> + 'a {
        self.state.stack().schedule_poll();
        self.state.settle(env)
    }

    /// Subscribe a fresh connection to `query`, optionally tagged with
    /// the client it serves (see [`BranchState::retain_subscribers`]).
    pub fn subscribe(&self, query: ConceptQuery, client: Option<String>, level: u32) -> Subscriber {
        self.state.subscribe(query, client, level)
    }
}
