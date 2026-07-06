//! The installed host — IO owner, no element.
//!
//! `install()` is called once at app boot. It attaches the
//! operation-event listeners to `document` (consumer events bubble
//! there for free — no ancestor element required), installs the
//! main-thread navigate provider and the idle-sync heartbeat, and
//! starts the `with` observer that refreshes affected subscriptions
//! when a routing context changes. State (the subscription
//! registry) lives in a thread-local for the page's lifetime.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, MutationObserver, MutationObserverInit, MutationRecord, window};

use crate::idle_sync::{self, IdleSync};
use crate::navigate::{self, NavigateListener};
use crate::ops::{self, InstalledListener};
use crate::registry::Registry;

/// Internal state shared across listener closures.
pub(crate) struct HostState {
    /// Subscription registry. Owns abort handles; entries are
    /// keyed by consumer element identity.
    pub registry: Registry,
}

impl HostState {
    fn new() -> Self {
        Self {
            registry: Registry::new(),
        }
    }
}

/// Everything `install()` wires up, held for the page's lifetime.
struct Installed {
    _state: Rc<RefCell<HostState>>,
    _listeners: Vec<InstalledListener>,
    _navigate: Option<NavigateListener>,
    _idle_sync: Option<IdleSync>,
    _observer: Option<WithObserver>,
}

thread_local! {
    static INSTALLED: RefCell<Option<Installed>> = const { RefCell::new(None) };
}

/// Install the host on the top page: document-level operation
/// listeners, the navigate provider, the idle-sync heartbeat, and
/// the `with` observer. Idempotent — a second call is a no-op.
pub fn install() {
    install_inner(true);
}

/// Install ONLY the IO surface — operation listeners + the `with`
/// observer — without the top-page-only effects (navigate provider,
/// idle-sync heartbeat). The sealed guest uses this: its `fetch` is
/// the portal bootstrap's relay, the top page already heartbeats for
/// the whole tab, and worker navigation messages only reach real
/// service-worker clients. Idempotent.
pub fn install_io() {
    install_inner(false);
}

fn install_inner(page_effects: bool) {
    let already = INSTALLED.with(|cell| cell.borrow().is_some());
    if already {
        tonk_common::log!("tonk-host: install skipped — already installed");
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        tonk_common::log!("tonk-host: install failed — no window/document");
        return;
    };
    let state = Rc::new(RefCell::new(HostState::new()));
    let listeners = ops::attach_all(document.as_ref(), state.clone());
    let installed = Installed {
        _listeners: listeners,
        // Main-thread navigate provider: a worker command can ask the page
        // to redirect by posting `{ type: "navigate", href }` to its client.
        _navigate: page_effects.then(navigate::install).flatten(),
        // Idle sync heartbeat: polls `POST /api/sync` on a
        // `requestIdleCallback` loop (and on refocus/reconnect) so an idle
        // tab still pulls upstream changes.
        _idle_sync: page_effects.then(idle_sync::install).flatten(),
        _observer: WithObserver::install(&document, state.clone()),
        _state: state,
    };
    INSTALLED.with(|cell| *cell.borrow_mut() = Some(installed));
    tonk_common::log!(
        "tonk-host: installed (page_effects={page_effects}) on {}",
        document.url().unwrap_or_default()
    );
}

/// Document-wide observer of `with` attribute mutations. A changed
/// routing context (a re-stamped template row, a navigation
/// rewriting a wrapper's `with`) triggers the depth-staggered
/// refresh of every subscription under the mutated element — the
/// replacement for the routing elements' `attributeChangedCallback`
/// → `tonk-context-refresh` flow.
struct WithObserver {
    observer: MutationObserver,
    _closure: Closure<dyn FnMut(Array, MutationObserver)>,
}

impl WithObserver {
    fn install(document: &web_sys::Document, state: Rc<RefCell<HostState>>) -> Option<Self> {
        let closure = Closure::wrap(Box::new(move |records: Array, _observer| {
            for record in records.iter() {
                let Ok(record) = record.dyn_into::<MutationRecord>() else {
                    continue;
                };
                let Some(target) = record.target().and_then(|n| n.dyn_into::<Element>().ok())
                else {
                    continue;
                };
                ops::refresh_under(&state, &target);
            }
        }) as Box<dyn FnMut(Array, MutationObserver)>);
        let observer = MutationObserver::new(closure.as_ref().unchecked_ref()).ok()?;
        let init = MutationObserverInit::new();
        init.set_subtree(true);
        init.set_attributes(true);
        init.set_attribute_filter(&Array::of1(&"with".into()));
        let root = document.document_element()?;
        observer.observe_with_options(&root, &init).ok()?;
        Some(Self {
            observer,
            _closure: closure,
        })
    }
}

impl Drop for WithObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}
