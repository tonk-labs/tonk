//! [`BranchState`] — cached branch handle plus the subscriptions
//! registered against it. Lives behind an `Arc`; subscription
//! operations on the branch happen directly on the state without
//! routing through the reactor's name-keyed lookup.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use dialog_artifacts::Statement;
use dialog_artifacts::{Changes, Entity};
use dialog_query::ConceptQuery;
use dialog_repository::{Branch, Drained, Ephemeral, Observer, Stack};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::env::{BranchOpenProvider, SelectProvider};
use crate::error::ReactorError;
use crate::subscription::{
    QueryHash, Status, Subscriber, SubscriberSession, Subscription, SubscriptionPoll,
};

/// Cached branch handle plus the subscriptions registered
/// against it. Held inside the reactor's cache as
/// `Arc<BranchState>` so callers can hand the state around
/// without re-locking the reactor's outer map.
/// The scope name the process's state layer is linked under: session
/// facts a branch's readers see and nothing replicates or keeps.
pub const STATE_SCOPE: &str = "memory:state";

/// The scope name the branch itself is linked under.
pub const SHARED_SCOPE: &str = "memory:shared";

fn scope(name: &str) -> Entity {
    name.parse().expect("a fixed scope name is a valid entity")
}

/// A branch as the reactor holds it: the branch, the stack every
/// read, subscription, and write of it goes through, and the
/// subscriptions standing over it.
///
/// The stack is `[branch, state, top]`: `state` is this process's
/// ephemeral layer, linked under [`STATE_SCOPE`] by a wiring layer
/// above it, so session facts placed on that scope — by the schema or
/// explicitly by [`write`](Self::write) — land there and never reach
/// the tree. Commands dispatched through the stack are witnessed on
/// the layer their scope names, else the branch's own session store;
/// both are observed, and [`drain_commands`](Self::drain_commands)
/// hands back what fired.
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
    subscriptions: Mutex<HashMap<QueryHash, Subscription>>,
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
        branch.bind(scope(STATE_SCOPE), dialog_repository::Target::Session);
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
            subscriptions: Mutex::new(HashMap::new()),
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

    /// Serializes commits on this branch. Held across the whole
    /// commit so a concurrent transaction never builds on a head this
    /// one is about to move.
    pub fn transactor(&self) -> &tokio::sync::Mutex<()> {
        &self.transactor
    }

    /// Assert a [`Statement`] into the state layer: session facts every
    /// read of the branch sees, never committed, never replicated.
    /// Placed explicitly, so the schema need not declare the attributes.
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
        let _transacting = self.transactor.lock().await;
        let mut transaction = self.stack.transaction();
        if !entities.is_empty() {
            transaction = transaction.forget(scope(STATE_SCOPE), entities);
        }
        transaction
            .assert_into(scope(STATE_SCOPE), claim)
            .commit()
            .publish()
            .perform(env)
            .await?;
        Ok(())
    }

    /// Retract a [`Statement`] from the state layer.
    pub async fn erase<S: Statement, Env>(&self, claim: S, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let _transacting = self.transactor.lock().await;
        self.stack
            .transaction()
            .retract_from(scope(STATE_SCOPE), claim)
            .commit()
            .publish()
            .perform(env)
            .await?;
        Ok(())
    }

    /// Drop every fact in the state layer. Used to keep a process's
    /// session facts from outliving the profile they were minted for.
    pub async fn clear<Env>(&self, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let _transacting = self.transactor.lock().await;
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
        let _transacting = self.transactor.lock().await;
        if entities.is_empty() {
            return Ok(());
        }
        self.stack
            .transaction()
            .forget(scope(STATE_SCOPE), entities)
            .commit()
            .publish()
            .perform(env)
            .await?;
        Ok(())
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

    pub fn subscriptions(&self) -> &Mutex<HashMap<QueryHash, Subscription>> {
        &self.subscriptions
    }

    /// Drop every subscription's subscribers; used on shutdown so a
    /// late poll finds nothing to deliver to.
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
        sender: mpsc::UnboundedSender<Bytes>,
    ) -> QueryHash {
        let hash = QueryHash::from(&query);
        self.install_subscriber(query, client, sender);
        hash
    }

    pub fn subscribe(
        &self,
        query: ConceptQuery,
        client: Option<String>,
    ) -> Result<Subscriber, ReactorError> {
        let hash = QueryHash::from(&query);
        let (sender, receiver) = mpsc::unbounded_channel();
        self.install_subscriber(query, client, sender);
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
    /// Cheap when nothing moved; an advance otherwise.
    pub async fn settle<Env>(&self, env: &Env) -> Result<(), ReactorError>
    where
        Env: BranchOpenProvider,
    {
        let _transacting = self.transactor.lock().await;
        if self.stack.behind() {
            self.stack.advance(env).await?;
        }
        Ok(())
    }

    /// Re-evaluate every subscription over the stack's current heads:
    /// movement made outside the stack is captured first, so a poll
    /// scheduled after a plain branch commit sees that commit.
    pub async fn poll<'a, Env: SelectProvider + BranchOpenProvider>(
        self: &'a Arc<Self>,
        env: &'a Env,
    ) {
        if let Err(error) = self.settle(env).await {
            dialog_common::log!("reactor poll: advance over moved heads failed: {error}");
        }
        let hashes: Vec<QueryHash> = {
            let subs = self.subscriptions.lock();
            subs.keys().cloned().collect()
        };
        for hash in hashes {
            SubscriptionPoll { state: self, hash }.perform(env).await;
        }
    }
}
