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

use crate::env::SelectProvider;
use crate::error::ReactorError;
use crate::subscription::{
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

    /// Drop every subscriber session on this branch.
    ///
    /// Each session owns an `mpsc::Sender`; dropping it surfaces
    /// `None` on the receiver side, which ends the
    /// `UnboundedReceiverStream` driving the SSE response body, so
    /// the in-flight fetch settles. Called from the worker's
    /// `onupdatefound` path so the old SW can be replaced —
    /// without this, the SW spec keeps the worker alive for as long
    /// as any open fetch still holds a stream.
    pub fn clear_subscribers(&self) {
        self.subscriptions.lock().clear();
    }

    /// Move every subscription registered on `other` into this state,
    /// leaving `other` empty.
    ///
    /// Used when the reactor replaces a cached branch handle (e.g. to
    /// pick up an upstream wired on a separate handle): the fresh
    /// [`BranchState`] adopts the live subscribers so their SSE streams
    /// keep updating instead of silently freezing on the discarded
    /// handle. Each `SubscriberSession` carries its `mpsc::Sender`, so
    /// re-polls through the new state reach the same receivers; the
    /// subscriptions keep their `last_hash`, so the next poll only
    /// re-emits on a genuine change.
    pub fn adopt_subscriptions_from(&self, other: &BranchState) {
        let moved = std::mem::take(&mut *other.subscriptions.lock());
        *self.subscriptions.lock() = moved;
    }

    /// Register a fresh subscriber for `query`. Returns a
    /// [`Subscriber`] carrying the subscription's hash and the
    /// receiver to read broadcast bytes from. The caller is
    /// expected to follow up with
    /// `branch_session.subscription(hash).poll().perform(&env)`
    /// so the new subscriber's first event is the current snapshot.
    ///
    /// Query identity is the blake3 hash of the serialized
    /// [`Query`] projection. We **don't** re-check `PartialEq`
    /// against the registered query, even though earlier
    /// revisions did: `NamedAttributes` in dialog-query derives
    /// `PartialEq` over a `Vec` whose order is randomized by the
    /// `HashMap`-mediated `Serialize` / `Deserialize` impls, so
    /// the same query round-tripped through ser/de can compare
    /// `!=` even though the hashes match. A genuine blake3
    /// collision is cryptographically impossible, so trusting
    /// the hash is the right move here. Track the upstream fix
    /// in dialog-db (make `NamedAttributes::PartialEq`
    /// order-insensitive, or serialize in sorted order).
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
