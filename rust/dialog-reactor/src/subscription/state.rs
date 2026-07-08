//! [`Subscription`] — one per `(branch, query)` pair, shared by
//! every subscriber that opened that query against that branch.
//!
//! The evaluation engine is dialog's demand-gated
//! [`dialog_repository::Subscription`]: each poll returns `None`
//! when nothing inside the query's demand cover changed (no query
//! work), or a [`Delta`](dialog_repository::Delta) of asserted /
//! retracted rows after an incremental maintenance or a recompute.
//! This crate wraps that engine with the fan-out to SSE subscribers.

use std::sync::Arc;

use dialog_common::Blake3Hash;
use dialog_query::ConceptQuery;
use dialog_query::Parameters;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::UnboundedSender;

use crate::Query;
use bytes::Bytes;

/// The dialog engine backing one reactor subscription.
pub type Engine = dialog_repository::Subscription<ConceptQuery>;

/// The engine slot: an `Option` behind an async mutex.
///
/// The poll takes the engine *out* (leaving `None`), releases the
/// lock, awaits [`poll`](dialog_repository::Subscription::poll), then
/// puts it back. It is deliberately **not** held as a guard across
/// the engine's `await`: a `tokio::sync::MutexGuard` borrowed across
/// the generic, higher-ranked `poll(env).await` collapses rustc's
/// Send-generality inference and makes every downstream axum handler
/// future non-`Send` on the native build. Taking the value out sheds
/// the guard before the await, so the future stays `Send`-general.
///
/// The lock still serializes take/put, so two polls of the same
/// subscription can't both hold the engine. A poll that finds the
/// slot already emptied (a concurrent poll has it) simply returns —
/// the in-flight poll covers the same or a newer revision, and poll
/// scheduling coalesces, so nothing is lost.
pub type EngineSlot = Arc<AsyncMutex<Option<Engine>>>;

/// Identity of a subscription within one branch — blake3 over a
/// deterministic serialization of the [`ConceptQuery`]. Wraps
/// [`Blake3Hash`] so two queries that hash to the same digest
/// share the same subscription.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QueryHash(Blake3Hash);

impl From<&ConceptQuery> for QueryHash {
    /// Hash a [`ConceptQuery`] into a [`QueryHash`] for use as
    /// a subscription identity within one branch.
    ///
    /// Serde-json over the wire [`Query`] projection is a
    /// deterministic function of `query` *within one Rust
    /// process* — sufficient for use as a hash input. Map
    /// ordering is the only concern; `Parameters` and
    /// `NamedAttributes` both emit keys in `BTreeMap` order via
    /// their custom serializers.
    fn from(query: &ConceptQuery) -> Self {
        let wire = Query::from(query);
        let bytes = serde_json::to_vec(&wire)
            .expect("wire Query is serializable for any valid ConceptQuery");
        Self(Blake3Hash::hash(&bytes))
    }
}

/// Per-subscriber state — has the subscriber received the
/// subscription's current snapshot yet?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Just attached — hasn't received a snapshot yet. The next
    /// poll sends it a full [`Frame::Snapshot`](crate::Frame),
    /// not a delta, so it needs no prior retained state.
    Pending,
    /// Has received the initial snapshot; subsequent polls send
    /// deltas.
    Established,
}

/// One subscriber's session against a [`Subscription`]. Holds
/// the sender side of the subscriber's channel plus the
/// per-subscriber delivery status. Internal-only — the
/// public-facing handle a subscriber receives is the
/// [`crate::Subscriber`] returned by
/// `BranchState::subscribe`.
pub struct SubscriberSession {
    /// Sender into the subscriber's mpsc channel.
    pub sender: UnboundedSender<Bytes>,
    /// Whether the subscriber has received its initial snapshot.
    pub status: Status,
}

/// One subscription, shared by every subscriber that opened the
/// same query against the same branch.
///
/// The [`Engine`] (dialog's incremental subscription) sits in its own
/// [`EngineSlot`] behind an async mutex, not under the branch's
/// subscription-map lock: [`poll`](dialog_repository::Subscription::poll)
/// takes `&mut self` and awaits, so the map's synchronous `parking_lot`
/// lock must be released before the poll. The poll path clones the
/// `EngineSlot` handle out under the map lock, takes the engine out of
/// the slot to run it, then re-locks the map only to fan bytes out to
/// subscribers.
pub struct Subscription {
    /// Dialog's demand-gated evaluation engine, in a take-out slot so
    /// the poll runs it without a guard held across the `await` (see
    /// [`EngineSlot`]).
    pub engine: EngineSlot,
    /// The query's term bindings, retained for
    /// [`Conclusion::project`](tonk_schema::conclusion::Conclusion::project)
    /// — the delta / snapshot rows are `ConceptConclusion`s that
    /// project to the wire `Conclusion` through these terms.
    pub terms: Parameters,
    /// The branch overlay epoch this subscription's engine was last
    /// seeded at. When the branch's epoch moves ahead (an overlay
    /// write), the poll re-seeds the engine with the current overlay
    /// and forces a recompute — an overlay change is off-tree, so the
    /// engine's own tree-diff gate can't observe it.
    pub seeded_overlay_epoch: u64,
    /// Open downstream channels with their delivery status.
    pub subscribers: Vec<SubscriberSession>,
}
