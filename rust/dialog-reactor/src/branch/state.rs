//! [`BranchState`] — a branch as the reactor holds it: the branch, the
//! stack every read, subscription and write of it goes through, and
//! the subscriptions registered against it. Lives behind an `Arc`;
//! subscription operations on the branch happen directly on the state
//! without routing through the reactor's name-keyed lookup.

use std::collections::HashSet;
use std::sync::Arc;

use bytes::Bytes;
use dialog_artifacts::{Attribute, Changes, Entity, Statement};
use dialog_query::ConceptQuery;
use dialog_repository::placement::Target as StoreTarget;
use dialog_repository::{Branch, Drained, Ephemeral, Observer, Placement, Stack};
use indexmap::IndexMap;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::env::{BranchOpenProvider, SelectProvider};
use crate::error::ReactorError;
use crate::subscription::{
    QueryHash, Status, Subscriber, SubscriberSession, Subscription, SubscriptionPoll,
};

/// The scope name the process's state layer is linked under: session
/// facts a branch's readers see and nothing replicates or keeps.
pub const STATE_SCOPE: &str = "memory:state";

/// The scope name the branch itself is linked under.
pub const SHARED_SCOPE: &str = "memory:shared";

/// The attribute namespace a stack records its links under, on the
/// layer that links: `dialog.link/{from, to, name, revision, ...}`.
const LINK_ATTRIBUTES: &str = "dialog.link/";

fn scope(name: &str) -> Entity {
    name.parse().expect("a fixed scope name is a valid entity")
}

