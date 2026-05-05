//! [`BranchState`] — cached branch handle plus the subscriptions
//! registered against it. Lives behind an `Arc`; subscription
//! operations on the branch happen directly on the state without
//! routing through the reactor's name-keyed lookup.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_query::ConceptQuery;
use dialog_repository::Branch;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::reactor::env::SelectProvider;
use crate::reactor::error::ReactorError;
use crate::reactor::subscription::{
    QueryHash, Status, Subscriber, SubscriberSession, Subscription, SubscriptionPoll,
};

/// Cached branch handle plus the subscriptions registered
/// against it. Held inside the reactor's cache as
/// `Arc<BranchState>` so callers can hand the state around
/// without re-locking the reactor's outer map.
pub struct BranchState {
    /// The open dialog branch handle. Cheap to clone (internal
    /// `Cell<Revision>` keeps this handle current as the branch
    /// advances).
    pub branch: Branch,
    /// Subscriptions on this branch, keyed by query hash.
    subscriptions: Mutex<HashMap<QueryHash, Subscription>>,
}

impl BranchState {
    /// Construct a fresh state over an open branch.
    pub fn new(branch: Branch) -> Self {
        Self {
            branch,
            subscriptions: Mutex::new(HashMap::new()),
        }
    }

    /// Borrow the subscription map. Used by [`SubscriptionPoll`]
    /// to walk subscribers, and by tests asserting on cache state.
    pub fn subscriptions(&self) -> &Mutex<HashMap<QueryHash, Subscription>> {
        &self.subscriptions
    }

    /// Register a fresh subscriber for `query`. Returns a
    /// [`Subscriber`] carrying the subscription's hash and the
    /// receiver to read broadcast bytes from. The caller is
    /// expected to follow up with
    /// `branch_session.subscription(hash).poll().perform(&env)`
    /// so the new subscriber's first event is the current snapshot.
    pub fn subscribe(&self, query: ConceptQuery) -> Result<Subscriber, ReactorError> {
        let hash = QueryHash::from(&query);
        let (sender, receiver) = mpsc::unbounded_channel();

        let mut subs = self.subscriptions.lock();
        let entry = subs.entry(hash.clone());
        let subscription = entry.or_insert_with(|| Subscription {
            query: query.clone(),
            last_hash: None,
            subscribers: Vec::new(),
        });
        if subscription.query != query {
            return Err(ReactorError::QueryHashCollision);
        }
        subscription.subscribers.push(SubscriberSession {
            sender,
            status: Status::Pending,
        });

        Ok(Subscriber { hash, receiver })
    }

    /// Re-poll every subscription on this branch. Mutating leaf
    /// effects call this on success so changed query results
    /// fan out to subscribers. Each subscription is polled via
    /// the same `SubscriptionPoll::perform` path the public
    /// chain uses.
    pub async fn poll<Env: SelectProvider>(self: &Arc<Self>, env: &Env) {
        let hashes: Vec<QueryHash> = {
            let subs = self.subscriptions.lock();
            subs.keys().cloned().collect()
        };
        for hash in hashes {
            SubscriptionPoll { state: self, hash }.perform(env).await;
        }
    }
}
