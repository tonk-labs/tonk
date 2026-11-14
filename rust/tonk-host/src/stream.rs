//! Shared subscription streams.
//!
//! Consumers in one document often ask the same question. Every row of a
//! list mounts a `<tonk-display>` that subscribes to its concept's model,
//! name and view, so a 200-row list opens the same three queries 200
//! times. Each open costs a fetch (relayed out of a sealed guest over its
//! port), a stream the worker evaluates and polls after every commit, and
//! a frame per commit posted back. Measured on a 200-note space, two in
//! three subscriptions a warm reload opened were exact duplicates.
//!
//! A [`Stream`] carries one upstream subscription for every registry entry
//! with the same URL and body. Each entry still owns its consumer, tag,
//! route and retry: the stream only multiplexes the transport. It keeps
//! the frames since the last snapshot so a consumer that joins late is
//! handed the same sequence an early one saw, applied by its own logic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::registry::EntryId;
use crate::sse::EventSource;

/// How many frames past a snapshot a stream keeps for late joiners. A
/// stream that has carried more deltas than this since its last snapshot
/// stops taking joiners, and a later identical subscription opens its own.
const REPLAY_LIMIT: usize = 64;

/// One upstream subscription shared by every entry that asked for it.
///
/// Entries hold it by `Rc`; the last entry to let go drops the upstream
/// transport with it. The open, frame, error and close callbacks hold it
/// weakly, so a stream nobody holds is never kept alive by its own reader.
pub(crate) struct Stream {
    inner: RefCell<Inner>,
}

struct Inner {
    /// Upstream transport, installed once the open resolves.
    source: Option<EventSource>,
    /// Every open that rides this stream. Entries refreshed or removed
    /// since are pruned the next time a frame finds them stale.
    subscribers: Vec<Subscriber>,
    /// The frames since (and including) the last snapshot, as received.
    frames: Vec<String>,
    /// Whether a new identical subscription may join. False once the
    /// stream ended or carried more than [`REPLAY_LIMIT`] frames.
    joinable: bool,
}

/// One entry's open of a shared stream.
#[derive(Clone, Copy)]
pub(crate) struct Subscriber {
    /// The registry entry the frames go to.
    pub entry: EntryId,
    /// The entry's open attempt this subscriber belongs to.
    pub attempt: u64,
    /// Whether the next frame this entry receives is its first after a
    /// reconnect.
    reconnect: bool,
    /// False until the entry has been handed the frames the stream carried
    /// before it joined. Live frames skip it until then; they are kept and
    /// reach it with the replay.
    ready: bool,
}

/// A frame to route to one subscriber.
pub(crate) struct Delivery {
    pub entry: EntryId,
    pub attempt: u64,
    pub reconnect: bool,
}

impl Stream {
    /// A stream with `entry` as its first, ready subscriber.
    pub(crate) fn open(entry: EntryId, attempt: u64, reconnect: bool) -> Rc<Self> {
        Rc::new(Self {
            inner: RefCell::new(Inner {
                source: None,
                subscribers: vec![Subscriber {
                    entry,
                    attempt,
                    reconnect,
                    ready: true,
                }],
                frames: Vec::new(),
                joinable: true,
            }),
        })
    }

    /// Add `entry` as a subscriber. It receives nothing until
    /// [`Stream::replay`] hands it what the stream has carried so far.
    pub(crate) fn join(&self, entry: EntryId, attempt: u64, reconnect: bool) {
        self.inner.borrow_mut().subscribers.push(Subscriber {
            entry,
            attempt,
            reconnect,
            ready: false,
        });
    }

