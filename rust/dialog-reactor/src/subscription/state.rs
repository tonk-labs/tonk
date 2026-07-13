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

use crate::{Conclusion, Frame, Query};
use bytes::Bytes;
use dialog_common::log;

/// The dialog engine backing one reactor subscription.
///
/// The subscribed application is a [`QueryPlan`](tonk_schema::concept::QueryPlan),
/// NOT a raw `ConceptQuery`: `QueryPlan::from` dispatches concept-of-concept /
/// command / rule metadata queries to the anonymous-enumeration applications that
/// surface those built-in rows. Subscribing with the raw `ConceptQuery` would run
/// a metadata query as a plain branch scan over `dialog.meta/*` facts that don't
/// exist as stored data, yielding an empty result (the "Model not found" on a
/// `<tonk-display model=…>` whose one-shot query — which does route through
/// `QueryPlan` — resolves fine).
pub type Engine = dialog_repository::Subscription<tonk_schema::concept::QueryPlan>;

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
    /// The client (e.g. a service-worker client id) this subscriber
    /// serves, when known. Lets the owner reconcile subscribers
    /// against its live-client set and drop the dead ones — channel
    /// closure alone can't be relied on: a client that vanishes
    /// without cancelling its response stream leaves the receiver
    /// alive, so the send-failure prune never fires.
    pub client: Option<String>,
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
    /// [`project`](tonk_schema::conclusion::project)
    /// — the delta / snapshot rows are `ConceptConclusion`s that
    /// project to the wire `Conclusion` through these terms.
    pub terms: Parameters,
    /// Open downstream channels with their delivery status.
    pub subscribers: Vec<SubscriberSession>,
}

impl Subscription {
    /// Fan a poll's result out to every subscriber, advancing each to
    /// [`Established`](Status::Established) once served.
    ///
    /// - A [`Pending`](Status::Pending) subscriber gets a full
    ///   [`Frame::Snapshot`] built from `snapshot_conclusions` (the
    ///   engine's retained results, projected to wire rows). This is the
    ///   race-safe half: the Pending set is read HERE, at delivery, not
    ///   from a flag captured before the poll's `await` — so a subscriber
    ///   that attached while the poll ran is still served its first frame
    ///   now rather than left hung on `loading` until an unrelated write
    ///   happens to schedule another poll (which, on a quiescent branch,
    ///   may never come).
    /// - An [`Established`](Status::Established) subscriber gets
    ///   `delta_bytes` when the poll reported a non-empty change, and
    ///   nothing otherwise.
    ///
    /// The snapshot is serialized lazily and at most once, only when a
    /// Pending subscriber is actually present. A subscriber whose channel
    /// has closed is dropped.
    pub fn deliver(&mut self, snapshot_conclusions: &[Conclusion], delta_bytes: Option<&Bytes>) {
        fan_out(&mut self.subscribers, snapshot_conclusions, delta_bytes);
    }
}

/// Deliver a poll's result to a subscriber set (see
/// [`Subscription::deliver`]). Split out from the `Subscription` method so
/// the delivery logic — the race-sensitive part — is exercised directly by
/// tests without fabricating a dialog engine.
fn fan_out(
    subscribers: &mut Vec<SubscriberSession>,
    snapshot_conclusions: &[Conclusion],
    delta_bytes: Option<&Bytes>,
) {
    // Memoized snapshot bytes: `None` = not built yet, `Some(inner)` =
    // built (`inner` may itself be `None` if serialization failed).
    let mut snapshot_bytes: Option<Option<Bytes>> = None;

    subscribers.retain_mut(|subscriber| {
        let bytes = match subscriber.status {
            Status::Pending => snapshot_bytes
                .get_or_insert_with(|| serialize_snapshot(snapshot_conclusions.to_vec()))
                .clone(),
            Status::Established => delta_bytes.cloned(),
        };
        let Some(bytes) = bytes else {
            // Nothing to send this subscriber this round (an Established
            // subscriber on a no-change poll, or a frame that failed to
            // serialize) — keep it.
            return true;
        };
        match subscriber.sender.send(bytes) {
            Ok(()) => {
                subscriber.status = Status::Established;
                true
            }
            Err(_) => false,
        }
    });
}

