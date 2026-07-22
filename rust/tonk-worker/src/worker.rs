//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    LspHub,
    axum::{RequestConversion, ResponseConversion},
    bootstrap_profile,
    router::{AppState, BridgeRegistry, ClientId, ViewBindings, api_router_with_state},
};
use axum::{
    Router,
    body::Body,
    http::{HeaderValue, header::HeaderName},
};
use dialog_capability::Subject;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::Storage;
use js_sys::Promise;
use tokio::sync::Mutex;
use tonk_common::log;
use tower_service::Service;
use wasm_bindgen::prelude::*;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{FetchEvent, Request, Response};

// Global `self.fetch(...)` in the service-worker scope. Fetches
// issued from an SW bypass the SW's own `onfetch` listener (per
// spec), so this is how we pass through requests the Rust side
// chose not to handle.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = fetch)]
    fn sw_fetch(request: &Request) -> Promise;
}

/// Extract the `resultingClientId` property from a `FetchEvent`.
///
/// `web-sys` exposes `FetchEvent.client_id()` but not
/// `resultingClientId`. For navigation requests `client_id` is
/// empty and `resultingClientId` carries the ID the new document
/// will have, so we need both. Read the property via reflection
/// rather than extending `FetchEvent` with a manual extern —
/// `wasm-bindgen` doesn't allow extending foreign types from
/// outside their defining crate.
fn event_resulting_client_id(event: &FetchEvent) -> String {
    js_sys::Reflect::get(event, &JsValue::from_str("resultingClientId"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Outcome of the routing decision made in
/// [`TonkServiceWorker::on_fetch`].
#[derive(Clone, Debug)]
enum Route {
    /// Dispatch through the axum router. `rewritten_path` is
    /// `Some` only when the request originated from a registered
    /// guest iframe and its path needs to be re-rooted under the
    /// iframe's branch.
    Handle { rewritten_path: Option<String> },
    /// Pass the request through to the network.
    Passthrough,
    /// Returned for view clients trying to reach the data plane
    /// directly. The SW synthesises a 404 response without
    /// invoking axum.
    Reject,
}

/// Decides how the Rust side should route this request.
///
/// - View clients (registered via `view_bindings`) hitting `/api/`
///   paths receive [`Route::Reject`] — the SW returns a synthetic
///   404 without invoking axum. The data plane is not reachable
///   directly from an iframe; only the bridge is.
/// - Paths under `/api/` from non-view clients are handled by the
///   axum router as-is.
/// - A small allow-list of shared dist-root assets (the iframe
///   bridge module) is passed straight through, even from guest
///   iframes — see [`is_shared_asset`] for why.
/// - Requests whose initiating client is a registered guest
///   iframe are *also* handled by the router, but with their
///   path rewritten to live under
///   `/api/repository/{repo}/branch/{branch}`. That gives the
///   iframe a virtual root scoped to its branch: a fetch for
///   `/foo.js` lands at `/api/repository/{repo}/branch/{branch}/foo.js`.
/// - Everything else falls through to the network.
async fn route_for(path: &str, client_id: &str, state: &AppState) -> Route {
    // Look up the view binding early; we need it both for the
    // reject check and for the later path-rewrite.
    let view_binding = if client_id.is_empty() {
        None
    } else {
        let bindings = state.read().await.view_bindings.clone();
        let guard = bindings.read().await;
        guard.get(&ClientId(client_id.to_string())).cloned()
    };

    // View clients are walled off from the data plane. Any
    // `/api/...` from a registered view client is a code path
    // we deliberately removed; surface it as a 404 so the
    // iframe-side bug is visible.
    if view_binding.is_some() && path.starts_with("/api/") {
        return Route::Reject;
    }

    if path.starts_with("/api/") {
        return Route::Handle {
            rewritten_path: None,
        };
    }
    if is_shared_asset(path) {
        return Route::Passthrough;
    }
    let Some(binding) = view_binding else {
        return Route::Passthrough;
    };

    let suffix = if path == "/" || path.is_empty() {
        String::new()
    } else {
        path.to_string()
    };
    let rewritten = format!(
        "/api/repository/{}/branch/{}{}",
        binding.repo, binding.branch, suffix,
    );
    Route::Handle {
        rewritten_path: Some(rewritten),
    }
}

/// Paths that must reach the network even when the requesting
/// client is a registered guest iframe. The iframe shell loads
/// `/__tonk/bridge.js` to install `globalThis.tonk`; without this
/// exemption the guest-binding rewrite would re-root that fetch
/// under the iframe's branch and 404.
fn is_shared_asset(path: &str) -> bool {
    matches!(path, "/__tonk/bridge.js")
}

/// Route the request through the axum router, apply response
/// headers (CORS, client-id echo), and convert back to a browser
/// `Response`.
///
/// If `rewritten_path` is `Some`, the request's URI is rewritten
/// to that path before axum gets it — that's how guest-iframe
/// requests get redirected into branch-scoped routes without
/// axum's own routing layer ever seeing the un-scoped URL.
async fn handle_via_router(
    router: Arc<Mutex<Router>>,
    browser_request: Request,
    client_id: String,
    rewritten_path: Option<String>,
) -> Result<JsValue, JsValue> {
    let mut request: axum::http::Request<Body> = RequestConversion::from(browser_request)
        .into_axum_request()
        .await
        .map_err(JsValue::from)?;

    if let Some(new_path) = rewritten_path {
        let uri_string = match request.uri().query() {
            Some(q) => format!("{}?{}", new_path, q),
            None => new_path.clone(),
        };
        if let Ok(uri) = uri_string.parse::<axum::http::Uri>() {
            log!("handle_via_router: rewriting path to {}", uri);
            *request.uri_mut() = uri;
        }
    }

    if !client_id.is_empty() {
        request.extensions_mut().insert(ClientId(client_id.clone()));
    }

    // Clone the router out of the lock and release the guard before
    // dispatching: `Router` is cheap to clone (its routes are `Arc`-shared)
    // and `Service::call` runs on the owned clone, so concurrent requests
    // aren't serialized behind one global lock. Holding the lock across
    // `.call().await` would queue every request behind the slowest one (e.g. a
    // network-bound `sync/status`).
    let mut router = router.lock().await.clone();
    let mut response = router
        .call(request)
        .await
        .expect_throw("Failed to handle API request");

    let headers = response.headers_mut();

    if !client_id.is_empty()
        && let Ok(value) = HeaderValue::from_str(&client_id)
    {
        headers.insert(HeaderName::from_static("x-tonk-client-id"), value);
    }

    // CORS: sandboxed iframes have an opaque origin and send
    // `Origin: null`, so cross-origin rules apply even though
    // the iframe and the SW share an origin. Send permissive
    // headers on every response — same-origin callers ignore
    // them and opaque-origin callers need them to read the body.
    headers.insert(
        HeaderName::from_static("access-control-allow-origin"),
        HeaderValue::from_static("*"),
    );
    headers.insert(
        HeaderName::from_static("access-control-allow-methods"),
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        HeaderName::from_static("access-control-allow-headers"),
        HeaderValue::from_static("content-type, if-none-match, accept"),
    );
    headers.insert(
        HeaderName::from_static("access-control-expose-headers"),
        HeaderValue::from_static("x-tonk-client-id"),
    );

    ResponseConversion::from(response)
        .try_into()
        .map(|value: Response| JsValue::from(value))
        .map_err(JsValue::from)
}

/// Synthesise a 404 response for view clients hitting `/api/...`.
///
/// The bridge is the only data plane an iframe gets; everything
/// else is closed off so the iframe-side bug — "tried to fetch
/// the API directly" — surfaces immediately.
///
/// Only reachable at runtime from `on_fetch`, which is invoked
/// exclusively in a WASM service-worker context.
fn reject_404() -> Result<JsValue, JsValue> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        let init = web_sys::ResponseInit::new();
        init.set_status(404);
        let response = web_sys::Response::new_with_opt_str_and_init(
            Some("view clients cannot reach /api/; use the bridge"),
            &init,
        )?;
        Ok(JsValue::from(response))
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        unreachable!("reject_404 is only callable in a WASM service-worker context")
    }
}

/// Pass the request through to the network or the shell cache.
///
/// Cacheable GETs go through stale-while-revalidate against the
/// shell cache — repeat loads serve from memory instead of
/// re-downloading Webawesome's chunk graph, Trunk-hashed JS/Wasm,
/// or the self-hosted font set.
///
/// Document navigations don't reach this function: the JS shim
/// serves them directly from the SW cache so navigation TTFB
/// doesn't wait on the Rust worker to initialize. This keeps
/// the data plane (`/api/*`) on the Rust side without paying
/// the worker boot cost for the HTML shell.
///
/// Non-cacheable requests (non-GETs, opaque/cross-origin) fall
/// through to `self.fetch(request)` to hit the network directly.
/// Such fetches bypass the SW's own `onfetch` handler per spec.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn passthrough(request: Request, _is_navigation: bool) -> Result<JsValue, JsValue> {
    let path = url::Url::parse(&request.url())
        .map(|u| u.path().to_string())
        .unwrap_or_default();

    if crate::cache::is_cacheable(&request, &path) {
        return crate::cache::stale_while_revalidate(&request).await;
    }

    let response: Response = JsFuture::from(sw_fetch(&request)).await?.dyn_into()?;
    Ok(JsValue::from(response))
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn passthrough(_request: Request, _is_navigation: bool) -> Result<JsValue, JsValue> {
    unreachable!("passthrough is only callable in a WASM service-worker context")
}

/// Default storage type — IndexedDB on WASM, filesystem on native.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub type DefaultSpace = dialog_storage::provider::storage::WebSpace;

/// Default storage type — filesystem on native.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub type DefaultSpace = dialog_storage::provider::storage::NativeSpace;

/// Concrete operator type for the default storage backend.
pub type DefaultOperator = Operator<DefaultSpace>;

/// Name of the profile this worker opens on startup. Also used as
/// the label for the profile's self-replica record in its meta
/// branch.
const PROFILE_NAME: &str = "tonk";

/// Application state containing the profile and operator.
pub struct TonkState {
    /// The user's persistent profile.
    pub profile: Profile,
    /// The operator derived from the profile.
    pub operator: DefaultOperator,
    /// Display name the profile was opened under. `Profile` does
    /// not retain this internally, so we carry it here for routes
    /// that report it back to the UI (e.g. `GET /api/profile`).
    pub profile_name: String,
    /// Reactive layer: cached repository/branch handles and the
    /// query subscriptions registered against them. Routes that
    /// mutate a branch flow through `reactor.repository(r).branch(b)`
    /// so subscription broadcasts happen automatically.
    pub reactor: crate::Reactor,
    /// View-iframe bindings keyed by service-worker Client ID.
    /// Behind its own interior lock so binding registration /
    /// lookup doesn't contend with profile/operator access on
    /// the outer state lock.
    pub view_bindings: ViewBindings,
    /// Per-client bridge sessions keyed by service-worker Client
    /// ID. Each session owns the transferred `MessagePort` and the
    /// abort handles for any open subscriptions on that client.
    pub bridges: BridgeRegistry,
    /// Registered command handlers — the typed-Rust effects fired by
    /// transient command concepts after a commit. Consulted by the
    /// transact path's post-commit dispatch.
    pub commands: crate::reactor::CommandRegistry<crate::router::CommandEnv>,
    /// Repositories with un-pushed local commits. A commit enqueues its repo;
    /// `POST /api/sync` (the page heartbeat) and the post-commit push drain
    /// reconcile it. See `router::sync::SyncQueue`.
    pub sync_queue: crate::router::SyncQueue,
    /// Liveness ledger: SW client → what it registered (site stamps) and
    /// whether we have observed it alive. The stale-client sweep reaps
    /// born-then-died clients from here. See [`crate::router::ClientRegistry`].
    pub clients: crate::router::ClientRegistry,
}

// SAFETY: Web browsers run Wasm in a single thread only. The interior types
// (Profile, Operator) contain `web_sys::CryptoKey` handles (via
// Ed25519SigningKey::WebCrypto) which are !Send/!Sync, but cross-thread access
// cannot occur in a single-threaded browser context.
#[cfg(target_arch = "wasm32")]
unsafe impl Send for TonkState {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for TonkState {}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod route_for_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use super::*;
    use crate::router;

    async fn state_with_view_client(client: &str, repo: &str, branch: &str) -> AppState {
        let state = router::tests::test_state().await;
        let app_state: AppState = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let bindings = app_state.read().await.view_bindings.clone();
        bindings.write().await.insert(
            ClientId(client.to_owned()),
            router::ViewBinding {
                repo: repo.to_owned(),
                branch: branch.to_owned(),
            },
        );
        app_state
    }

    #[dialog_common::test]
    async fn it_rejects_api_paths_from_view_clients() {
        let state = state_with_view_client("c1", "r", "main").await;
        let r = route_for("/api/repository/r/branch/main/query", "c1", &state).await;
        assert!(
            matches!(r, Route::Reject),
            "view client should be rejected, got {r:?}",
        );
    }

    #[dialog_common::test]
    async fn it_lets_non_view_clients_through_to_api() {
        let state = router::tests::test_state().await;
        let app_state: AppState = std::sync::Arc::new(tokio::sync::RwLock::new(state));
        let r = route_for(
            "/api/repository/r/branch/main/query",
            "no-such-client",
            &app_state,
        )
        .await;
        assert!(
            matches!(
                r,
                Route::Handle {
                    rewritten_path: None
                }
            ),
            "non-view client should pass through to axum, got {r:?}",
        );
    }

    #[dialog_common::test]
    async fn it_still_rewrites_static_subresources_for_view_clients() {
        let state = state_with_view_client("c1", "r", "main").await;
        let r = route_for("/foo.js", "c1", &state).await;
        assert!(
            matches!(
                r,
                Route::Handle {
                    rewritten_path: Some(_)
                }
            ),
            "view client static subresource should be rewritten, got {r:?}",
        );
    }

    #[dialog_common::test]
    async fn it_passes_through_bridge_js_for_view_clients() {
        let state = state_with_view_client("c1", "r", "main").await;
        let r = route_for("/__tonk/bridge.js", "c1", &state).await;
        assert!(
            matches!(r, Route::Passthrough),
            "bridge module must bypass the rewrite, got {r:?}",
        );
    }

    // The scheduler's clock is passed in (not `Date::now()`), so these drive it
    // with fixed timestamps — no real time, fully deterministic.

    #[dialog_common::test]
    fn it_coalesces_a_burst_into_the_last_ticket() {
        let s = SyncScheduler::default();
        // Three requests in the same instant: only the last ticket is current,
        // so the first two wake and no-op, the last drains.
        let t1 = s.next(0.0);
        let t2 = s.next(0.0);
        let t3 = s.next(0.0);
        let wake = SYNC_DEBOUNCE_MS as f64;
        assert!(
            !s.should_drain(t1, wake, false),
            "superseded ticket must not drain"
        );
        assert!(
            !s.should_drain(t2, wake, false),
            "superseded ticket must not drain"
        );
        assert!(s.should_drain(t3, wake, false), "latest ticket drains");
    }

    #[dialog_common::test]
    fn it_forces_a_drain_when_traffic_never_settles() {
        let s = SyncScheduler::default();
        // A request arrives every 100ms, faster than the 500ms debounce, so no
        // ticket is ever the latest when it wakes. Without the cap this starves
        // forever; with it, once the burst has been pending SYNC_MAX_WAIT_MS an
        // earlier ticket drains despite the newer requests.
        let first = s.next(0.0);
        for i in 1..=40 {
            s.next(i as f64 * 100.0);
        }
        // `first` woke long ago and is not the latest — but the burst began at
        // t=0 and we're now past the cap, so it drains.
        assert!(
            s.should_drain(first, SYNC_MAX_WAIT_MS as f64, false),
            "max-wait cap must force a drain under continuous traffic",
        );
        // Just before the cap, the same non-latest ticket must NOT drain.
        assert!(
            !s.should_drain(first, SYNC_MAX_WAIT_MS as f64 - 1.0, false),
            "before the cap, a superseded ticket still defers",
        );
    }

    #[dialog_common::test]
    fn it_restarts_the_cap_clock_after_a_drain() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        // Drain at the trailing edge, which clears the burst clock.
        assert!(s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false));
        s.begin_drain();
        s.end_drain(SYNC_DEBOUNCE_MS as f64);
        // A fresh burst after the drain starts a NEW cap clock at t=10_000. Use a
        // superseded ticket (t2, then t3 bumps the generation) so `is_latest` is
        // false and only the cap can trigger a drain — that's what proves the
        // clock reset. If the cap still measured from wall-clock zero, t2 would
        // already be past the cap; it must instead measure from t=10_000.
        let t2 = s.next(10_000.0);
        let _t3 = s.next(10_050.0);
        assert!(
            !s.should_drain(t2, 10_000.0 + SYNC_MAX_WAIT_MS as f64 - 1.0, false),
            "cap must measure from the post-drain burst start, not wall-clock zero",
        );
        assert!(
            s.should_drain(t2, 10_000.0 + SYNC_MAX_WAIT_MS as f64, false),
            "once the new burst passes the cap, a superseded ticket drains",
        );
    }

    #[dialog_common::test]
    fn it_never_overlaps_two_drains() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        s.begin_drain();
        // While a drain is in flight, even a past-the-cap ticket must wait.
        assert!(
            !s.should_drain(t1, SYNC_MAX_WAIT_MS as f64 * 10.0, false),
            "in_flight guard must block a second concurrent drain",
        );
        s.end_drain(SYNC_MAX_WAIT_MS as f64 * 10.0);
    }

    #[dialog_common::test]
    fn it_defers_a_drain_while_a_page_is_loading() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        // A data-plane request is in flight (a page booting): even the latest
        // ticket at its debounce edge must NOT drain — sync yields the thread.
        let _guard = s.enter_loading(0.0);
        assert!(
            !s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false),
            "an in-flight data-plane request defers the drain",
        );
    }

    #[dialog_common::test]
    fn it_drains_once_the_load_settles() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        {
            let _guard = s.enter_loading(0.0);
            assert!(!s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false));
        }
        // Guard dropped — the request completed, so the drain proceeds.
        assert!(
            s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false),
            "the drain runs once the in-flight request completes",
        );
    }

    /// The cooldown must stay strictly under the loop interval, or the idle
    /// pull cadence is not the interval at all.
    ///
    /// The loop ticks every [`SYNC_LOOP_MS`] and each tick is refused if it
    /// lands inside the cooldown measured from the last drain's completion. At
    /// cooldown >= interval, a tick arriving right after a drain is always
    /// refused and the page waits for the tick *after* it — silently doubling
    /// the worst-case latency for seeing another device's change, with nothing
    /// in the logs to say so.
    #[dialog_common::test]
    fn it_keeps_the_cooldown_under_the_loop_interval() {
        assert!(
            (SYNC_COOLDOWN_MS as u64) < SYNC_LOOP_MS,
            "cooldown ({SYNC_COOLDOWN_MS}ms) must be under the loop interval \
             ({SYNC_LOOP_MS}ms), else every other idle tick is refused and the \
             real pull cadence is 2x the interval",
        );
    }

    /// A stopped worker refuses every drain — that is the point of the flag: a
    /// worker being replaced must start no new sync work, or it re-arms
    /// `waitUntil` and pins itself in `waiting`.
    #[dialog_common::test]
    fn it_refuses_every_drain_once_stopped() {
        let s = SyncScheduler::default();
        let t = s.next(0.0);
        s.stop();
        assert!(
            !s.may_drain(0.0, false),
            "a stopped worker starts no sync work"
        );
        assert!(!s.should_drain(t, SYNC_DEBOUNCE_MS as f64, false));
    }

    /// ...but `stop()` must not be a ONE-WAY latch. `updatefound` fires on the
    /// registration, so a newly-installing worker hears it about its own
    /// arrival and can stop itself; it then activates and serves the page. It
    /// must sync. `resume()` (called from `onactivate`) is what un-latches it —
    /// without it that worker refuses every drain for the rest of its life.
    #[dialog_common::test]
    fn it_resumes_when_it_turns_out_to_be_the_serving_worker() {
        let s = SyncScheduler::default();
        s.stop();
        assert!(!s.may_drain(0.0, false));

        // Activating: we are the worker now serving, so we are not retiring.
        s.resume();

        assert!(
            s.may_drain(0.0, false),
            "an activated worker must sync, even if it previously stopped itself",
        );
        let t = s.next(0.0);
        assert!(s.should_drain(t, SYNC_DEBOUNCE_MS as f64, false));
    }

    #[dialog_common::test]
    fn it_stops_deferring_past_the_load_cap() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        // A request that never completes (held guard) must not defer sync
        // forever: past SYNC_LOAD_DEFER_MS from its start, the drain proceeds.
        let _guard = s.enter_loading(0.0);
        assert!(
            !s.should_drain(t1, SYNC_LOAD_DEFER_MS as f64 - 1.0, false),
            "within the cap, an in-flight request still defers",
        );
        assert!(
            s.should_drain(t1, SYNC_LOAD_DEFER_MS as f64, false),
            "past the cap, a stuck request no longer defers the drain",
        );
    }

    #[dialog_common::test]
    fn it_counts_concurrent_loads() {
        let s = SyncScheduler::default();
        let t1 = s.next(0.0);
        let g1 = s.enter_loading(0.0);
        let g2 = s.enter_loading(0.0);
        drop(g1);
        // One of two concurrent requests finished; the other still defers
        // (e.g. a second tab still booting).
        assert!(
            !s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false),
            "a drain waits while any data-plane request is still in flight",
        );
        drop(g2);
        assert!(s.should_drain(t1, SYNC_DEBOUNCE_MS as f64, false));
    }

    #[dialog_common::test]
    fn it_holds_drains_to_the_hidden_interval_while_hidden() {
        let s = SyncScheduler::default();
        s.set_visible(false);
        s.begin_drain();
        s.end_drain(0.0);
        let t = s.next(10_000.0);
        assert!(
            !s.should_drain(t, SYNC_HIDDEN_INTERVAL_MS as f64 - 1.0, false),
            "hidden pages must not drain at the active cadence"
        );
        assert!(s.should_drain(t, SYNC_HIDDEN_INTERVAL_MS as f64 + 1.0, false));
    }

    /// The user edits, then switches tabs. Their un-pushed commit must not
    /// wait out the hidden interval — that is a minute during which
    /// collaborators see nothing, and the page may never come back.
    #[dialog_common::test]
    fn it_drains_pending_local_work_at_the_active_cadence_while_hidden() {
        let s = SyncScheduler::default();
        s.set_visible(false);
        s.begin_drain();
        s.end_drain(0.0);
        let t = s.next(1_000.0);

        assert!(
            !s.should_drain(t, SYNC_COOLDOWN_MS as f64 + 1.0, false),
            "with nothing to push, a hidden page still holds to the hidden interval"
        );
        assert!(
            s.should_drain(t, SYNC_COOLDOWN_MS as f64 + 1.0, true),
            "a non-empty dirty set bypasses the hidden interval"
        );
        assert!(
            !s.should_drain(t, SYNC_COOLDOWN_MS as f64 - 1.0, true),
            "the bypass drops to the cooldown floor, it does not remove the floor"
        );
    }

    /// A visible page always polls at the active cadence, however long it
    /// has sat idle: the only thing that widens the gap is going hidden,
    /// and coming back drops it again on the very next gate check. A
    /// visible-but-idle reader must never lag a collaborator.
    #[dialog_common::test]
    fn it_resumes_the_active_cadence_on_becoming_visible() {
        let s = SyncScheduler::default();
        s.set_visible(false);
        s.begin_drain();
        s.end_drain(0.0);
        let t = s.next(10_000.0);
        assert!(
            !s.should_drain(t, 10_000.0, false),
            "a hidden page holds to the hidden interval"
        );

        s.set_visible(true);
        assert_eq!(
            s.quiet_interval(),
            0.0,
            "regaining visibility must clear the hidden hold"
        );
        assert!(
            s.should_drain(t, 10_000.0, false),
            "the same ticket now drains at the active cadence"
        );
    }
}