/// A branch as the reactor holds it: the branch, the stack every
/// read, subscription and write of it goes through, and the
/// subscriptions standing over it.
///
/// The stack is `[branch, state, top]`: `state` is this process's
/// ephemeral layer, linked under [`STATE_SCOPE`] by a wiring layer
/// above it, so facts placed on that scope — by the schema's `scope:`
/// declarations or by a writer through [`write`](Self::write) — land
/// there and never reach the tree. Commands dispatched through the
/// stack are witnessed on the layer their scope names, else the
/// branch's own session store; both are observed, and
/// [`drain_commands`](Self::drain_commands) hands back what fired.
pub struct BranchState {
    /// The branch itself; transactions and pulls address it through
    /// the stack.
    pub branch: Branch,
    state: Ephemeral,
    top: Ephemeral,
    stack: Stack,
    /// Instants on the state layer and on the branch's session store:
    /// where dispatched commands are witnessed.
    witnessed: [Observer; 2],
    /// The attributes this process has placed on the state scope of
    /// this branch, in the branch's session store, so a writer declares
    /// each attribute once.
    placed: Mutex<HashSet<Attribute>>,
    /// Subscriptions on this branch, keyed by query hash, in the order
    /// they were created. [`Self::poll`] walks them lowest level first (see
    /// [`Subscription::level`]), creation order breaking ties.
    subscriptions: Mutex<IndexMap<QueryHash, Subscription>>,
    /// Serializes *transactions* on this branch — concurrent writers (e.g.
    /// two browser tabs committing through one service worker) line up rather
    /// than racing the head CAS. Guards nothing but the right to be
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
    /// Open the branch's stack through the environment: a fresh state
    /// layer and wiring layer, linked above the branch.
    pub async fn open<Env>(branch: Branch, env: &Env) -> Result<Self, ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let state = Ephemeral::create().perform(env).await;
        let top = Ephemeral::create().perform(env).await;
        // A commit made through the branch handle rather than the stack
        // (the evaluate route's dialog transaction) still meets facts the
        // schema places on the state scope; bound to the branch's own
        // session store they land beside the tree, ephemeral, and the
        // stack's composite reads them with the branch. Through the stack
        // the scope resolves to the state layer instead.
        branch.bind(scope(STATE_SCOPE), StoreTarget::Session);
        let stack = Stack::open(top.clone())
            .link(&top, &state, scope(STATE_SCOPE))
            .link(&state, &branch, scope(SHARED_SCOPE))
            .perform(env)
            .await?;
        let witnessed = [
            state.observe_everything(),
            branch.overlay().observe_everything(),
        ];
        Ok(Self {
            branch,
            state,
            top,
            stack,
            witnessed,
            placed: Mutex::new(HashSet::new()),
            subscriptions: Mutex::new(IndexMap::new()),
            transactor: tokio::sync::Mutex::new(()),
        })
    }

    /// The stack every read and write of this branch goes through.
    pub fn stack(&self) -> &Stack {
        &self.stack
    }

    /// The process's state layer above the branch.
    pub fn state_layer(&self) -> &Ephemeral {
        &self.state
    }

    /// The wiring layer at the top of the stack, which a per-client
    /// stack links to reach the state layer.
    pub fn top_layer(&self) -> &Ephemeral {
        &self.top
    }

    /// The per-branch transaction lock. A transaction takes it
    /// (`transactor().lock().await`) around its commit so concurrent
    /// transactions serialize instead of failing the head CAS. Sync does not
    /// participate — see the field docs.
    pub fn transactor(&self) -> &tokio::sync::Mutex<()> {
        &self.transactor
    }

    /// Assert a [`Statement`] into the state layer: session facts every
    /// read of the branch sees, never committed, never replicated. The
    /// attributes the claim touches are placed on the state scope by
    /// the write itself, as this process's own declaration in the
    /// branch's session store, so the schema need not declare them and
    /// the tree never changes for it.
    ///
    /// A placement is a property of the attribute, on this branch, for
    /// every writer in this process: a concept written here must own its
    /// attributes. A session marker that carried a durable concept's
    /// attribute (a replica's `subject`, say) would place that attribute
    /// on the state scope and route the durable concept's facts there
    /// from then on.
    pub async fn write<S: Statement, Env>(&self, claim: S, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        self.apply(Vec::new(), claim, env).await
    }

    /// Forget `entities` in the state layer and assert `claim` there in
    /// one commit, so a reader never sees the layer between the two: a
    /// re-stamp that must replace an entity's facts rather than merge
    /// into them goes through here.
    pub async fn apply<S: Statement, Env>(
        &self,
        entities: Vec<Entity>,
        claim: S,
        env: &Env,
    ) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let mut changes = Changes::new();
        claim.assert(&mut changes);
        self.commit(entities, changes, env).await
    }

    /// Retract a [`Statement`] from the state layer.
    pub async fn erase<S: Statement, Env>(&self, claim: S, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let mut changes = Changes::new();
        claim.retract(&mut changes);
        self.commit(Vec::new(), changes, env).await
    }

    /// Drop every fact in the state layer. Used to keep a process's
    /// session facts from outliving the profile they were minted for.
    pub async fn clear<Env>(&self, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let _transacting = self.transactor.lock().await;
        if self.stack.behind() {
            self.stack.advance(env).await?;
        }
        self.stack
            .transaction()
            .clear(scope(STATE_SCOPE))
            .commit()
            .publish()
            .perform(env)
            .await?;
        Ok(())
    }

    /// Drop every state-layer fact recorded for `entities`: the
    /// garbage-collection primitive for per-client facts keyed by
    /// short-lived entities.
    pub async fn forget<Env>(&self, entities: Vec<Entity>, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        if entities.is_empty() {
            return Ok(());
        }
        self.commit(entities, Changes::new(), env).await
    }

    /// One stack commit: declare the attributes `changes` touches on the
    /// state scope where this process has not yet, then forget
    /// `entities` in the state layer and land `changes` there in one
    /// instant. The declarations go in the branch's own session store,
    /// this process's placements: the tree never holds them, no peer
    /// learns of them, and the commit touches no tree chain.
    async fn commit<Env>(
        &self,
        entities: Vec<Entity>,
        changes: Changes,
        env: &Env,
    ) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let _transacting = self.transactor.lock().await;
        // The stack routes at the heads it captured. A commit made outside
        // it since (the evaluate route, a seed install, a pull) may have
        // declared placements this write must honour: capture it first.
        if self.stack.behind() {
            self.stack.advance(env).await?;
        }
        self.place(&changes)?;
        let mut transaction = self.stack.transaction();
        if !entities.is_empty() {
            transaction = transaction.forget(scope(STATE_SCOPE), entities);
        }
        transaction
            .assert(changes)
            .commit()
            .publish()
            .perform(env)
            .await?;
        Ok(())
    }

    /// Declare on the state scope, in the branch's session store, every
    /// attribute `changes` touches that this process has not declared on
    /// this branch yet.
    fn place(&self, changes: &Changes) -> Result<(), ReactorError> {
        let mut placed = self.placed.lock();
        for (_, attribute, _) in changes.iter() {
            if placed.contains(attribute) {
                continue;
            }
            self.branch
                .overlay()
                .assert(Placement::new(attribute.clone(), scope(STATE_SCOPE)))
                .map_err(|error| ReactorError::Commit(error.into()))?;
            placed.insert(attribute.clone());
        }
        Ok(())
    }

    /// The state layer's facts, asserts and retracts alike: what a
    /// successor process restores to carry this one's session across.
    /// The stack's own wiring (`dialog.link/*`, which the state layer
    /// holds as the layer linking the branch) stays behind: a successor
    /// opens a stack of its own and must not inherit this one's links.
    pub fn export(&self) -> Changes {
        use dialog_artifacts::{Change, Update as _};
        let mut changes = Changes::new();
        for (entity, attribute, change) in self.state.export().iter() {
            if attribute.as_str().starts_with(LINK_ATTRIBUTES) {
                continue;
            }
            match change {
                Change::Assert(value) => {
                    changes.associate(attribute.clone(), entity.clone(), value.clone())
                }
                Change::Replace(value) => {
                    changes.associate_unique(attribute.clone(), entity.clone(), value.clone())
                }
                Change::Retract(value) => {
                    changes.dissociate(attribute.clone(), entity.clone(), value.clone())
                }
            }
        }
        changes
    }

    /// The commands witnessed since the last drain, as the facts they
    /// asserted: what a stack commit dispatched, including a command a
    /// rule concluded and the next round consumed. A gapped observer
    /// yields nothing for its store; a command it missed is lost, as a
    /// command is when its consumer is gone.
    pub fn drain_commands(&self) -> Changes {
        use dialog_artifacts::Update as _;
        let mut changes = Changes::new();
        for observer in &self.witnessed {
            let Drained::Instants(instants) = observer.drain() else {
                continue;
            };
            for instant in instants {
                if !instant.transient {
                    continue;
                }
                for fact in instant.asserted {
                    changes.associate(fact.the, fact.of, fact.is);
                }
            }
        }
        changes
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
        // The stack folds the state layer and the branch's session store
        // into every read, so the subscription sees ephemeral facts with no
        // extra wiring here.
        let plan = tonk_schema::concept::QueryPlan::from(query);
        let subscription = entry.or_insert_with(|| Subscription {
            engine: Arc::new(tokio::sync::Mutex::new(Some(
                self.stack.query().subscribe(plan),
            ))),
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

    /// Capture movement the stack has not seen: a commit made through
    /// the branch handle rather than the stack (the evaluate route's
    /// dialog transaction, a direct retract) moves the branch's head
    /// under a stack whose reads are pinned to the head it captured.
    /// Cheap when nothing moved; an advance otherwise. Never called
    /// under the transactor lock: it takes it.
    pub async fn settle<Env>(&self, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        if !self.stack.behind() {
            return Ok(());
        }
        let _transacting = self.transactor.lock().await;
        if self.stack.behind() {
            self.stack.advance(env).await?;
        }
        Ok(())
    }

    /// Re-poll every subscription on this branch. Mutating leaf
    /// effects call this on success so changed query results
    /// fan out to subscribers. Each subscription is polled via
    /// the same `SubscriptionPoll::perform` path the public
    /// chain uses. Movement made outside the stack is captured
    /// first, so a poll scheduled after a plain branch commit sees
    /// that commit.
    ///
    /// Lower levels go first. A display's level is how many displays
    /// enclose it, so it learns of a change before the displays nested
    /// inside it do: a parent that re-renders
    /// its children away first spares them a frame for an address it is
    /// replacing. Creation order breaks ties (a stable sort over the
    /// insertion-ordered map).
    pub async fn poll<'a, Env: SelectProvider + BranchOpenProvider>(
        self: &'a Arc<Self>,
        env: &'a Env,
    ) {
        if let Err(error) = self.settle(env).await {
            dialog_common::log!("reactor poll: advance over moved heads failed: {error}");
        }
        let hashes: Vec<QueryHash> = {
            let subs = self.subscriptions.lock();
            let mut ordered: Vec<(u32, QueryHash)> = subs
                .iter()
                .map(|(hash, subscription)| (subscription.level(), hash.clone()))
                .collect();
            ordered.sort_by_key(|(level, _)| *level);
            ordered.into_iter().map(|(_, hash)| hash).collect()
        };
        for hash in hashes {
            SubscriptionPoll { state: self, hash }.perform(env).await;
        }
    }
}
