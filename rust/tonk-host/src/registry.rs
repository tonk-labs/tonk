//! Subscription registry — the host's record of live consumer
//! subscriptions.
//!
//! Each entry holds enough state to refresh the subscription
//! against a new `(space, branch)` context: the consumer
//! element, the query body, the optional tag, and the structural
//! depth. The abort handle is held so the entry's `Drop` cancels
//! the upstream.
//!
//! Keyed by an opaque `EntryId`. v1 uses linear scans for both
//! consumer lookup and context-refresh filtering — fine for the
//! handful of subscriptions a page has. Future work: index by
//! consumer element identity for O(1) unsubscribe, and ref-count
//! entries sharing the same upstream subscription for dedup.

use std::collections::BTreeMap;

use crate::sse::EventSource;
use wasm_bindgen::JsValue;
use web_sys::Element;

/// Opaque handle identifying one registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct EntryId(u64);

/// One live consumer subscription.
pub(crate) struct Entry {
    /// The consumer element. Calls to `reset` / `update` /
    /// `error` go to this element.
    pub consumer: Element,
    /// Space context at subscribe time. Re-read from the
    /// nearest `<tonk-repository>` ancestor on refresh, but the
    /// stored value lets the host abort early if context hasn't
    /// actually changed.
    pub space: Option<String>,
    /// Branch context at subscribe time. Same role as `space`.
    pub branch: Option<String>,
    /// The query body, preserved so refresh can re-issue it.
    pub query: JsValue,
    /// Consumer's opaque tag, round-tripped on every method
    /// call. Preserved so refresh delivers frames to the same
    /// per-stream handler on the consumer.
    pub tag: Option<JsValue>,
    /// Structural depth — number of consumer ancestors between
    /// the dispatcher and the host at subscribe time. Drives the
    /// shallowest-first refresh ordering.
    pub depth: u32,
    /// Upstream transport handle. Dropping it cancels the SSE.
    pub abort: Option<EventSource>,
}

/// The host's subscription table.
pub(crate) struct Registry {
    next_id: u64,
    entries: BTreeMap<EntryId, Entry>,
}

impl Registry {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            entries: BTreeMap::new(),
        }
    }

    /// Insert a new entry; returns its identifier.
    pub(crate) fn insert(&mut self, entry: Entry) -> EntryId {
        let id = EntryId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.entries.insert(id, entry);
        id
    }

    /// Install an abort handle on an existing entry. If the entry
    /// is gone (canceled during the await that produced the
    /// handle), the handle is dropped immediately, cancelling the
    /// upstream.
    pub(crate) fn install_abort(&mut self, id: EntryId, abort: EventSource) {
        match self.entries.get_mut(&id) {
            Some(e) => e.abort = Some(abort),
            None => drop(abort),
        }
    }

    /// Remove an entry by id, returning it. Dropping the entry
    /// drops its abort handle, cancelling the upstream.
    pub(crate) fn remove(&mut self, id: EntryId) -> Option<Entry> {
        self.entries.remove(&id)
    }

    /// Find all entries whose consumer matches the given element.
    /// Linear scan; v1 has few enough entries for this to be
    /// fine. Used by `tonk-unsubscribe` to find the consumer's
    /// entries.
    pub(crate) fn ids_for_consumer(&self, consumer: &Element) -> Vec<EntryId> {
        self.entries
            .iter()
            .filter(|(_, e)| &e.consumer == consumer)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get a reference to an entry by id.
    pub(crate) fn get(&self, id: EntryId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    /// Mutable access to the entry map. Exposed so the refresh
    /// path can update `abort` / `space` / `branch` in place.
    /// Whether the registry holds no live subscription.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn entries_mut(&mut self) -> &mut BTreeMap<EntryId, Entry> {
        &mut self.entries
    }

    /// Find all entries whose consumer is a DOM descendant of
    /// the given root element. Used by context refresh to scope
    /// the refresh to the subtree under a changed routing
    /// element.
    pub(crate) fn ids_under(&self, root: &Element) -> Vec<EntryId> {
        let root_node: &web_sys::Node = root.as_ref();
        self.entries
            .iter()
            .filter(|(_, e)| {
                let consumer_node: &web_sys::Node = e.consumer.as_ref();
                root_node.contains(Some(consumer_node))
            })
            .map(|(id, _)| *id)
            .collect()
    }
}
