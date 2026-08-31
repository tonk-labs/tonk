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
    _observer: Option<WithObserver>,
    _controller: Option<ControllerWatch>,
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
        // (Sync needs no page-side heartbeat: the SW schedules its own
        // drains while the page holds live subscriptions.)
        _navigate: page_effects.then(navigate::install).flatten(),
        _observer: WithObserver::install(&document, state.clone()),
        _controller: page_effects
            .then(|| ControllerWatch::install(state.clone()))
            .flatten(),
        _state: state.clone(),
    };
    INSTALLED.with(|cell| *cell.borrow_mut() = Some(installed));
    if page_effects {
        spawn_keepalive(state);
    }
    tonk_common::log!(
        "tonk-host: installed (page_effects={page_effects}) on {}",
        document.url().unwrap_or_default()
    );
}

/// Refreshes every subscription the moment a new service worker takes
/// over (`controllerchange`): the old worker's streams are dead or about
/// to be, and one immediate pass beats trickling back on jittered retry
/// timers. Top page only — sealed guests have no service-worker container;
/// their streams ride this page's relays and heal with it.
struct ControllerWatch {
    _closure: Closure<dyn FnMut()>,
}

impl ControllerWatch {
    fn install(state: Rc<RefCell<HostState>>) -> Option<Self> {
        let container = window()?.navigator().service_worker();
        let closure = Closure::wrap(Box::new(move || {
            ops::refresh_all(&state);
        }) as Box<dyn FnMut()>);
        container
            .add_event_listener_with_callback("controllerchange", closure.as_ref().unchecked_ref())
            .ok()?;
        Some(Self { _closure: closure })
    }
}

/// Keep the service worker alive while this page holds live
/// subscriptions. An SSE response body does NOT extend the worker's
/// lifetime, and SW-internal timers don't either — so without page
/// events the browser terminates the worker (~30s idle), closing every
/// stream and forcing an endless reconnect/re-stamp churn the nested
/// guests heal slower than it repeats. A `POST /api/sync?why=keepalive`
/// every 10s is a real fetch event: it extends the worker's lifetime and
/// rides the same debounced drain scheduling as any other request.
/// Skipped while the page holds no subscriptions (nothing to keep alive
/// for — the browser may reclaim the worker) or is offline. Top page
/// only: guests relay through it, so its keepalive covers the tab.
fn spawn_keepalive(state: Rc<RefCell<HostState>>) {
    wasm_bindgen_futures::spawn_local(async move {
        loop {
            crate::ops::wait_ms(10_000).await;
            if state.borrow().registry.is_empty() {
                continue;
            }
            let Some(win) = window() else { return };
            if !win.navigator().on_line() {
                continue;
            }
            // A pending update means the CURRENT worker is on its way out:
            // poking it would extend the very lifetime `skipWaiting` is
            // waiting to end. Hold the keepalive until the takeover; the
            // controller-change refresh re-establishes everything.
            if update_pending().await {
                continue;
            }
            let Ok(headers) = crate::http::request_context_headers() else {
                continue;
            };
            let init = web_sys::RequestInit::new();
            init.set_method("POST");
            init.set_headers(&headers);
            // Awaiting consumes the rejection: a beat that loses the race
            // with a navigation or a worker swap fails quietly, and the
            // next beat covers.
            let _ = wasm_bindgen_futures::JsFuture::from(
                win.fetch_with_str_and_init("/api/sync?why=keepalive", &init),
            )
            .await;
        }
    });
}

/// Whether a newer service worker is installing or waiting to take over.
async fn update_pending() -> bool {
    let Some(win) = window() else {
        return false;
    };
    let Ok(registration) =
        wasm_bindgen_futures::JsFuture::from(win.navigator().service_worker().get_registration())
            .await
    else {
        return false;
    };
    let Ok(registration) = registration.dyn_into::<web_sys::ServiceWorkerRegistration>() else {
        return false;
    };
    registration.waiting().is_some() || registration.installing().is_some()
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
