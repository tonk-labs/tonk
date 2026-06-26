//! Tiny LRU for `tonk-query` responses.
//!
//! Every `<tonk-display>` mount fires a phase-1 query to resolve
//! its concept's descriptor. With multiple displays
//! mounted in a chain (board → column → tile → inspector-view),
//! that's N serial HTTP round-trips for what's effectively a
//! few unique concept lookups — measured at ~900 ms across the
//! board's chain. Stale data isn't a concern within a single
//! page session because concept descriptors barely change at
//! runtime; if a `tonk-claim` or `tonk-evaluate` lands on the
//! same `(space, branch)` we drop the cache to be safe.
//!
//! The cache is per-`<tonk-host>` instance (i.e. per-page) and
//! lives in `HostState`. Single-thread JS event loop — no sync
//! needed.

#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;
use std::collections::VecDeque;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

/// Cache key. Same query body posted to the same branch should
/// return the same response within a session.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Key {
    pub space: Option<String>,
    pub branch: Option<String>,
    pub body: String,
}

/// Fixed-capacity LRU plus an in-flight Promise table. The
/// expected unique-key set for one `<tonk-host>` session is
/// bounded by the concept surface of the mounted displays —
/// single digits to low tens. 64 is comfortable headroom; over
/// that we evict the least-recently used. `VecDeque` is fine at
/// this size — linear scans dominate only for caches an order
/// of magnitude larger.
///
/// The `pending` map deduplicates *concurrent* requests for the
/// same key. When N displays mount in parallel and each fires a
/// phase-1 query, the first triggers HTTP and stashes a Promise
/// here; the rest reuse it instead of stampeding the worker.
/// `pending` is wasm-only because `JsValue` only exists on
/// wasm; native code paths don't fan out in parallel anyway.
pub(crate) struct QueryCache {
    entries: VecDeque<(Key, String)>,
    capacity: usize,
    #[cfg(target_arch = "wasm32")]
    pending: HashMap<Key, JsValue>,
}

impl QueryCache {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            capacity: 64,
            #[cfg(target_arch = "wasm32")]
            pending: HashMap::new(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn get_pending(&self, key: &Key) -> Option<JsValue> {
        self.pending.get(key).cloned()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn put_pending(&mut self, key: Key, promise: JsValue) {
        self.pending.insert(key, promise);
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn clear_pending(&mut self, key: &Key) {
        self.pending.remove(key);
    }

    /// Look up a cached response. Moves the entry to MRU on hit
    /// so the next lookup of the same key is fast and the LRU
    /// eviction tracks actual usage.
    pub(crate) fn get(&mut self, key: &Key) -> Option<String> {
        let position = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(position)?;
        let response = entry.1.clone();
        self.entries.push_back(entry);
        Some(response)
    }

    /// Insert a (key, response) pair, evicting the LRU entry
    /// when at capacity.
    pub(crate) fn put(&mut self, key: Key, response: String) {
        if let Some(position) = self.entries.iter().position(|(k, _)| k == &key) {
            self.entries.remove(position);
        }
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, response));
    }

    /// Drop every entry for a given `(space, branch)`. Called on
    /// every claim / evaluate against that branch so a write
    /// doesn't leave the cache serving stale results.
    pub(crate) fn invalidate_branch(&mut self, space: Option<&str>, branch: Option<&str>) {
        self.entries
            .retain(|(k, _)| k.space.as_deref() != space || k.branch.as_deref() != branch);
        #[cfg(target_arch = "wasm32")]
        self.pending
            .retain(|k, _| k.space.as_deref() != space || k.branch.as_deref() != branch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Run wasm32 tests in the browser (ChromeDriver), matching the
    // sibling tonk-* crates. Without this the default wasm-bindgen
    // runner is Node.js, which the CI web test leg does not provide.
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn key(branch: &str, body: &str) -> Key {
        Key {
            space: Some("home".into()),
            branch: Some(branch.into()),
            body: body.into(),
        }
    }

    #[dialog_common::test]
    fn it_returns_none_for_a_cold_lookup() {
        let mut cache = QueryCache::new();
        assert!(cache.get(&key("main", "q")).is_none());
    }

    #[dialog_common::test]
    fn it_returns_the_response_for_a_warm_lookup() {
        let mut cache = QueryCache::new();
        cache.put(key("main", "q"), "result".into());
        assert_eq!(cache.get(&key("main", "q")).as_deref(), Some("result"));
    }

    #[dialog_common::test]
    fn it_overwrites_a_repeated_put_with_the_latest_response() {
        let mut cache = QueryCache::new();
        cache.put(key("main", "q"), "v1".into());
        cache.put(key("main", "q"), "v2".into());
        assert_eq!(cache.get(&key("main", "q")).as_deref(), Some("v2"));
    }

    #[dialog_common::test]
    fn it_evicts_the_least_recently_used_entry_at_capacity() {
        let mut cache = QueryCache::new();
        cache.capacity = 2;
        cache.put(key("main", "a"), "1".into());
        cache.put(key("main", "b"), "2".into());
        // Touch `a` so `b` becomes the LRU.
        assert!(cache.get(&key("main", "a")).is_some());
        cache.put(key("main", "c"), "3".into());
        assert!(cache.get(&key("main", "b")).is_none());
        assert!(cache.get(&key("main", "a")).is_some());
        assert!(cache.get(&key("main", "c")).is_some());
    }

    #[dialog_common::test]
    fn it_invalidates_every_entry_for_a_given_branch() {
        let mut cache = QueryCache::new();
        cache.put(key("main", "a"), "1".into());
        cache.put(key("main", "b"), "2".into());
        cache.put(key("dev", "a"), "3".into());
        cache.invalidate_branch(Some("home"), Some("main"));
        assert!(cache.get(&key("main", "a")).is_none());
        assert!(cache.get(&key("main", "b")).is_none());
        assert!(cache.get(&key("dev", "a")).is_some());
    }
}