    /// The frames `entry` missed, marking it ready so later frames reach
    /// it live. Empty when the entry is no longer subscribed.
    pub(crate) fn replay(&self, entry: EntryId, attempt: u64) -> Vec<(String, bool)> {
        let mut inner = self.inner.borrow_mut();
        let Inner {
            subscribers,
            frames,
            joinable,
            ..
        } = &mut *inner;
        let Some(subscriber) = subscribers
            .iter_mut()
            .find(|s| s.entry == entry && s.attempt == attempt && !s.ready)
        else {
            return Vec::new();
        };
        subscriber.ready = true;
        let replay = frames
            .iter()
            .map(|frame| (frame.clone(), std::mem::take(&mut subscriber.reconnect)))
            .collect();
        if !*joinable && subscribers.iter().all(|s| s.ready) {
            frames.clear();
        }
        replay
    }

    /// Install the upstream transport once its open resolves.
    pub(crate) fn install(&self, source: EventSource) {
        self.inner.borrow_mut().source = Some(source);
    }

    /// Record `frame` and return who it goes to now: every ready
    /// subscriber, consuming each one's reconnect marker.
    pub(crate) fn record(&self, frame: &str) -> Vec<Delivery> {
        let mut inner = self.inner.borrow_mut();
        if is_snapshot(frame) {
            inner.frames.clear();
        }
        let waiting = inner.subscribers.iter().any(|s| !s.ready);
        if inner.joinable || waiting {
            inner.frames.push(frame.to_owned());
        }
        if inner.frames.len() > REPLAY_LIMIT {
            inner.joinable = false;
            if !waiting {
                inner.frames.clear();
            }
        }
        inner
            .subscribers
            .iter_mut()
            .filter(|s| s.ready)
            .map(|s| Delivery {
                entry: s.entry,
                attempt: s.attempt,
                reconnect: std::mem::take(&mut s.reconnect),
            })
            .collect()
    }

    /// Drop subscribers whose entry no longer rides this stream.
    pub(crate) fn forget(&self, stale: &[(EntryId, u64)]) {
        if stale.is_empty() {
            return;
        }
        self.inner
            .borrow_mut()
            .subscribers
            .retain(|s| !stale.contains(&(s.entry, s.attempt)));
    }

    /// End the stream: no one joins it again. Returns every subscriber,
    /// whose entries each retry on their own.
    pub(crate) fn end(&self) -> Vec<Subscriber> {
        let mut inner = self.inner.borrow_mut();
        inner.joinable = false;
        inner.frames.clear();
        inner.subscribers.clone()
    }

    fn joinable(&self) -> bool {
        self.inner.borrow().joinable
    }
}

/// Whether `frame` replaces everything before it: a `snapshot` frame, or a
/// bare array from the pre-delta wire. The worker serializes the tag
/// first, so the prefix decides without parsing the rows.
fn is_snapshot(frame: &str) -> bool {
    let frame = frame.trim_start();
    frame.starts_with('[') || frame.starts_with(r#"{"kind":"snapshot""#)
}

/// The document's live streams, by URL and body.
#[derive(Default)]
pub(crate) struct Streams {
    by_key: HashMap<String, Weak<Stream>>,
}

impl Streams {
    /// The live, joinable stream for `key`, if there is one.
    pub(crate) fn joinable(&self, key: &str) -> Option<Rc<Stream>> {
        self.by_key
            .get(key)
            .and_then(Weak::upgrade)
            .filter(|stream| stream.joinable())
    }

    /// Make `stream` the one later subscriptions for `key` join, and drop
    /// keys whose streams are gone.
    pub(crate) fn insert(&mut self, key: String, stream: &Rc<Stream>) {
        self.by_key.retain(|_, weak| weak.strong_count() > 0);
        self.by_key.insert(key, Rc::downgrade(stream));
    }

    /// Stop offering `stream` under `key`, if it is still the one offered.
    pub(crate) fn remove(&mut self, key: &str, stream: &Stream) {
        if self
            .by_key
            .get(key)
            .is_some_and(|weak| std::ptr::eq(weak.as_ptr(), stream))
        {
            self.by_key.remove(key);
        }
    }

    /// How many streams are open. Test-only introspection.
    #[cfg(test)]
    pub(crate) fn live(&self) -> usize {
        self.by_key
            .values()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}
