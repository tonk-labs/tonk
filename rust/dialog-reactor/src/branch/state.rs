//! [`BranchState`] — cached branch handle plus the subscriptions
//! registered against it. Lives behind an `Arc`; subscription
//! operations on the branch happen directly on the state without
//! routing through the reactor's name-keyed lookup.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use dialog_artifacts::Statement;
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
    /// Serializes *transactions* on this branch — concurrent writers (e.g.
    /// two browser tabs committing through one service worker) line up rather
    /// than racing the head CAS and failing. Guards nothing but the right to be
    /// the one committing; the commit's data lives on the branch handle.
    ///
    /// An async [`tokio::sync::Mutex`], not `parking_lot`, because the guard is
    /// held *across* the commit's `await`s. A mutex, not an `RwLock`: only
    /// writers take it (readers and sync don't), so there is no read side to
    /// share. Deliberately separate from the reactor's `TonkState` lock and NOT
    /// taken by sync: sync coordinates with transactions through the head CAS
    /// (it refreshes and retries on a mismatch), so it must never wait on — or
    /// block — a transaction. Per branch, so commits to different branches
    /// proceed in parallel.
    transactor: tokio::sync::Mutex<()>,
}

impl BranchState {
    /// Construct a fresh state over an open branch.
    pub fn new(branch: Branch) -> Self {
        Self {
            branch,
            subscriptions: Mutex::new(HashMap::new()),
            transactor: tokio::sync::Mutex::new(()),
        }
    }

    /// The per-branch transaction lock. A transaction takes it
    /// (`transactor().lock().await`) around its commit so concurrent
    /// transactions serialize instead of failing the head CAS. Sync does not
    /// participate — see the field docs.
    pub fn transactor(&self) -> &tokio::sync::Mutex<()> {
        &self.transactor
    }

    /// Assert a [`Statement`] into the branch's session overlay —
    /// ephemeral facts folded into every read of the branch (queries,
    /// transaction views, and standing subscriptions) but never
    /// committed. Delegates to [`Branch::overlay`]: dialog owns the
    /// overlay now, folds it in at the single `QueryLayer::from(&Branch)`
    /// point, and bumps an epoch so the branch's subscriptions re-evaluate
    /// on their next poll. A cardinality-one re-assert overwrites in place.
    pub fn assert_overlay<S: Statement>(&self, claim: S) {
        self.branch.overlay().assert(claim);
    }

    /// Retract a [`Statement`] from the branch's session overlay.
    pub fn retract_overlay<S: Statement>(&self, claim: S) {
        self.branch.overlay().retract(claim);
    }

    /// Drop every fact in the branch's session overlay. Used to keep
    /// exactly one live entry across keys that differ between writes (e.g.
    /// each invitation is keyed by a fresh membership DID, so a
    /// cardinality-one re-assert wouldn't replace the prior one — clearing
    /// does).
    pub fn clear_overlay(&self) {
        self.branch.overlay().clear();
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

        let terms = query.terms.clone();

        let mut subs = self.subscriptions.lock();
        let entry = subs.entry(hash.clone());
        // Route through `QueryPlan::from` (the same projection the one-shot
        // `QueryEffect` applies) so a concept-of-concept / command / rule
        // metadata query dispatches to its anonymous-enumeration application
        // instead of scanning for `dialog.meta/*` facts that aren't stored.
        // The branch folds its own session overlay into every read, so the
        // subscription sees ephemeral facts with no extra wiring here.
        let plan = tonk_schema::concept::QueryPlan::from(query);
        let subscription = entry.or_insert_with(|| Subscription {
            engine: Arc::new(tokio::sync::Mutex::new(Some(self.branch.subscribe(plan)))),
            terms,
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
    // `impl Future + 'a` (not `async fn`) so the env lifetime is
    // named, keeping callers (the axum handlers draining scheduled
    // polls) Send-general — see `SubscriptionPoll::perform`.
    pub fn poll<'a, Env: SelectProvider>(
        self: &'a Arc<Self>,
        env: &'a Env,
    ) -> impl Future<Output = ()> + 'a {
        async move {
            let hashes: Vec<QueryHash> = {
                let subs = self.subscriptions.lock();
                subs.keys().cloned().collect()
            };
            for hash in hashes {
                SubscriptionPoll { state: self, hash }.perform(env).await;
            }
        }
    }
}
