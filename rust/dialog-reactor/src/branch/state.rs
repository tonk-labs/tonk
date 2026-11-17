//! [`BranchState`] — cached branch handle, the stack over it, and the
//! connections streaming its subscriptions. Lives behind an `Arc`;
//! subscription operations on the branch happen directly on the state
//! without routing through the reactor's name-keyed lookup.

use std::sync::Arc;

use bytes::Bytes;
use dialog_corpus::Statement;
use dialog_query::ConceptQuery;
use dialog_repository::{Branch, CommitError, Stack};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::env::SettleProvider;
use crate::subscription::{Connection, Listener, Plan, QueryHash, Subscriber};

/// Cached branch handle plus the subscriptions registered
/// against it. Held inside the reactor's cache as
/// `Arc<BranchState>` so callers can hand the state around
/// without re-locking the reactor's outer map.
pub struct BranchState {
    /// The open dialog branch handle. Cheap to clone (internal
    /// `Cell<Revision>` keeps this handle current as the branch
    /// advances).
    pub branch: Branch,
    /// The [`Stack`] every commit on this branch goes through: the
    /// branch as primary with the layers its schema declares, and the
    /// `control:` loopback a committed command lands in for one tick.
    /// Opened with the branch ([`acquire`]) and shared by every writer
    /// after; see [`Self::stack`].
    ///
    /// [`acquire`]: crate::BranchReference::acquire
    stack: Stack,
    /// Whether the embedder has registered its providers on the stack
    /// ([`provide_once`](Self::provide_once)).
    provided: std::sync::atomic::AtomicBool,
    /// The connections streaming this branch's subscriptions, in the
    /// order they were made. The evaluations are the stack's
    /// ([`Stack::subscribe`]); [`Self::deliver`] sends each connection
    /// what the stack delivered it, lowest level first.
    listeners: Mutex<Vec<Listener>>,
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
    /// Construct a fresh state over an open branch and the stack opened
    /// over it.
    pub fn new(branch: Branch, stack: Stack) -> Self {
        Self {
            branch,
            stack,
            provided: std::sync::atomic::AtomicBool::new(false),
            listeners: Mutex::new(Vec::new()),
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

    /// The stack every commit on this branch goes through, opened over
    /// [`Self::branch`] when the branch was: it holds a layer for each
    /// the primary's schema declares and the `control:` loopback. A
    /// clone shares the layers, so the one stack serves every writer and
    /// every reader of what their dispatches land.
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    /// Register the embedder's providers on the stack, once per
    /// `BranchState`: `provide` runs the first time, and a later call
    /// returns at once. The state is opened by whoever acquires the
    /// branch first, with no embedder state in hand, so the embedder
    /// registers here, when it first performs an effect on the branch,
    /// which is before any provider is looked up. A registration that
    /// fails is reported and leaves the state unprovided, so the next
    /// call tries again.
    pub fn provide_once<F>(&self, provide: F) -> Result<(), CommitError>
    where
        F: FnOnce(&Stack) -> Result<(), CommitError>,
    {
        use std::sync::atomic::Ordering;
        if self
            .provided
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        provide(&self.stack).inspect_err(|_| self.provided.store(false, Ordering::Release))
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
    /// overlay, and every read through the stack reads it as rows of the
    /// primary. The stack does not see a write into it, so this schedules
    /// the stack's poll ([`Stack::schedule_poll`]), which the next settle
    /// runs. A cardinality-one re-assert overwrites in place.
    ///
    /// The overlay holds facts only, so a statement that changes an asset
    /// is refused; that is a mistake in the caller, reported in the log.
    pub fn assert_overlay<S: Statement>(&self, claim: S) {
        if let Err(error) = self.branch.overlay().assert(claim) {
            dialog_common::log!("overlay assert refused: {error}");
        }
        self.stack.schedule_poll();
    }

    /// Retract a [`Statement`] from the branch's session overlay. A
    /// statement that changes an asset is refused, as for
    /// [`assert_overlay`](Self::assert_overlay).
    pub fn retract_overlay<S: Statement>(&self, claim: S) {
        if let Err(error) = self.branch.overlay().retract(claim) {
            dialog_common::log!("overlay retract refused: {error}");
        }
        self.stack.schedule_poll();
    }

    /// Drop every fact in the branch's session overlay. Used to keep
    /// exactly one live entry across keys that differ between writes (e.g.
    /// each invitation is keyed by a fresh membership DID, so a
    /// cardinality-one re-assert wouldn't replace the prior one — clearing
    /// does).
    pub fn clear_overlay(&self) {
        self.branch.overlay().clear();
        self.stack.schedule_poll();
    }

    /// Drop every connection on this branch, so each one's stream ends.
    ///
    /// Each listener owns its connection's sender; dropping it surfaces
    /// `None` on the receiver side, which ends the stream driving the
    /// SSE response body, so the in-flight fetch settles and drops its
    /// [`Subscriber`], and with it the dialog reader. Called from the
    /// worker's `onupdatefound` path so the old SW can be replaced —
    /// without this, the SW spec keeps the worker alive for as long as
    /// any open fetch still holds a stream.
    pub fn clear_subscribers(&self) {
        self.listeners.lock().clear();
    }

    /// Subscribe a fresh connection to `query`: a reader of the stack's
    /// subscription to it, registered now or attached to the one the
    /// stack holds. Its frames arrive once [`Self::settle`] (or the
    /// reactor's `run_scheduled_polls`) has delivered: the first is the
    /// whole result set.
    pub fn subscribe(&self, query: ConceptQuery, client: Option<String>, level: u32) -> Subscriber {
        let (sender, receiver) = mpsc::unbounded_channel();
        let connection = self.connect(query, client, level, sender);
        Subscriber::new(receiver, connection)
    }

    /// Attach a connection that already owns its channel: the adoption
    /// path for a subscription registered before this branch existed,
    /// whose sender is already wired to a stream the page is holding
    /// open. Its [`Connection`] is the one the waiting subscriber holds,
    /// handed its reader here; one already gone attaches nothing.
    pub(crate) fn adopt(&self, pending: crate::PendingSubscription) {
        let Some(connection) = pending.connection.upgrade() else {
            return;
        };
        connection.attach(self.stack.subscribe(Plan::new(pending.query)));
        self.listeners.lock().push(Listener {
            connection: Arc::downgrade(&connection),
            sender: pending.sender,
            client: pending.client,
            level: pending.level,
        });
    }

    /// Register a connection reading `query`, whose frames go into
    /// `sender`.
    pub fn connect(
        &self,
        query: ConceptQuery,
        client: Option<String>,
        level: u32,
        sender: mpsc::UnboundedSender<Bytes>,
    ) -> Arc<Connection> {
        let terms = query.terms.clone();
        let reader = self.stack.subscribe(Plan::new(query));
        let connection = Connection::new(Some(reader), terms);
        self.listeners.lock().push(Listener {
            connection: Arc::downgrade(&connection),
            sender,
            client,
            level,
        });
        connection
    }

    /// Drop every connection whose `client` tag fails `keep`; an
    /// untagged connection (`None`) is always kept. Dropping a
    /// connection's listener ends its client's stream, which drops the
    /// dialog reader the client held; a subscription left with no reader
    /// goes with it.
    ///
    /// This is the liveness-driven prune: a vanished client may never
    /// close its stream, so its connection would otherwise keep its
    /// subscription evaluating on every poll forever.
    pub fn retain_subscribers<F: Fn(&str) -> bool>(&self, keep: F) {
        self.listeners
            .lock()
            .retain(|listener| listener.client.as_deref().is_none_or(&keep));
    }

    /// The levels of this branch's live connections, in the order they
    /// were made.
    pub fn connection_levels(&self) -> Vec<u32> {
        self.listeners
            .lock()
            .iter()
            .filter(|listener| listener.connection.strong_count() > 0)
            .map(|listener| listener.level)
            .collect()
    }

    /// How many subscriptions the branch's stack holds that some
    /// connection still reads.
    pub fn subscriptions_held(&self) -> usize {
        self.stack.subscriptions_held()
    }

    /// Send every connection the frames the stack delivered it since its
    /// last delivery, subscription by subscription, lowest level first: a
    /// display's level is how many displays enclose it, so it learns of a
    /// change before the displays nested inside it do, and a parent that
    /// re-renders its children away first spares them a frame for an
    /// address it is replacing. A subscription shared across levels is
    /// delivered at its outermost reader's, to all its readers at once;
    /// the order subscriptions were first read breaks ties. A connection
    /// whose client has dropped it, or stopped listening, is pruned.
    pub fn deliver(&self) {
        let mut listeners = self.listeners.lock();
        // Each listener's subscription, as the hash its connection reads
        // (a connection gone, or still waiting for its reader, has none).
        let hashes: Vec<Option<QueryHash>> = listeners
            .iter()
            .map(|listener| listener.connection.upgrade().and_then(|c| c.hash()))
            .collect();
        // Each subscription's rank: its outermost reader's level, then
        // where it was first read.
        let mut ranks: Vec<(Option<QueryHash>, u32, usize)> = Vec::new();
        for (index, (listener, hash)) in listeners.iter().zip(&hashes).enumerate() {
            match ranks
                .iter_mut()
                .find(|(seen, _, _)| seen == hash && hash.is_some())
            {
                Some((_, level, _)) => *level = (*level).min(listener.level),
                None => ranks.push((hash.clone(), listener.level, index)),
            }
        }
        let rank = |index: usize| {
            ranks
                .iter()
                .find(|(seen, _, first)| {
                    *first == index || (hashes[index].is_some() && *seen == hashes[index])
                })
                .map(|(_, level, first)| (*level, *first))
                .unwrap_or((u32::MAX, index))
        };
        let mut order: Vec<usize> = (0..listeners.len()).collect();
        order.sort_by_key(|index| rank(*index));
        let mut live = vec![true; listeners.len()];
        for index in order {
            live[index] = listeners[index].deliver();
        }
        let mut live = live.into_iter();
        listeners.retain(|_| live.next().unwrap_or(true));
    }

    /// Settle the branch's stack, polling its subscriptions once if a
    /// write scheduled a poll ([`Stack::settle`]), then send every
    /// connection what it was delivered ([`Self::deliver`]). A
    /// connection attached to a subscription already evaluated has its
    /// result set waiting, and is sent it even when no poll was
    /// scheduled.
    ///
    /// A poll can await the network (a node still held remotely), and a
    /// write re-evaluates every subscription on the stack, so a slow one
    /// is named with what it serves.
    pub async fn settle<Env: SettleProvider>(&self, env: &Env) {
        const SLOW: web_time::Duration = web_time::Duration::from_millis(500);
        let started = web_time::Instant::now();
        if let Err(error) = self.stack.settle(env).await {
            dialog_common::log!(
                "[reactor] subscription poll on {} failed: {error:?}",
                self.branch.name()
            );
        }
        let elapsed = started.elapsed();
        if elapsed >= SLOW {
            dialog_common::log!(
                "[reactor] slow branch settle: {} subscriptions on {} elapsed_ms={}",
                self.stack.subscriptions_held(),
                self.branch.name(),
                elapsed.as_millis()
            );
        }
        self.deliver();
    }

    /// Drop every session-overlay fact recorded for entities that
    /// fail `keep` — the branch-level surface of
    /// [`Overlay::retain_entities`](dialog_repository::Overlay).
    /// Returns whether anything was removed; the caller schedules a
    /// poll when it was, so live subscribers observe the removal.
    pub fn retain_overlay_entities<F: FnMut(&dialog_corpus::Entity) -> bool>(
        &self,
        keep: F,
    ) -> bool {
        self.branch.overlay().retain_entities(keep)
    }
}
