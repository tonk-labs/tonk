//! [`BranchState`] — cached branch handle plus the subscriptions
//! registered against it. Lives behind an `Arc`; subscription
//! operations on the branch happen directly on the state without
//! routing through the reactor's name-keyed lookup.

use std::sync::Arc;

use bytes::Bytes;
use dialog_artifacts::Statement;
use dialog_query::ConceptQuery;
use dialog_repository::Branch;
use indexmap::IndexMap;
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
    /// Subscriptions on this branch, keyed by query hash, in the order
    /// they were created. [`Self::poll`] walks them lowest level first (see
    /// [`Subscription::level`]), creation order breaking ties.
    subscriptions: Mutex<IndexMap<QueryHash, Subscription>>,
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
    /// Whether [`BranchReference::migrate_once`] has run this branch's one
    /// migration since it was opened (to completion or to a failure). A
    /// call that found nothing to move does not set it.
    ///
    /// [`BranchReference::migrate_once`]: crate::BranchReference::migrate_once
    migration_claimed: std::sync::atomic::AtomicBool,
    /// Whether that migration is running now: claimed, found needed, and
    /// not yet finished. A page asking whether to show an upgrade notice
    /// reads it.
    migrating: std::sync::atomic::AtomicBool,
}

impl BranchState {
    /// Construct a fresh state over an open branch.
    pub fn new(branch: Branch) -> Self {
        Self {
            branch,
            subscriptions: Mutex::new(IndexMap::new()),
            transactor: tokio::sync::Mutex::new(()),
            migration_claimed: std::sync::atomic::AtomicBool::new(false),
            migrating: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Mark the one migration this branch runs while open as run.
    pub(crate) fn claim_migration(&self) {
        self.migration_claimed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether the migration has run, to completion or to a failure.
    pub(crate) fn migration_claimed(&self) -> bool {
        self.migration_claimed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Mark the migration as running, or as finished.
    pub(crate) fn set_migrating(&self, running: bool) {
        self.migrating
            .store(running, std::sync::atomic::Ordering::Release);
    }

    /// Whether the migration is running now.
    pub fn is_migrating(&self) -> bool {
        self.migrating.load(std::sync::atomic::Ordering::Acquire)
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
    ///
    /// The overlay holds facts only, so a statement that changes an asset
    /// is refused; that is a mistake in the caller, reported in the log.
    pub fn assert_overlay<S: Statement>(&self, claim: S) {
        if let Err(error) = self.branch.overlay().assert(claim) {
            dialog_common::log!("overlay assert refused: {error}");
        }
    }

    /// Retract a [`Statement`] from the branch's session overlay. A
    /// statement that changes an asset is refused, as for
    /// [`assert_overlay`](Self::assert_overlay).
    pub fn retract_overlay<S: Statement>(&self, claim: S) {
        if let Err(error) = self.branch.overlay().retract(claim) {
            dialog_common::log!("overlay retract refused: {error}");
        }
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
    pub fn subscriptions(&self) -> &Mutex<IndexMap<QueryHash, Subscription>> {
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
    /// Attach a subscriber that already owns its channel.
    ///
    /// The adoption path for a subscription registered before this
    /// branch existed: its sender is already wired to a stream the page
    /// is holding open, so re-minting a channel here would deliver
    /// frames nowhere. Otherwise identical to [`Self::subscribe`] —
    /// same plan, same dedup by query hash — so an adopted subscriber
    /// joins whatever subscription its peers are already on.
    pub fn adopt_subscriber(
        &self,
        query: ConceptQuery,
        client: Option<String>,
        level: u32,
        sender: mpsc::UnboundedSender<Bytes>,
    ) -> QueryHash {
        let hash = QueryHash::from(&query);
        self.install_subscriber(query, client, level, sender);
        hash
    }

    pub fn subscribe(
        &self,
        query: ConceptQuery,
        client: Option<String>,
        level: u32,
    ) -> Result<Subscriber, ReactorError> {
        let hash = QueryHash::from(&query);
        let (sender, receiver) = mpsc::unbounded_channel();
        self.install_subscriber(query, client, level, sender);
        Ok(Subscriber { hash, receiver })
    }

    /// Register `sender` against `query`'s subscription, creating the
    /// subscription (and its engine) if this is the first subscriber.
    /// Shared by [`Self::subscribe`] and [`Self::adopt_subscriber`] so
    /// the two cannot drift.
    fn install_subscriber(
        &self,
        query: ConceptQuery,
        client: Option<String>,
        level: u32,
        sender: mpsc::UnboundedSender<Bytes>,
    ) {
        let hash = QueryHash::from(&query);

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
            client,
            level,
        });
    }

    /// Drop every subscriber whose `client` tag fails `keep`; an
    /// untagged subscriber (`None`) is always kept. A subscription
    /// left with no subscribers is removed with its engine.
    ///
    /// This is the liveness-driven prune: send-failure pruning in
    /// `fan_out` only fires once the receiver is actually dropped,
    /// which a vanished client may never trigger — its stale
    /// subscription would re-evaluate on every poll forever.
    pub fn retain_subscribers<F: Fn(&str) -> bool>(&self, keep: F) {
        let mut subs = self.subscriptions.lock();
        subs.retain(|_, subscription| {
            subscription
                .subscribers
                .retain(|s| s.client.as_deref().is_none_or(&keep));
            !subscription.subscribers.is_empty()
        });
    }

    /// Drop every session-overlay fact recorded for entities that
    /// fail `keep` — the branch-level surface of
    /// [`Overlay::retain_entities`](dialog_repository::Overlay).
    /// Returns whether anything was removed; the caller schedules a
    /// poll when it was, so live subscribers observe the removal.
    pub fn retain_overlay_entities<F: FnMut(&dialog_artifacts::Entity) -> bool>(
        &self,
        keep: F,
    ) -> bool {
        self.branch.overlay().retain_entities(keep)
    }

    /// Re-poll every subscription on this branch. Mutating leaf
    /// effects call this on success so changed query results
    /// fan out to subscribers. Each subscription is polled via
    /// the same `SubscriptionPoll::perform` path the public
    /// chain uses.
    ///
    /// Lower levels go first. A display's level is how many displays
    /// enclose it, so it learns of a change before the displays nested
    /// inside it do: a parent that re-renders
    /// its children away first spares them a frame for an address it is
    /// replacing. Creation order breaks ties (a stable sort over the
    /// insertion-ordered map).
    pub async fn poll<'a, Env: SelectProvider>(self: &'a Arc<Self>, env: &'a Env) {
        let hashes: Vec<QueryHash> = {
            let subs = self.subscriptions.lock();
            let mut ordered: Vec<(u32, QueryHash)> = subs
                .iter()
                .map(|(hash, subscription)| (subscription.level(), hash.clone()))
                .collect();
            ordered.sort_by_key(|(level, _)| *level);
            ordered.into_iter().map(|(_, hash)| hash).collect()
        };
        // A poll can await the network (a node still held remotely), and
        // an overlay write re-evaluates every subscription on the branch,
        // so a slow one is named with what it serves.
        const SLOW: web_time::Duration = web_time::Duration::from_millis(500);
        let started = web_time::Instant::now();
        let count = hashes.len();
        for hash in hashes {
            let polled = web_time::Instant::now();
            let (level, subscribers) = {
                let subs = self.subscriptions.lock();
                subs.get(&hash)
                    .map(|subscription| (subscription.level(), subscription.subscribers.len()))
                    .unwrap_or_default()
            };
            SubscriptionPoll {
                state: self,
                hash: hash.clone(),
            }
            .perform(env)
            .await;
            let elapsed = polled.elapsed();
            if elapsed >= SLOW {
                dialog_common::log!(
                    "[reactor] slow subscription poll: {hash:?} level={level} subscribers={subscribers} elapsed_ms={}",
                    elapsed.as_millis()
                );
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= SLOW {
            dialog_common::log!(
                "[reactor] slow branch poll: {count} subscriptions elapsed_ms={}",
                elapsed.as_millis()
            );
        }
    }
}
