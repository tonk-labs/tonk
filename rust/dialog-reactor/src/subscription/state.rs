//! [`Subscription`] — one per `(branch, query)` pair, shared by
//! every subscriber that opened that query against that branch.

use bytes::Bytes;
use dialog_common::Blake3Hash;
use dialog_query::ConceptQuery;
use tokio::sync::mpsc::UnboundedSender;

use crate::Query;

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
/// subscription's current `last_hash` yet?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Just attached — hasn't received the current snapshot yet.
    Pending,
    /// Has received bytes whose hash matches the subscription's
    /// `last_hash`.
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
    /// Whether the subscriber has received the current
    /// `last_hash` yet.
    pub status: Status,
}

/// One subscription, shared by every subscriber that opened the
/// same query against the same branch. The branch handle isn't
/// carried here — the poll path already has access via the
/// parent `BranchState`.
pub struct Subscription {
    /// The query to re-run on every poll.
    pub query: ConceptQuery,
    /// Hash of the most recent serialization of the result.
    /// `None` until the first poll completes.
    pub last_hash: Option<Blake3Hash>,
    /// Open downstream channels with their delivery status.
    pub subscribers: Vec<SubscriberSession>,
}
