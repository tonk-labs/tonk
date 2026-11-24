//! [`Connection`], [`Listener`] and [`Subscriber`]: one client's reader
//! of a stack subscription, the sender its frames go into, and the handle
//! the client holds.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use bytes::Bytes;
use dialog_common::log;
use dialog_query::Parameters;
use parking_lot::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError};

use super::{Plan, QueryHash};
use crate::{Conclusion, Frame, project};

/// The reader of a stack subscription one connection holds.
pub type Reader = dialog_repository::Subscriber<Plan>;

/// One client's reader of a stack subscription, and what it has been
/// sent: before its first frame, the whole result set is owed it as a
/// [`Frame::Snapshot`]; after it, each change as a [`Frame::Delta`].
///
/// Held strongly by the [`Subscriber`] the client streams from, and
/// weakly by its branch's [`Listener`], so the client's going is the
/// reader's: dialog's subscription lives while a reader of it does.
pub struct Connection {
    /// The dialog reader, or `None` while the branch it waits for does
    /// not exist yet (a pending subscription), until the branch is
    /// opened and the reader attached ([`Connection::attach`]).
    reader: Mutex<Option<Reader>>,
    /// The query's term bindings: a delivered row is a
    /// `ConceptConclusion`, projected to the wire `Conclusion` through
    /// them.
    terms: Parameters,
    /// Whether the client has been sent its snapshot.
    established: AtomicBool,
}

impl Connection {
    /// A connection reading through `reader`, for a query bound by
    /// `terms`, or waiting for a reader when there is none yet.
    pub(crate) fn new(reader: Option<Reader>, terms: Parameters) -> Arc<Self> {
        Arc::new(Self {
            reader: Mutex::new(reader),
            terms,
            established: AtomicBool::new(false),
        })
    }

    /// A connection waiting for its branch to exist, for a query bound by
    /// `terms`.
    pub(crate) fn waiting(terms: Parameters) -> Arc<Self> {
        Self::new(None, terms)
    }

    /// Attach the reader of a branch that came into existence after the
    /// client subscribed.
    pub(crate) fn attach(&self, reader: Reader) {
        *self.reader.lock() = Some(reader);
    }

    /// The identity of the subscription this connection reads, once it
    /// has a reader.
    pub fn hash(&self) -> Option<QueryHash> {
        self.reader
            .lock()
            .as_ref()
            .map(|reader| reader.hash().clone())
    }

    /// What dialog delivered to this connection's reader since it was
    /// last read, as wire frames: the first delivery is the whole result
    /// set and goes as a snapshot, a later one goes as a delta. A
    /// delivery that changed nothing is not sent.
    pub(crate) fn frames(&self) -> Vec<Bytes> {
        let deltas = match self.reader.lock().as_mut() {
            Some(reader) => reader.poll(),
            None => return Vec::new(),
        };
        let mut frames = Vec::new();
        for delta in deltas {
            let project_all = |rows: &[dialog_query::concept::descriptor::ConceptConclusion]| {
                rows.iter()
                    .map(|row| project(row, &self.terms))
                    .collect::<Vec<Conclusion>>()
            };
            let frame = if self.established.swap(true, Ordering::AcqRel) {
                if delta.is_empty() {
                    continue;
                }
                Frame::Delta {
                    asserted: project_all(&delta.asserted),
                    retracted: project_all(&delta.retracted),
                }
            } else {
                Frame::Snapshot {
                    conclusions: project_all(&delta.asserted),
                }
            };
            match serde_json::to_vec(&frame) {
                Ok(bytes) => frames.push(Bytes::from(bytes)),
                Err(error) => log!("[reactor] failed to serialize a frame: {error}"),
            }
        }
        frames
    }
}