/// Serialize a [`Frame::Snapshot`] to wire bytes, logging and dropping on
/// failure (a serialization error is not worth killing the fan-out).
fn serialize_snapshot(conclusions: Vec<Conclusion>) -> Option<Bytes> {
    match serde_json::to_vec(&Frame::Snapshot { conclusions }) {
        Ok(bytes) => Some(Bytes::from(bytes)),
        Err(err) => {
            log!("[reactor] failed to serialize snapshot frame: {err}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(missing_docs)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    use std::collections::BTreeMap;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;

    /// One row of a snapshot.
    fn conclusion(this: &str) -> Conclusion {
        Conclusion {
            this: this.to_owned(),
            fields: BTreeMap::new(),
        }
    }

    /// A subscriber in the given delivery state, paired with its receiver.
    fn subscriber(status: Status) -> (SubscriberSession, UnboundedReceiver<Bytes>) {
        let (sender, receiver) = unbounded_channel();
        (
            SubscriberSession {
                sender,
                status,
                client: None,
            },
            receiver,
        )
    }

    /// Decode a delivered frame off a receiver.
    fn recv(receiver: &mut UnboundedReceiver<Bytes>) -> Option<Frame> {
        receiver
            .try_recv()
            .ok()
            .map(|bytes| serde_json::from_slice(&bytes).expect("frame decodes"))
    }

    /// A Pending subscriber must receive its snapshot on the next fan-out —
    /// even when it is the ONLY pending subscriber and every other is
    /// Established. The regression: the snapshot used to be built off a flag
    /// captured before the engine poll, so a subscriber that attached during
    /// the poll (or after that flag was read) was silently kept unserved and
    /// its `<tonk-display>` hung on `loading` forever. Delivery now decides
    /// per-subscriber at fan-out, so this can't happen.
    #[dialog_common::test]
    fn it_snapshots_a_pending_subscriber_even_beside_established_ones() {
        let (established, mut established_rx) = subscriber(Status::Established);
        let (pending, mut pending_rx) = subscriber(Status::Pending);
        let mut subscribers = vec![established, pending];

        // A no-change poll: no delta for the Established subscriber, but the
        // Pending one still needs its first snapshot.
        fan_out(&mut subscribers, &[conclusion("id:one")], None);

        // The Pending subscriber got a Snapshot and is now Established.
        match recv(&mut pending_rx) {
            Some(Frame::Snapshot { conclusions }) => {
                assert_eq!(conclusions, vec![conclusion("id:one")]);
            }
            other => panic!("pending subscriber must get a Snapshot, got {other:?}"),
        }
        assert_eq!(subscribers[1].status, Status::Established);

        // The Established subscriber got nothing on a no-change poll.
        assert!(
            recv(&mut established_rx).is_none(),
            "established subscriber gets no frame when the delta is empty"
        );
    }

    /// The snapshot is serialized at most once regardless of how many
    /// Pending subscribers there are, and every one of them receives it.
    #[dialog_common::test]
    fn it_serves_every_pending_subscriber_one_snapshot() {
        let (a, mut a_rx) = subscriber(Status::Pending);
        let (b, mut b_rx) = subscriber(Status::Pending);
        let mut subscribers = vec![a, b];

        fan_out(&mut subscribers, &[conclusion("id:x")], None);

        for rx in [&mut a_rx, &mut b_rx] {
            assert!(
                matches!(recv(rx), Some(Frame::Snapshot { .. })),
                "each pending subscriber receives the snapshot"
            );
        }
        assert!(subscribers.iter().all(|s| s.status == Status::Established));
    }

    /// An Established subscriber receives a non-empty delta; a Pending one
    /// added in the same round still gets a full snapshot, not the delta.
    #[dialog_common::test]
    fn it_sends_deltas_to_established_and_snapshots_to_pending() {
        let (established, mut established_rx) = subscriber(Status::Established);
        let (pending, mut pending_rx) = subscriber(Status::Pending);
        let mut subscribers = vec![established, pending];

        let delta = Bytes::from(
            serde_json::to_vec(&Frame::Delta {
                asserted: vec![],
                retracted: vec![],
            })
            .unwrap(),
        );
        fan_out(&mut subscribers, &[conclusion("id:one")], Some(&delta));

        assert!(
            matches!(recv(&mut established_rx), Some(Frame::Delta { .. })),
            "established subscriber gets the delta"
        );
        assert!(
            matches!(recv(&mut pending_rx), Some(Frame::Snapshot { .. })),
            "pending subscriber gets a snapshot, not the delta"
        );
    }

    /// A subscriber whose receiver has been dropped is pruned from the set.
    #[dialog_common::test]
    fn it_drops_a_subscriber_whose_channel_closed() {
        let (live, mut live_rx) = subscriber(Status::Pending);
        let (dead, dead_rx) = subscriber(Status::Pending);
        drop(dead_rx);
        let mut subscribers = vec![live, dead];

        fan_out(&mut subscribers, &[conclusion("id:one")], None);

        assert_eq!(subscribers.len(), 1, "the closed subscriber is pruned");
        assert!(matches!(recv(&mut live_rx), Some(Frame::Snapshot { .. })));
    }
}
