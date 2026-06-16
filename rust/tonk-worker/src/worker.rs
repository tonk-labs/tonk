//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    LspHub,
    axum::{RequestConversion, ResponseConversion},
    bootstrap_profile_meta,
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
    pub reactor: crate::TonkReactor,
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
    pub commands: crate::reactor::CommandRegistry,
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
}

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
        let reactor = crate::TonkReactor::new(profile.clone());
        let state = TonkState {
            profile,
            operator,
            profile_name: PROFILE_NAME.to_string(),
            reactor,
            view_bindings: Default::default(),
            bridges: Default::default(),
            commands: crate::router::command_registry(),
        };
        bootstrap_profile_meta(&state)
            .await
            .map_err(|e| JsError::new(&format!("Failed to bootstrap profile meta: {}", e)))?;

        // 5. Wrap state in the router. `api_router_with_state`
        // returns the LSP hub *and* a cloneable `AppState` handle:
        // the worker keeps the latter so `on_fetch` can read the
        // guest-binding map without going through the router.
        let (router, state, lsp) = api_router_with_state(state);
        let router = Arc::new(Mutex::new(router));

        Ok(Self { router, state, lsp })
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

        future_to_promise(async move {
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

    /// Runs a durable background sync for the repository named in
    /// `tag`, invoked from the Background Sync API `onsync` event.
    ///
    /// The event carries only a tag string, so the repo identity
    /// travels in it as `tonk-sync:{repo}`. The worker enumerates that
    /// repo's upstream branches and reconciles each through the
    /// `/sync` route — the same selection the in-page sweep makes.
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to `undefined` once every
    /// upstream branch has reconciled. A malformed or unrecognized tag
    /// resolves as a no-op. If a branch fails to land, the promise
    /// rejects so the user agent retries the sync with backoff.
    pub fn sync(&self, tag: String) -> Promise {
        let state = self.state.clone();

        future_to_promise(async move {
            let Some(repo) = crate::router::repo_from_sync_tag(&tag) else {
                log!("Background sync: ignoring unrecognized tag {tag:?}");
                return Ok(JsValue::UNDEFINED);
            };
            log!("Background sync triggered for repo '{repo}'");

            match crate::router::sync_repository(&state, repo).await {
                Ok(()) => Ok(JsValue::UNDEFINED),
                Err(e) => Err(JsValue::from_str(&e)),
            }
        })
    }
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