/// Trailing-edge debounce coordinator for the background sync drain, with a
/// max-wait cap so continuous traffic can't defer the drain forever.
///
/// Every `on_fetch` bumps `generation` and captures its value, then schedules a
/// drain after a quiet window. After the window the request drains if its
/// captured generation is still current (no newer request superseded it) —
/// bursts collapse into one trailing-edge drain, the last request wins.
///
/// The trap that alone leaves open: a tab making requests *faster* than the
/// debounce window keeps bumping the generation, so no ticket is ever the last
/// one and the drain starves indefinitely. So `pending_since` stamps when the
/// current un-drained burst began; once it has waited [`SYNC_MAX_WAIT_MS`], a
/// woken ticket drains even though newer requests exist. Cleared on every drain,
/// so the cap measures from the first request after the last drain.
///
/// `in_flight` guards against two drains overlapping if a new winner (or a
/// max-wait firing) starts while the previous drain is still running.
///
/// Single-threaded SW, so plain `Cell`s behind `Rc` — no atomics needed.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[derive(Clone)]
struct SyncScheduler {
    generation: std::rc::Rc<std::cell::Cell<u64>>,
    in_flight: std::rc::Rc<std::cell::Cell<bool>>,
    /// Count of outstanding data-plane (`/api/*`) requests. A sync drain
    /// competes with these for the single SW thread and the reactor's branch
    /// locks, so it holds off while any are in flight — a page actively loading
    /// (this tab or another) finishes its boot before sync does network work.
    /// Bounded by [`SYNC_LOAD_DEFER_MS`] so a hung request can't defer sync
    /// forever.
    loading: std::rc::Rc<std::cell::Cell<u32>>,
    /// Wall-clock ms of the last data-plane request start, used with `loading`
    /// to cap how long an in-flight request may defer a drain.
    last_request_at: std::rc::Rc<std::cell::Cell<f64>>,
    /// Wall-clock ms when the current un-drained burst began, or `None` when the
    /// last drain cleared it. The max-wait cap measures from here.
    pending_since: std::rc::Rc<std::cell::Cell<Option<f64>>>,
    /// The request that began the current un-drained burst. The drain is
    /// coalesced at the trailing edge, so the burst-opener is what initiated
    /// it — later requests just ride the debounce. Taken (and logged) by the
    /// ticket that actually drains.
    cause: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    /// Wall-clock ms when the last drain FINISHED, so the next one can be
    /// held off for [`SYNC_COOLDOWN_MS`]. The debounce measures from the
    /// triggering request, which says nothing about how long the previous
    /// drain ran: on a slow link a drain can outlast the loop's interval, so
    /// without a completion-relative gap the next drain starts the moment the
    /// last one lands and sync runs back-to-back forever.
    /// `None` until the first drain completes: with no previous drain there is
    /// nothing to cool down from, so the first one runs immediately.
    last_drain_end: std::rc::Rc<std::cell::Cell<Option<f64>>>,
    /// Set once the worker is being replaced. A dying worker must not start
    /// new sync work: the SW spec keeps it alive until every `waitUntil`
    /// settles and every fetch completes, so a drain scheduled (or a loop
    /// tick fired) after `updatefound` pins the outgoing worker in `waiting`
    /// — which is why it "won't go away".
    stopped: std::rc::Rc<std::cell::Cell<bool>>,
    /// Whether any window client was visible at the last check. Hidden
    /// pages hold drains to [`SYNC_HIDDEN_INTERVAL_MS`] — a
    /// backgrounded tab keeps its SSE subscriptions (and the keepalive)
    /// alive, so subscription liveness alone can't tell "watching"
    /// from "abandoned overnight". The only thing that widens the gap:
    /// a visible tab always polls at the active cadence.
    visible: std::rc::Rc<std::cell::Cell<bool>>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Default for SyncScheduler {
    fn default() -> Self {
        Self {
            generation: Default::default(),
            in_flight: Default::default(),
            loading: Default::default(),
            last_request_at: Default::default(),
            pending_since: Default::default(),
            cause: Default::default(),
            last_drain_end: Default::default(),
            stopped: Default::default(),
            visible: std::rc::Rc::new(std::cell::Cell::new(true)),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl SyncScheduler {
    /// Bump the generation and return the new value — the caller's ticket. The
    /// caller drains only if this ticket is still current after the debounce (or
    /// the max-wait cap has elapsed). `now` starts the burst clock if this is
    /// the first request since the last drain.
    fn next(&self, now: f64) -> u64 {
        if self.pending_since.get().is_none() {
            self.pending_since.set(Some(now));
        }
        let next = self.generation.get().wrapping_add(1);
        self.generation.set(next);
        next
    }

    /// Mark a data-plane (`/api/*`) request as started — a drain defers while
    /// any are outstanding. Returns a guard that decrements on drop, so the
    /// count is released on any request outcome (success, error, or panic).
    fn enter_loading(&self, now: f64) -> LoadingGuard {
        self.loading.set(self.loading.get().saturating_add(1));
        self.last_request_at.set(now);
        LoadingGuard {
            loading: self.loading.clone(),
        }
    }

    /// Whether a page is actively loading: at least one data-plane request is
    /// in flight AND the most recent one started within [`SYNC_LOAD_DEFER_MS`].
    /// The time bound caps how long a single stuck request can defer sync — past
    /// it, the drain proceeds even with the counter non-zero.
    fn is_loading(&self, now: f64) -> bool {
        self.loading.get() > 0 && now - self.last_request_at.get() < SYNC_LOAD_DEFER_MS as f64
    }

    /// Whether a woken ticket should drain now. True when no drain is already
    /// running AND no page is actively loading AND either the ticket is still
    /// the latest (normal trailing edge) or the current burst has been pending
    /// past [`SYNC_MAX_WAIT_MS`] (the cap, so continuous traffic can't starve
    /// the drain).
    ///
    /// `dirty` is the sync queue's pending-local-work reading — see
    /// [`may_drain`](Self::may_drain).
    fn should_drain(&self, ticket: u64, now: f64, dirty: bool) -> bool {
        if !self.may_drain(now, dirty) {
            return false;
        }
        let is_latest = self.generation.get() == ticket;
        let capped = self
            .pending_since
            .get()
            .is_some_and(|since| now - since >= SYNC_MAX_WAIT_MS as f64);
        is_latest || capped
    }

    /// The gate EVERY drain entrypoint passes through — the debounced
    /// per-fetch drain, the self-scheduled loop, and Background Sync's
    /// `onsync`. Previously only the per-fetch path consulted `in_flight`,
    /// so a loop tick could start a second drain on top of a running one and
    /// they would contend for the SW thread and the branch locks.
    ///
    /// Refuses when: the worker is being replaced (a dying worker must start
    /// no new work — see `stopped`), a drain is already running, a page is
    /// actively loading, or the previous drain finished less than the quiet
    /// interval ago (see [`Self::quiet_interval`]).
    ///
    /// `dirty` says whether the sync queue holds un-pushed local commits.
    /// It is passed in rather than read here because the queue lives on
    /// `AppState`, which the scheduler has no handle to — and taking it as
    /// an argument makes the compiler force EVERY drain entrypoint to supply
    /// it, so no path can quietly skip the bypass. It also keeps the gate a
    /// pure function of its inputs, so the unit tests below need no app state.
    fn may_drain(&self, now: f64, dirty: bool) -> bool {
        if self.stopped.get() {
            return false;
        }
        if self.in_flight.get() {
            return false;
        }
        // Yield to an actively-loading page — a boot burst (this tab or another)
        // finishes before sync competes for the SW thread and branch locks.
        if self.is_loading(now) {
            return false;
        }
        // Quiet period measured from the LAST DRAIN'S COMPLETION, not from the
        // request that triggered this one. Without it, a drain that outlasts
        // the loop interval (easy on a slow link) is followed immediately by
        // the next, and sync runs continuously.
        //
        // The floor is SYNC_COOLDOWN_MS; a hidden page raises it (see
        // quiet_interval) so a backgrounded tab stops paying the active
        // cadence. Every drain entrypoint passes through here — including
        // the drains the page's keepalive fetches schedule — so the quiet
        // interval binds them all.
        //
        // Un-pushed local commits bypass the quiet interval entirely and get
        // the active cadence: the user edits, then switches tabs, and without
        // this their last edit sits unpushed for a minute while collaborators
        // see nothing. Durable locally, so it is latency, not loss — but it
        // is a minute of it. The bypass costs nothing when idle, since the
        // dirty set is empty then.
        let quiet = if dirty {
            SYNC_COOLDOWN_MS as f64
        } else {
            (SYNC_COOLDOWN_MS as f64).max(self.quiet_interval())
        };
        let allowed = self
            .last_drain_end
            .get()
            .is_none_or(|end| now - end >= quiet);
        // Log only refusals the QUIET INTERVAL caused — not the plain-cooldown
        // ones, which happen constantly on an active page and would flood the
        // console. This is the one line that makes the live check conclusive:
        // if the hidden-tab path is a no-op (say `any_client_visible` reads
        // visible on every browser because the `WindowClient` downcast fails),
        // it never prints, and the only other symptom is "the request count
        // didn't move".
        if !allowed && quiet > SYNC_COOLDOWN_MS as f64 {
            log!(
                "sync drain held off: quiet={quiet}ms visible={} dirty={dirty}",
                self.visible.get()
            );
        }
        allowed
    }

    /// Mark a drain as started: take the `in_flight` guard and clear the burst
    /// clock so the max-wait cap restarts from the next request.
    fn begin_drain(&self) {
        self.in_flight.set(true);
        self.pending_since.set(None);
    }

    /// Release the `in_flight` guard once a drain finishes, stamping `now` as
    /// the completion time so the cooldown measures from here. The clock is
    /// passed in rather than read internally so the gate is testable against a
    /// synthetic one.
    fn end_drain(&self, now: f64) {
        self.in_flight.set(false);
        self.last_drain_end.set(Some(now));
    }

    /// Stop all sync work: the worker is being replaced. Idempotent.
    fn stop(&self) {
        self.stopped.set(true);
    }

    /// Allow sync work again: this worker is the one now serving, so it is not
    /// retiring after all. Called from `onactivate`. Idempotent.
    ///
    /// Without this, [`stop`](Self::stop) is a one-way latch: a worker that
    /// stopped but was never actually replaced (a failed install, an update
    /// that never activates) refuses every drain for the rest of its life.
    fn resume(&self) {
        self.stopped.set(false);
    }

    /// Whether the worker has been told to stop.
    fn stopped(&self) -> bool {
        self.stopped.get()
    }

    /// Record the burst-opening request: the first call after a drain wins,
    /// later requests in the same burst are riders.
    fn note_cause(&self, cause: impl FnOnce() -> String) {
        let mut slot = self.cause.borrow_mut();
        if slot.is_none() {
            *slot = Some(cause());
        }
    }

    /// Take the recorded cause of the current burst.
    fn take_cause(&self) -> Option<String> {
        self.cause.borrow_mut().take()
    }

    /// The enforced gap between drain completions, from visibility alone.
    /// Zero while a page is visible — [`SYNC_COOLDOWN_MS`] stays the floor,
    /// so a visible tab always polls at the active cadence, however idle it
    /// looks. A visible-but-idle reader must never lag a collaborator.
    fn quiet_interval(&self) -> f64 {
        if !self.visible.get() {
            return SYNC_HIDDEN_INTERVAL_MS as f64;
        }
        0.0
    }

    /// Update the visibility reading — the sole input to
    /// [`quiet_interval`](Self::quiet_interval).
    fn set_visible(&self, visible: bool) {
        self.visible.set(visible);
    }
}

/// Decrements the scheduler's in-flight data-plane count on drop, so a request
/// releases its "loading" hold on the sync drain regardless of how it finishes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
struct LoadingGuard {
    loading: std::rc::Rc<std::cell::Cell<u32>>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl Drop for LoadingGuard {
    fn drop(&mut self) {
        self.loading.set(self.loading.get().saturating_sub(1));
    }
}

/// How long an in-flight data-plane request may defer the sync drain. Past this,
/// a stuck or slow request no longer holds sync off — the drain proceeds even
/// with requests still counted as in flight. Comfortably longer than a normal
/// page load (sub-500ms measured) so a real boot always completes first, but
/// short enough that a genuinely hung request doesn't stall sync for long.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_LOAD_DEFER_MS: i32 = 2_000;

/// Quiet window before a request's scheduled sync drain fires. A burst of boot
/// queries collapses into one drain at the trailing edge. Kept short so a
/// reactive pull (and the `<tonk-host>` idle poll every ~2× this) feels
/// near-immediate; it is the real rate-limiter on drain frequency.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_DEBOUNCE_MS: i32 = 500;

/// Max-wait cap on the trailing-edge debounce: however continuous the request
/// traffic, a drain fires at least this often. Without it, a tab making
/// requests faster than [`SYNC_DEBOUNCE_MS`] resets the window every time and
/// the drain never runs. Measured from the first request after the last drain.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_MAX_WAIT_MS: i32 = 3_000;

/// Minimum quiet period between the END of one sync drain and the start of
/// the next. The debounce measures from the triggering request, which says
/// nothing about how long the previous drain ran — on a slow link a drain can
/// outlast the loop's interval, and without a completion-relative gap the next
/// one starts the instant the last lands, so sync runs continuously and starves
/// the queries it shares the single SW thread with.
///
/// Deliberately smaller than [`SYNC_LOOP_MS`] so the loop's interval, not this,
/// is what sets the idle pull cadence: at cooldown >= interval every other tick
/// lands inside the quiet period and is refused, silently doubling the latency.
/// It stays non-zero because the starvation it prevents is real — it just needs
/// to be a gap, not a rate limit. A drain costs ~40ms locally, so this leaves
/// the thread free the overwhelming majority of the time.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_COOLDOWN_MS: i32 = 500;

/// Enforced drain gap while no window client is visible. A hidden tab
/// still keepalives and holds subscriptions, so without this it pays
/// the active cadence all night for changes nobody is watching.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_HIDDEN_INTERVAL_MS: i32 = 60_000;

/// The main Tonk service worker that handles browser fetch events.
///
/// This struct bridges the browser's service worker API with an Axum router,
/// allowing HTTP-like request handling in a Wasm context.
#[wasm_bindgen]
pub struct TonkServiceWorker {
    router: Arc<Mutex<Router>>,
    /// Shared application state. The router owns it too, but we
    /// keep a handle here so [`Self::on_fetch`] can consult the
    /// guest-binding map *before* dispatching — that's how we
    /// decide whether a request should be routed through axum
    /// (with optional path rewriting) or passed through to the
    /// network.
    state: AppState,
    /// Handle to the language-server hub. Used by [`Self::onupdatefound`]
    /// to release in-flight SSE responses when a newer worker
    /// version begins installing — without this the active worker
    /// can't be replaced because long-lived fetches keep it alive.
    lsp: Arc<LspHub>,
    /// Trailing-edge debounce for the background sync drain every fetch
    /// schedules on `event.waitUntil`. See [`SyncScheduler`]. Wasm-only: the
    /// drain runs on the service-worker event loop, which native builds (tests)
    /// don't have.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    sync_scheduler: SyncScheduler,
    /// Whether the self-scheduled sync loop is running. The SW owns the
    /// sync cadence (the page no longer polls): while any branch holds a
    /// live subscriber, a loop drains every [`SYNC_LOOP_MS`]; it stops when
    /// the page goes quiet or connectivity drops, and any fetch (or the
    /// `online` event) restarts it.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    sync_loop: std::rc::Rc<std::cell::Cell<bool>>,
}

#[wasm_bindgen]
impl TonkServiceWorker {
    /// Creates a new service worker instance.
    ///
    /// Initializes the user profile and operator — no repositories
    /// are opened or created here. Repositories are created
    /// on-demand via `PUT /api/repository/{name}`; subsequent
    /// requests against routes that expect the repository to exist
    /// will 404 until that happens.
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    pub async fn new() -> Result<Self, JsError> {
        log!("Tonk worker initializing...");

        // Patch IDB versionchange handling before any IDB operations.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        crate::patch_idb_versionchange();

        // 1. Create storage backend
        let storage = Storage::<DefaultSpace>::default();

        // 2. Open or create profile
        let profile = Profile::open(PROFILE_NAME)
            .perform(&storage)
            .await
            .map_err(|e| JsError::new(&format!("Failed to open profile: {}", e)))?;
        log!("Profile DID: {}", profile.did());

        // 3. Derive operator with full capabilities
        let operator = profile
            .derive(b"worker")
            .allow(Subject::any())
            .build(storage)
            .await
            .map_err(|e| JsError::new(&format!("Failed to build operator: {}", e)))?;

        // 4. Build state and bootstrap the profile repo's meta
        // branch. Idempotent — safe to run on every worker boot.
        let reactor = crate::Reactor::new(profile.clone());
        let state = TonkState {
            profile,
            operator,
            profile_name: PROFILE_NAME.to_string(),
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            commands: crate::router::command_registry(),
            sync_queue: Default::default(),
            clients: Default::default(),
        };
        bootstrap_profile(&state)
            .await
            .map_err(|e| JsError::new(&format!("Failed to bootstrap profile meta: {}", e)))?;

        // 5. Wrap state in the router. `api_router_with_state`
        // returns the LSP hub *and* a cloneable `AppState` handle:
        // the worker keeps the latter so `on_fetch` can read the
        // guest-binding map without going through the router.
        let (router, state, lsp) = api_router_with_state(state);
        let router = Arc::new(Mutex::new(router));

        Ok(Self {
            router,
            state,
            lsp,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            sync_scheduler: SyncScheduler::default(),
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            sync_loop: std::rc::Rc::new(std::cell::Cell::new(false)),
        })
    }

    /// Hook the SW's `updatefound` event from JavaScript.
    ///
    /// When the registration sees a newer worker entering the
    /// `installing` state, this active worker is on its way out.
    /// We use the moment to close every long-lived stream we're
    /// serving — `/api/lsp/events` SSE plus every `/query` SSE
    /// subscription — so the in-flight fetch events settle and the
    /// new worker can activate.
    ///
    /// Without this the SW spec keeps the active worker alive
    /// while any of its fetches are open, so a freshly-installed
    /// worker would sit in `waiting` until every browsing context
    /// hosting the page closed.
    #[wasm_bindgen(js_name = "onupdatefound")]
    pub fn on_update_found(&self) -> Promise {
        log!("Update found — releasing in-flight streams");
        // Stop all sync work FIRST. The spec keeps this worker alive until
        // every in-flight fetch and every `waitUntil` promise settles; a drain
        // scheduled on a fetch's `waitUntil`, or the self-scheduled loop's next
        // tick, would keep re-arming that condition and pin the outgoing worker
        // in `waiting` indefinitely — the "SW won't go away" symptom.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        {
            self.sync_scheduler.stop();
            self.sync_loop.set(false);
        }
        let lsp = self.lsp.clone();
        let state = self.state.clone();
        future_to_promise(async move {
            lsp.shutdown().await;
            // Also drain every query subscription. Each carries an
            // `mpsc::Sender` whose receiver drives an SSE response
            // body; dropping the sender ends the body so the fetch
            // settles.
            let tonk = state.read().await;
            tonk.reactor.shutdown();
            log!("Streams are released");
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Called from the JS shim's `self.onactivate` after
    /// `clients.claim()` has resolved. Drops every shell cache
    /// from older SW versions so the first fetch under the new
    /// worker doesn't race against stale entries.
    #[wasm_bindgen(js_name = "onactivate")]
    pub fn on_activate(&self) -> Promise {
        // A worker that is activating is the one now serving the page, so it is
        // by definition not retiring: clear the sync stop-flag. Makes the latch
        // self-healing even if a worker stopped itself and was then never
        // replaced — whoever ends up serving syncs.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        self.sync_scheduler.resume();

        future_to_promise(async move {
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            {
                if let Err(err) = crate::cache::purge_old_caches().await {
                    log!("purge_old_caches failed: {:?}", err);
                }
            }
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Handles incoming fetch events from the browser.
    ///
    /// Called from the JS shim's `self.onfetch` listener for *every*
    /// request the service worker sees. The Rust side decides
    /// whether to handle the request itself (via the axum router)
    /// or pass it through to the network. The shim never
    /// inspects the URL — all routing policy lives here.
    ///
    /// Decision rule (see [`route_for`]):
    /// - If the request's path starts with `/api/`, route through
    ///   the axum router unchanged.
    /// - If the initiating client is a registered guest iframe,
    ///   rewrite the path to live under
    ///   `/api/repository/{repo}/branch/{branch}` and route
    ///   through the axum router. The iframe sees its branch as
    ///   its virtual root.
    /// - Otherwise, pass through to the network via `sw_fetch`.
    ///
    /// Navigation requests that pass through and 404 are retried
    /// against `/index.html` so the client-side SPA router can
    /// match unknown paths.
    #[wasm_bindgen(js_name = "onfetch")]
    pub fn on_fetch(&self, event: FetchEvent) -> Promise {
        let request = event.request();
        let client_id = event.client_id().unwrap_or_default();
        let resulting_client_id = event_resulting_client_id(&event);

        // Prefer `clientId` (subresource fetches) over
        // `resultingClientId` (navigations). Exactly one is
        // populated for any fetch a service worker sees.
        let effective_client_id = if !client_id.is_empty() {
            client_id
        } else {
            resulting_client_id
        };

        let url = request.url();
        let is_navigation = request.mode() == web_sys::RequestMode::Navigate;

        let path = url::Url::parse(&url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();

        let router = self.router.clone();
        let state = self.state.clone();

        // Schedule a debounced background sync drain on this event's lifetime.
        // Every request (not just commits) does this: the page's normal traffic
        // IS the sync heartbeat. The drain pushes repos with un-pushed commits
        // and pulls every open repo, so upstream changes arrive without the page
        // committing or poking a dedicated endpoint. `wait_until` keeps the SW
        // alive through the debounce window so the drain actually runs. A burst
        // of boot queries collapses into one trailing-edge drain via the
        // generation ticket — only the last request still matches and drains.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        schedule_sync_drain(&event, &self.sync_scheduler, &self.state);
        // Any traffic (re)starts the SW-owned sync loop — the page no
        // longer polls, so this is what keeps an idle-but-subscribed tab
        // pulling upstream changes.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        self.ensure_sync_loop();

        // Count data-plane (`/api/*`) requests as in-flight for the duration of
        // this fetch: they contend with the sync drain for the SW thread and the
        // reactor's branch locks, so the drain holds off while any are running.
        // A page actively booting (this tab or another) thus finishes before
        // sync does network work. Static-asset fetches don't count — they never
        // touch the reactor. The guard rides into the future below and drops when
        // the request completes.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let loading_guard = path
            .starts_with("/api/")
            .then(|| self.sync_scheduler.enter_loading(js_sys::Date::now()));

        future_to_promise(async move {
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            let _loading_guard = loading_guard;
            // Opportunistic cleanup of stale bridge sessions and view
            // bindings. Cheap enough to run on every fetch.
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            crate::router::bridge::sweep_stale_clients(&state).await;

            match route_for(&path, &effective_client_id, &state).await {
                Route::Handle { rewritten_path } => {
                    handle_via_router(router, request, effective_client_id, rewritten_path).await
                }
                Route::Passthrough => passthrough(request, is_navigation).await,
                Route::Reject => reject_404(),
            }
        })
    }

    /// Handles `message` events from view clients. Routes the
    /// initial `hello` envelope (and future query/subscribe/evaluate
    /// envelopes) to the bridge module.
    ///
    /// Wired into the SW global by the same JS bootstrap that sets
    /// `self.onfetch`.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[wasm_bindgen(js_name = "onmessage")]
    pub fn on_message(&self, event: web_sys::ExtendableMessageEvent) -> Promise {
        let state = self.state.clone();
        let source_client_id = event_source_client_id(&event);
        let data = event.data();
        let ports = event.ports();

        future_to_promise(async move {
            let envelope: serde_json::Value = match serde_wasm_bindgen::from_value(data) {
                Ok(v) => v,
                Err(e) => {
                    log!("on_message: malformed envelope: {e:?}");
                    return Ok(JsValue::UNDEFINED);
                }
            };

            let Some(client_id) = source_client_id else {
                log!("on_message: envelope has no source client id");
                return Ok(JsValue::UNDEFINED);
            };

            crate::router::bridge::handle_message(state, ClientId(client_id), envelope, ports)
                .await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Runs a durable background sync, invoked from the Background Sync API
    /// `onsync` event — the tab-closed backstop for the in-fetch drain.
    ///
    /// The SW owns the sync work-queue (repos with un-pushed commits) and knows
    /// every open repo, so the event needs no per-repo identity: the tag is a
    /// single bare `"sync"`, ignored here. This funnels to the same
    /// [`drain_sync`](crate::router::drain_sync) as `POST /api/sync` and the
    /// per-fetch `event.waitUntil` drain — push the dirty set, pull every open
    /// repo.
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to `undefined` once the drain
    /// completes.
    pub fn sync(&self, tag: String) -> Promise {
        let state = self.state.clone();

        // Same gate as every other drain entrypoint: never overlap a running
        // drain, never run on a worker that is being replaced, and honor the
        // completion-relative cooldown.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        let scheduler = self.sync_scheduler.clone();
        future_to_promise(async move {
            log!("Background sync triggered ({tag:?})");
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            {
                if !scheduler.may_drain(js_sys::Date::now(), has_pending_local_work(&state).await) {
                    return Ok(JsValue::UNDEFINED);
                }
                scheduler.begin_drain();
                crate::router::drain_sync(&state).await;
                scheduler.end_drain(js_sys::Date::now());
            }
            #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
            {
                // No scheduler on this build target.
                crate::router::drain_sync(&state).await;
            }
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Connectivity changed. Re-read `navigator.onLine` (reliable in the SW
    /// scope) and reconcile: offline stamps `sync:offline` on every open repo
    /// so the chips/discs reflect the disconnect; online runs a drain and
    /// restarts the self-scheduled sync loop the offline transition stopped.
    ///
    /// Fired both by the SW's own `offline`/`online` events and by a
    /// `{type:"connectivity"}` nudge from the active page (whose events fire
    /// even when the SW's don't). Either way the decision is made from
    /// `navigator.onLine`, not from who fired — a flapping transition settles
    /// on the current reading.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[wasm_bindgen(js_name = "onconnectivity")]
    pub fn on_connectivity(&self) -> Promise {
        let offline = offline();
        if !offline {
            // Restart the self-scheduled loop the offline transition stopped.
            self.ensure_sync_loop();
        }
        let state = self.state.clone();
        let scheduler = self.sync_scheduler.clone();
        future_to_promise(async move {
            if offline {
                crate::router::mark_offline(&state).await;
            } else if scheduler.may_drain(js_sys::Date::now(), has_pending_local_work(&state).await)
            {
                scheduler.begin_drain();
                crate::router::drain_sync(&state).await;
                scheduler.end_drain(js_sys::Date::now());
            }
            Ok(JsValue::UNDEFINED)
        })
    }

    /// A page became visible: restore the active cadence and reconcile
    /// immediately, instead of waiting out the hidden interval.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[wasm_bindgen(js_name = "onvisibility")]
    pub fn on_visibility(&self) -> Promise {
        self.sync_scheduler.set_visible(true);
        self.ensure_sync_loop();
        let state = self.state.clone();
        let scheduler = self.sync_scheduler.clone();
        future_to_promise(async move {
            if !offline()
                && scheduler.may_drain(js_sys::Date::now(), has_pending_local_work(&state).await)
            {
                scheduler.begin_drain();
                crate::router::drain_sync(&state).await;
                scheduler.end_drain(js_sys::Date::now());
            }
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Start the self-scheduled sync loop if it isn't running. The SW owns
    /// the sync cadence: while any cached branch holds a live subscriber
    /// (an open SSE keeps the SW alive, so the timer chain survives), the
    /// loop drains every [`SYNC_LOOP_MS`]. It stops when the page goes
    /// quiet — no subscribers means nothing is watching, and stopping lets
    /// the browser reclaim the worker — or when connectivity drops (after
    /// stamping `sync:offline`); any fetch or the `online` event restarts
    /// it. This replaces the page-side `POST /api/sync` heartbeat.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn ensure_sync_loop(&self) {
        // A worker being replaced starts no new work — its loop would keep
        // waking up and pinning it in `waiting`.
        if self.sync_scheduler.stopped() || self.sync_loop.get() {
            return;
        }
        self.sync_loop.set(true);
        let running = self.sync_loop.clone();
        let state = self.state.clone();
        let scheduler = self.sync_scheduler.clone();
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                let _ = crate::sleep(web_time::Duration::from_millis(SYNC_LOOP_MS)).await;
                // The worker is being replaced (or `onupdatefound` cleared the
                // running flag): exit so this worker can be released.
                if scheduler.stopped() || !running.get() {
                    break;
                }
                if offline() {
                    // Reflect the disconnect, then stop — `ononline`
                    // restarts the loop and reconciles immediately.
                    crate::router::mark_offline(&state).await;
                    break;
                }
                if !has_live_subscribers(&state).await {
                    break;
                }
                if !has_syncable_repo(&state).await {
                    // Every open space is paused (or none is open): park.
                    // No wake-up plumbing needed — a resume arrives as a
                    // transact, and every fetch restarts this loop.
                    log!("sync loop parked: every open space is paused");
                    break;
                }
                // Through the SAME gate as the per-fetch drain: a loop tick used
                // to call `drain_sync` directly, so it could start a second
                // drain on top of one already running (they only consulted
                // `in_flight` on the per-fetch path) and the two then contended
                // for the single SW thread and the branch locks. The gate also
                // enforces the cooldown, so a drain that outlasts this interval
                // is not immediately followed by another.
                scheduler.set_visible(any_client_visible().await);
                if !scheduler.may_drain(js_sys::Date::now(), has_pending_local_work(&state).await) {
                    continue;
                }
                scheduler.begin_drain();
                crate::router::drain_sync(&state).await;
                scheduler.end_drain(js_sys::Date::now());
            }
            running.set(false);
        });
    }
}

/// Interval between self-scheduled sync drains while subscriptions are live.
///
/// This is the ONLY thing that pulls on an idle page: with no traffic there is
/// no per-fetch drain to ride, so it sets the worst-case latency for seeing
/// another device's change. An active page already syncs within
/// [`SYNC_DEBOUNCE_MS`] of its own requests.
///
/// [`SYNC_COOLDOWN_MS`] is the real floor on drain frequency (measured from the
/// last drain's *completion*, so a slow drain can't be followed immediately by
/// another), which is why this can sit at the same value without sync running
/// continuously: a tick that arrives inside the cooldown is refused, and the
/// next one picks it up. A no-op pull costs one round-trip per open repo
/// (~40ms locally), so ticking this often would be cheap relative to the
/// latency it buys — but `may_drain`'s quiet-interval gate (see
/// [`SyncScheduler::quiet_interval`]) widens the real gap well past this tick
/// rate once the page is hidden, so a backgrounded tab settles onto a far
/// coarser cadence than this constant alone implies. A visible tab always
/// gets this cadence.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const SYNC_LOOP_MS: u64 = 2_000;

/// Whether any open repository still has auto-sync enabled on its content
/// branch. All-paused parks the self-scheduled loop — the drain would
/// sweep the list and skip every entry anyway (the per-repo
/// `is_sync_enabled` gate), so ticking is pure churn until a resume.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn has_syncable_repo(state: &AppState) -> bool {
    let tonk = state.read().await;
    let repos: Vec<String> = tonk.reactor.repos().read().keys().cloned().collect();
    for repo in repos {
        if crate::router::is_sync_enabled(&tonk, &repo, "main").await {
            return true;
        }
    }
    false
}

/// Whether any cached branch — named repositories or the profile — holds a
/// live subscriber. The signal that a page is watching, so the SW should
/// keep pulling upstream changes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn has_live_subscribers(state: &AppState) -> bool {
    let tonk = state.read().await;
    {
        let repos = tonk.reactor.repos().read();
        for repo in repos.values() {
            for branch in repo.branches().read().values() {
                if !branch.subscriptions().lock().is_empty() {
                    return true;
                }
            }
        }
    }
    if let Some(repo) = tonk.reactor.profile_repo_state() {
        for branch in repo.branches().read().values() {
            if !branch.subscriptions().lock().is_empty() {
                return true;
            }
        }
    }
    false
}

/// Whether the sync queue holds un-pushed local commits.
///
/// The bypass input to [`SyncScheduler::may_drain`]: pending local work
/// always gets the active cadence, even on a hidden page. Every drain
/// entrypoint reads it through this one helper, so there is a single
/// definition of "pending" for the gate to honor.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn has_pending_local_work(state: &AppState) -> bool {
    state.read().await.sync_queue.has_dirty()
}

/// Whether any window client of this SW is currently visible.
/// `clients.matchAll()` defaults to window clients. A failed call reads
/// as visible so a Clients API hiccup can never silently stall sync.
///
/// An EMPTY client list is deliberately the opposite: it reads as hidden,
/// because that is what it means in the steady state (every tab closed).
/// `matchAll()` also defaults to `includeUncontrolled: false`, so during
/// the claim window — this worker activated in a previous session and the
/// current document is not yet controlled — it returns `[]` for a page
/// that plainly is visible. That misread self-heals within one loop tick,
/// once `clients.claim()` lands, and until then the page's own traffic is
/// driving drains anyway. Widening the query to uncontrolled clients would
/// count windows this worker does not serve, which is worse.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn any_client_visible() -> bool {
    use wasm_bindgen::JsCast;
    let global: web_sys::ServiceWorkerGlobalScope = js_sys::global().unchecked_into();
    let Ok(clients) = wasm_bindgen_futures::JsFuture::from(global.clients().match_all()).await
    else {
        return true;
    };
    let clients: js_sys::Array = clients.unchecked_into();
    clients.iter().any(|c| {
        // A client that fails to downcast to `WindowClient` reads as
        // visible too, per the same "never stall sync on an API
        // surprise" rule as the outer `match_all` failure above. In
        // practice this never fires: matchAll() with no options
        // defaults to window clients. Logged because if it DOES fire
        // systematically — a browser whose client objects don't satisfy
        // the downcast — every page reads visible and the hidden-tab
        // cadence is a silent no-op with no other symptom.
        match c.dyn_into::<web_sys::WindowClient>() {
            Ok(w) => w.visibility_state() == web_sys::VisibilityState::Visible,
            Err(_) => {
                log!("any_client_visible: client is not a WindowClient, reading as visible");
                true
            }
        }
    })
}

/// Schedule a debounced background sync drain on `event`'s lifetime.
///
/// Bumps the scheduler's generation, captures the ticket, and hands
/// `event.wait_until` a promise that sleeps the debounce window and then drains
/// if the ticket is still current (no newer request superseded it) OR the
/// max-wait cap has elapsed, and no drain is already running. `wait_until` keeps
/// the SW alive through the sleep, so the trailing-edge drain actually runs even
/// when the originating request finished long before. Failures are swallowed —
/// a background drain never rejects the fetch.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn schedule_sync_drain(event: &FetchEvent, scheduler: &SyncScheduler, state: &AppState) {
    use wasm_bindgen::JsCast;

    let ticket = scheduler.next(js_sys::Date::now());
    // Record the burst-opener (method + path + query — the query carries the
    // heartbeat's `?why=`), so the coalesced drain can log what initiated it.
    scheduler.note_cause(|| {
        let request = event.request();
        let raw = request.url();
        let path = url::Url::parse(&raw)
            .map(|u| match u.query() {
                Some(q) => format!("{}?{}", u.path(), q),
                None => u.path().to_string(),
            })
            .unwrap_or(raw);
        format!("{} {}", request.method(), path)
    });
    let scheduler = scheduler.clone();
    let state = state.clone();

    let promise = future_to_promise(async move {
        // Quiet window: a burst of requests collapses into one drain.
        let _ = crate::sleep(web_time::Duration::from_millis(SYNC_DEBOUNCE_MS as u64)).await;

        scheduler.set_visible(any_client_visible().await);
        if !scheduler.should_drain(
            ticket,
            js_sys::Date::now(),
            has_pending_local_work(&state).await,
        ) {
            // A newer request superseded us (and the max-wait cap hasn't
            // elapsed), or a drain is already running.
            return Ok(JsValue::UNDEFINED);
        }
        // No upstream while offline: skip the network sweep, but stamp
        // `sync:offline` locally so the chip/disc reflect the disconnect
        // (skipping silently left them frozen on the last online status).
        // Traffic keeps scheduling, so the first drain after connectivity
        // returns proceeds normally (and the page's `online` listener polls
        // immediately).
        if offline() {
            scheduler.begin_drain();
            crate::router::mark_offline(&state).await;
            scheduler.end_drain(js_sys::Date::now());
            return Ok(JsValue::UNDEFINED);
        }
        scheduler.begin_drain();
        if let Some(cause) = scheduler.take_cause() {
            log!("sync drain, caused by: {cause}");
        }
        crate::router::drain_sync(&state).await;
        scheduler.end_drain(js_sys::Date::now());
        Ok(JsValue::UNDEFINED)
    });

    // `wait_until` lives on `ExtendableEvent`, the base of `FetchEvent`. Upcast
    // and extend the event's lifetime to cover the debounced drain.
    let extendable: &web_sys::ExtendableEvent = event.unchecked_ref();
    let _ = extendable.wait_until(&promise);
}

/// Whether the worker reports no network connectivity, read straight from
/// `navigator.onLine` in the service-worker scope (which does update under
/// DevTools offline emulation and reflects the real connection in the wild).
/// A failure to read the scope counts as online — a wrongly skipped drain is
/// worse than a failed fetch (which itself stamps `sync:offline`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn offline() -> bool {
    use wasm_bindgen::JsCast;
    js_sys::global()
        .dyn_into::<web_sys::WorkerGlobalScope>()
        .map(|scope| !scope.navigator().on_line())
        .unwrap_or(false)
}

/// Extracts the `source.id` string from an `ExtendableMessageEvent`.
///
/// The source of a service-worker message event is a `Client`; we
/// need its id so we can look up (or create) the `BridgeSession` in
/// the registry. Returns `None` if the source is absent or cannot be
/// cast to a `Client`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn event_source_client_id(event: &web_sys::ExtendableMessageEvent) -> Option<String> {
    use wasm_bindgen::JsCast;
    let source = event.source()?;
    let client: web_sys::Client = source.dyn_into().ok()?;
    Some(client.id())
}
