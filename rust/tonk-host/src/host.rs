//! `<tonk-host>` custom element — IO owner.
//!
//! Page-level singleton. Mounted outside `<Routes>`. Owns:
//!
//! - Transport selection (fetch / SSE; bridge support deferred).
//! - Phase-1 descriptor cache.
//! - Subscription dedup table (deferred — v1 opens one upstream
//!   per consumer subscription).
//! - The central registry of live consumer subscriptions, keyed
//!   by `event.target`, recording `depth` for staggered refresh.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

use crate::navigate::{self, NavigateListener};
use crate::ops::{self, InstalledListener};
use crate::query_cache::QueryCache;
use crate::registry::Registry;

/// Internal state shared across listener closures.
pub(crate) struct HostState {
    /// Set true by `disconnected_callback` so closures running
    /// after detach bail before mutating state.
    pub disposed: bool,
    /// Subscription registry. Owns abort handles; entries are
    /// keyed by consumer element identity.
    pub registry: Registry,
    /// LRU for `tonk-query` responses. Repeats from any
    /// consumer in the page reuse the cached body and skip the
    /// HTTP round-trip. Invalidated per-branch on every claim
    /// or evaluate.
    pub query_cache: QueryCache,
}

impl HostState {
    fn new() -> Self {
        Self {
            disposed: false,
            registry: Registry::new(),
            query_cache: QueryCache::new(),
        }
    }
}

/// Outer per-element struct held by `custom-elements`. Holds
/// the shared state plus the listener handles so we can detach
/// on disconnect.
#[derive(Default)]
pub(crate) struct TonkHost {
    state: RefCell<Option<Rc<RefCell<HostState>>>>,
    listeners: RefCell<Vec<InstalledListener>>,
    /// `navigator.serviceWorker` `message` listener that performs
    /// worker-requested navigations (the main-thread navigate provider).
    navigate: RefCell<Option<NavigateListener>>,
}

impl CustomElement for TonkHost {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let state = Rc::new(RefCell::new(HostState::new()));
        *self.state.borrow_mut() = Some(state.clone());
        let installed = ops::attach_all(this, state);
        *self.listeners.borrow_mut() = installed;
        // Install the main-thread navigate provider: a worker command can
        // ask the page to redirect by posting `{ type: "navigate", href }`
        // to its client, and this listener performs it.
        *self.navigate.borrow_mut() = navigate::install();
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        let listeners = std::mem::take(&mut *self.listeners.borrow_mut());
        ops::detach_all(this, &listeners);
        if let Some(navigate) = self.navigate.borrow_mut().take() {
            navigate.remove();
        }
        if let Some(state) = self.state.borrow_mut().take() {
            let mut s = state.borrow_mut();
            s.disposed = true;
            s.registry.clear();
        }
    }
}

/// Register `<tonk-host>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkHost::define("tonk-host");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-host").is_undefined()
}

/// True iff the host has not yet been disconnected. Used by the
/// refresh loop to bail out if the host element was removed
/// mid-refresh.
pub(crate) fn is_alive(state: &std::rc::Rc<std::cell::RefCell<HostState>>) -> bool {
    !state.borrow().disposed
}
