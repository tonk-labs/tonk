//! [`Subscription`] — one per `(branch, query)` pair, shared by
//! every subscriber that opened that query against that branch.

use bytes::Bytes;
use dialog_common::Blake3Hash;
use dialog_query::ConceptQuery;
use tokio::sync::mpsc;

use super::wire::WireQuery;

/// Identity of a subscription within one branch — blake3 over a
/// deterministic serialization of the [`ConceptQuery`]. Wraps
/// [`Blake3Hash`] so two queries that hash to the same digest
/// share the same subscription.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct QueryHash(Blake3Hash);

impl QueryHash {
    pub(crate) fn of(query: &ConceptQuery) -> Self {
        // Serde-json over the WireQuery projection is a
        // deterministic function of `query` *within one Rust
        // process* — sufficient for use as a hash input. Map
        // ordering is the only concern; `Parameters` and
        // `NamedAttributes` both emit keys in `BTreeMap` order
        // via their custom serializers.
        let wire = WireQuery::from(query);
        let bytes = serde_json::to_vec(&wire)
            .expect("WireQuery is serializable for any valid ConceptQuery");
        Self(Blake3Hash::hash(&bytes))
    }
}

/// Per-subscriber state — has the subscriber received the
/// subscription's current `last_hash` yet?
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Status {
    /// Just attached — hasn't received the current snapshot yet.
    Pending,
    /// Has received bytes whose hash matches the subscription's
    /// `last_hash`.
    Established,
}

/// One downstream listener attached to a [`Subscription`].
pub(super) struct Subscriber {
    pub sender: mpsc::UnboundedSender<Bytes>,
    pub status: Status,
}

/// One subscription, shared by every subscriber that opened the
/// same query against the same branch. The branch handle isn't
/// carried here — the poll path already has access via the
/// parent `BranchEntry`.
pub(super) struct Subscription {
    /// The query to re-run on every poll.
    pub query: ConceptQuery,
    /// Hash of the most recent serialization of the result.
    /// `None` until the first poll completes.
    pub last_hash: Option<Blake3Hash>,
    /// Open downstream channels with their delivery status.
    pub subscribers: Vec<Subscriber>,
}