/// A branch's record of one connection: where its frames go, whose it
/// is, and how deeply its display is nested. Holds the connection weakly:
/// once the client has dropped its [`Subscriber`], the listener is
/// pruned at the next delivery.
pub(crate) struct Listener {
    pub(crate) connection: Weak<Connection>,
    pub(crate) sender: UnboundedSender<Bytes>,
    /// The client (e.g. a service-worker client id) the connection
    /// serves, when known, so the owner can drop the connections of a
    /// client that is gone: a client that vanishes without cancelling
    /// its response stream never drops its subscriber itself.
    pub(crate) client: Option<String>,
    /// How deeply the connection's consumer is nested: 0 is outermost.
    /// A branch delivers to lower levels first.
    pub(crate) level: u32,
}

impl Listener {
    /// Send the connection the frames dialog delivered it. `false` when
    /// the connection is gone or its client stopped listening, and the
    /// listener should be dropped.
    pub(crate) fn deliver(&self) -> bool {
        let Some(connection) = self.connection.upgrade() else {
            return false;
        };
        if self.sender.is_closed() {
            return false;
        }
        connection
            .frames()
            .into_iter()
            .all(|frame| self.sender.send(frame).is_ok())
    }
}

/// What a client subscribing gets: the frames of its standing query, and
/// the [`Connection`] that reads them, which it owns.
///
/// Dropping the subscriber, which is what a page closing its stream
/// does, drops its dialog reader, and the stack's subscription goes with
/// its last reader. A subscriber whose branch drops its listener (the
/// worker retiring, the space evicted, the client gone) reads the end of
/// its frames.
pub struct Subscriber {
    receiver: UnboundedReceiver<Bytes>,
    connection: Arc<Connection>,
}

impl Subscriber {
    /// A subscriber reading `receiver`, which `connection`'s frames are
    /// sent into.
    pub(crate) fn new(receiver: UnboundedReceiver<Bytes>, connection: Arc<Connection>) -> Self {
        Self {
            receiver,
            connection,
        }
    }

    /// The subscription's identity on its branch, once it has a reader
    /// (a subscriber waiting for its branch has none yet).
    pub fn hash(&self) -> Option<QueryHash> {
        self.connection.hash()
    }

    /// The next frame, waiting for one; `None` once the branch has
    /// dropped this connection.
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.receiver.recv().await
    }

    /// The next frame if one has been sent, without waiting.
    pub fn try_recv(&mut self) -> Result<Bytes, TryRecvError> {
        self.receiver.try_recv()
    }

    /// The frames as a stream, which owns the connection: the stream's
    /// end, or its being dropped, is the connection's. The stream is
    /// `Send` on every target, as a response body must be: on wasm, where
    /// the dialog reader is not, the connection is held in a
    /// [`SendWrapper`](send_wrapper::SendWrapper), which the
    /// single-threaded runtime never moves across threads.
    pub fn into_stream(self) -> Frames {
        Frames {
            receiver: self.receiver,
            _connection: Held::new(self.connection),
        }
    }
}

/// The connection a [`Frames`] stream keeps alive.
#[cfg(not(target_arch = "wasm32"))]
type Held = HeldConnection;
/// The connection a [`Frames`] stream keeps alive.
#[cfg(target_arch = "wasm32")]
type Held = send_wrapper::SendWrapper<Arc<Connection>>;

/// The connection itself, natively, where it is `Send`.
#[cfg(not(target_arch = "wasm32"))]
struct HeldConnection(#[allow(dead_code)] Arc<Connection>);

#[cfg(not(target_arch = "wasm32"))]
impl HeldConnection {
    fn new(connection: Arc<Connection>) -> Self {
        Self(connection)
    }
}

/// A subscriber's frames as a stream ([`Subscriber::into_stream`]): it
/// owns the connection, so dropping the stream drops the dialog reader.
pub struct Frames {
    receiver: UnboundedReceiver<Bytes>,
    _connection: Held,
}

impl futures_util::Stream for Frames {
    type Item = Bytes;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Bytes>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}
