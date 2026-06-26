//! [`BranchState`] — cached branch handle plus the subscriptions
//! registered against it. Lives behind an `Arc`; subscription
//! operations on the branch happen directly on the state without
//! routing through the reactor's name-keyed lookup.

use std::collections::HashMap;
use std::sync::Arc;

use dialog_artifacts::{Changes, Statement};
use dialog_query::ConceptQuery;
use dialog_repository::Branch;
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;

use crate::env::SelectProvider;
use crate::error::ReactorError;
use crate::rules::ConceptCache;
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
    /// In-memory session overlay — ephemeral facts folded into every
    /// read query (one-shot and subscription poll) via
    /// [`QueryLayer::with`](dialog_repository::QueryLayer), but never
    /// written to the branch tree and never replicated. Holds secrets
    /// that must stay out of storage (e.g. an invite's private seed):
    /// readable by the UI's display query, invisible to the
    /// transaction/commit path (dialog's transaction query is
    /// non-composable), and lost on branch eviction.
    ///
    /// `RwLock`, not `Mutex`: every read query takes the shared lock to
    /// clone the overlay in, so reads must not serialize against each
    /// other; only the rare overlay write takes the exclusive lock.
    overlay: RwLock<Changes>,
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
    /// Per-branch cache of resolved deductive rules, keyed by
    /// conclusion concept. Shared (`Arc`) into the
    /// [`ReactorRuleSource`](crate::ReactorRuleSource) handed to each
    /// read query so rule-resolution work is paid once per
    /// (concept, branch-head) rather than per query.
    rules: Arc<ConceptCache>,
}

impl BranchState {
    /// Construct a fresh state over an open branch.
    pub fn new(branch: Branch) -> Self {
        Self {
            branch,
            subscriptions: Mutex::new(HashMap::new()),
            overlay: RwLock::new(Changes::new()),
            transactor: tokio::sync::Mutex::new(()),
            rules: Arc::new(ConceptCache::new()),
        }
    }

    /// The per-branch deductive-rule cache. Cloned (cheap `Arc`) into
    /// a [`ReactorRuleSource`](crate::ReactorRuleSource) for each read.
    pub fn rule_cache(&self) -> Arc<ConceptCache> {
        Arc::clone(&self.rules)
    }

    /// A [`ReactorRuleSource`](crate::ReactorRuleSource) over this
    /// branch's deductive-rule cache, for
    /// [`QueryLayer::with_rules`](dialog_repository::QueryLayer::with_rules).
    /// Carries the current branch head so the cache can distinguish
    /// fresh entries from stale.
    pub fn rule_source(&self) -> crate::ReactorRuleSource {
        let head = self
            .branch
            .revision()
            .map(|revision| revision.tree)
            .unwrap_or_default();
        crate::ReactorRuleSource::new(self.rule_cache(), head)
    }

    /// The per-branch transaction lock. A transaction takes it
    /// (`transactor().lock().await`) around its commit so concurrent
    /// transactions serialize instead of failing the head CAS. Sync does not
    /// participate — see the field docs.
    pub fn transactor(&self) -> &tokio::sync::Mutex<()> {
        &self.transactor
    }

    /// A clone of the current session overlay, to fold into a read query
    /// via [`QueryLayer::with`](dialog_repository::QueryLayer). Taken
    /// under the shared read lock so concurrent reads don't serialize.
    pub fn overlay(&self) -> Changes {
        self.overlay.read().clone()
    }

    /// The inverse of the current overlay: a [`Changes`] that retracts
    /// every fact the overlay asserts. Integrate this into a transaction
    /// *before* committing so an evaluation that folded the overlay in for
    /// reads does not carry the ephemeral facts (e.g. an invite seed) into
    /// the durable write. Asserts/replaces become retracts; existing
    /// retracts are dropped (nothing to undo).
    pub fn overlay_retraction(&self) -> Changes {
        use dialog_artifacts::{Change, Update};
        let overlay = self.overlay.read();
        let mut inverse = Changes::new();
        for (entity, attribute, change) in overlay.iter() {
            if let Change::Assert(value) | Change::Replace(value) = change {
                inverse.dissociate(attribute.clone(), entity.clone(), value.clone());
            }
        }
        inverse
    }

    /// Assert a [`Statement`] into the session overlay. A cardinality-one
    /// re-assert overwrites the prior value in place, so "keep exactly
    /// one live fact" needs no separate retract. Takes the exclusive
    /// write lock.
    pub fn assert_overlay<S: Statement>(&self, claim: S) {
        let mut overlay = self.overlay.write();
        claim.assert(&mut *overlay);
    }

    /// Retract a [`Statement`] from the session overlay. Takes the
    /// exclusive write lock.
    pub fn retract_overlay<S: Statement>(&self, claim: S) {
        let mut overlay = self.overlay.write();
        claim.retract(&mut *overlay);
    }

    /// Drop every fact in the session overlay. Used to keep exactly one
    /// live entry across keys that differ between writes (e.g. each
    /// invitation is keyed by a fresh membership DID, so a cardinality-one
    /// re-assert wouldn't replace the prior one — clearing does).
    pub fn clear_overlay(&self) {
        *self.overlay.write() = Changes::new();
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
