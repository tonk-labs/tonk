//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    api_router,
    axum::{RequestConversion, ResponseConversion},
    router::{AppState, ClientId},
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
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use web_sys::{FetchEvent, Request, Response};

// Global `self.fetch(...)` in the service-worker scope. Fetches
// issued from an SW bypass the SW's own `onfetch` listener (per
// spec), so this is how we pass through requests the Rust side
// chose not to handle.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = fetch)]
    fn sw_fetch(request: &Request) -> Promise;

    #[wasm_bindgen(js_name = fetch)]
    fn sw_fetch_str(url: &str) -> Promise;
}

/// Extract the `resultingClientId` property from a `FetchEvent`.
///
/// web-sys 0.3.85 exposes `FetchEvent.client_id()` but not
/// `resultingClientId`. For navigation requests `client_id` is
/// empty and `resultingClientId` holds the ID the new document
/// will have, so we need both. Read the property via reflection
/// rather than extending `FetchEvent` with a manual extern —
/// wasm-bindgen doesn't allow `impl`-style extension of foreign
/// types from outside their defining crate.
fn event_resulting_client_id(event: &FetchEvent) -> String {
    js_sys::Reflect::get(event, &JsValue::from_str("resultingClientId"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default()
}

/// Outcome of the routing decision made in `on_fetch`.
enum Route {
    /// Dispatch through the axum router. `rewritten_path` is
    /// `Some` only when the request originated from a registered
    /// guest iframe and its path needs to be re-rooted under the
    /// iframe's repository (e.g. `/` → `/api/repository/home`).
    Handle { rewritten_path: Option<String> },
    /// Pass the request through to the network.
    Passthrough,
}

/// Decides how the Rust side should route this request.
///
/// - Paths under `/api/` are always handled by the axum router as-is.
/// - Requests whose initiating client is a registered guest
///   iframe are *also* handled by the router, but with their
///   path rewritten to live under `/api/repository/{repo}`. That
///   's what gives the iframe a virtual root scoped to its
///   repository: the iframe fetches `/` and the router sees
///   `/api/repository/{repo}`, etc.
/// - Everything else falls through to the network.
async fn route_for(path: &str, client_id: &str, state: &AppState) -> Route {
    if path.starts_with("/api/") {
        return Route::Handle {
            rewritten_path: None,
        };
    }
    if client_id.is_empty() {
        return Route::Passthrough;
    }

    let binding = {
        let guests = state.read().await.guests.clone();
        let guard = guests.read().await;
        guard.get(&ClientId(client_id.to_string())).cloned()
    };

    let Some(binding) = binding else {
        return Route::Passthrough;
    };

    let rewritten = if path == "/" || path.is_empty() {
        format!("/api/repository/{}", binding.repo)
    } else {
        format!("/api/repository/{}{}", binding.repo, path)
    };
    Route::Handle {
        rewritten_path: Some(rewritten),
    }
}

/// Route the request through the axum router, apply response
/// headers (CORS, client-id echo), and convert back to a browser
/// `Response`.
///
/// If `rewritten_path` is `Some`, the request's URI is rewritten
/// to that path before axum gets it — that's how guest-iframe
/// requests get redirected into their repository's namespace
/// without axum's own routing layer ever seeing the un-scoped
/// URL.
async fn handle_via_router(
    router: Arc<Mutex<Router>>,
    browser_request: Request,
    client_id: String,
    rewritten_path: Option<String>,
) -> Result<JsValue, JsValue> {
    let mut request: axum::http::Request<Body> = RequestConversion::from(browser_request)
        .try_into()
        .map_err(JsValue::from)?;

    if let Some(new_path) = rewritten_path {
        // Preserve the query string; axum routes by path but
        // handlers parse query params, so losing them would drop
        // things like `?the=…&of=…` on claim/select requests.
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
        request
            .extensions_mut()
            .insert(ClientId(client_id.clone()));
    }

    log!("handle_via_router: acquiring router lock");
    let mut response = router
        .lock()
        .await
        .call(request)
        .await
        .expect_throw("Failed to handle API request");
    log!(
        "handle_via_router: router returned status={}",
        response.status()
    );

    let headers = response.headers_mut();

    if !client_id.is_empty()
        && let Ok(value) = HeaderValue::from_str(&client_id)
    {
        headers.insert(HeaderName::from_static("x-tonk-client-id"), value);
    }

    // CORS: sandboxed iframes have an opaque origin and send
    // `Origin: null`, so cross-origin rules kick in even though
    // the iframe and the service worker sit on the same URL
    // origin. Send permissive headers on every response — same-
    // origin callers ignore them and opaque-origin callers need
    // them to read the body. Credentials stay off: the only
    // valid value for `Access-Control-Allow-Credentials` is
    // `true` (the header is omitted otherwise), so we simply
    // don't emit it.
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
        HeaderValue::from_static("content-type, if-none-match"),
    );
    headers.insert(
        HeaderName::from_static("access-control-expose-headers"),
        HeaderValue::from_static("x-tonk-client-id"),
    );

    log!("handle_via_router: converting response");
    let result = ResponseConversion::from(response)
        .try_into()
        .map(|value: Response| JsValue::from(value))
        .map_err(JsValue::from);
    log!("handle_via_router: returning result");
    result
}

/// Pass the request through to the network by calling
/// `self.fetch(request)` from inside the service worker. Such
/// fetches bypass the SW's own `onfetch` handler (per spec), so
/// this really does hit the network.
///
/// For navigation requests that 404, retry against `/index.html`
/// so the client-side SPA router can match unknown paths.
async fn passthrough(request: Request, is_navigation: bool) -> Result<JsValue, JsValue> {
    let response: Response = JsFuture::from(sw_fetch(&request)).await?.dyn_into()?;

    if is_navigation && response.status() == 404 {
        let fallback: Response = JsFuture::from(sw_fetch_str("/index.html"))
            .await?
            .dyn_into()?;
        return Ok(JsValue::from(fallback));
    }

    Ok(JsValue::from(response))
}

/// Default storage type — IndexedDB on WASM, filesystem on native.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub type DefaultSpace = dialog_storage::provider::storage::WebSpace;

/// Default storage type — filesystem on native.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub type DefaultSpace = dialog_storage::provider::storage::NativeSpace;

/// Concrete operator type for the default storage backend.
pub type DefaultOperator = Operator<DefaultSpace>;

/// Application state containing the profile, operator, and SW-
/// plumbing bookkeeping (currently just the guest-iframe client
/// bindings).
pub struct TonkState {
    /// The user's persistent profile.
    pub profile: Profile,
    /// The operator derived from the profile.
    pub operator: DefaultOperator,
    /// Guest-iframe bindings keyed by service-worker Client ID.
    /// Behind its own interior lock so guest registration /
    /// lookup doesn't contend with profile/operator access.
    pub guests: crate::router::GuestBindings,
}

// SAFETY: Web browsers run Wasm in a single thread only. The interior types
// (Profile, Operator) contain `web_sys::CryptoKey` handles (via
// Ed25519SigningKey::WebCrypto) which are !Send/!Sync, but cross-thread access
// cannot occur in a single-threaded browser context.
#[cfg(target_arch = "wasm32")]
unsafe impl Send for TonkState {}
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for TonkState {}

/// The main Tonk service worker that handles browser fetch events.
///
/// This struct bridges the browser's service worker API with an Axum router,
/// allowing HTTP-like request handling in a Wasm context.
#[wasm_bindgen]
pub struct TonkServiceWorker {
    router: Arc<Mutex<Router>>,
    /// Shared application state. Also owned by the router's
    /// internal state, but we keep a handle here so `on_fetch`
    /// can consult the guest-binding map *before* dispatching —
    /// that's how we decide whether to route through axum or
    /// pass the request through to the network.
    state: AppState,
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
        let profile = Profile::open("tonk")
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

        // 4. Build state and router
        let tonk_state = TonkState {
            profile,
            operator,
            guests: Default::default(),
        };
        let (router, state) = api_router(tonk_state);
        let router = Arc::new(Mutex::new(router));

        Ok(Self { router, state })
    }

    /// Handles incoming fetch events from the browser.
    ///
    /// Called from the JS shim's `self.onfetch` listener for
    /// *every* request the service worker sees. Rust decides
    /// whether to handle the request itself (via the axum router)
    /// or pass it through to the network. The shim never
    /// inspects the URL — all routing policy lives here.
    ///
    /// Decision rule (see [`route_for`]):
    /// - If the request's path starts with `/api/`, route through
    ///   the axum router unchanged.
    /// - If the initiating client is a registered guest iframe,
    ///   rewrite the path to live under `/api/repository/{repo}`
    ///   and route through the axum router. This is what makes
    ///   the iframe see its repo as its virtual root: it fetches
    ///   `/` and we dispatch `/api/repository/{repo}`, it fetches
    ///   `/branch/main/...` and we dispatch
    ///   `/api/repository/{repo}/branch/main/...`.
    /// - Otherwise, pass through to the network via `sw_fetch`.
    ///
    /// Navigation requests that pass through and 404 are retried
    /// against `/index.html` so the client-side SPA router gets
    /// a chance at unknown routes.
    #[wasm_bindgen(js_name = "onfetch")]
    pub fn on_fetch(&self, event: FetchEvent) -> Promise {
        let request = event.request();
        let client_id = event.client_id().unwrap_or_default();
        let resulting_client_id = event_resulting_client_id(&event);

        // Prefer `clientId` (present on subresource requests) over
        // `resultingClientId` (present on navigations). Exactly one
        // is populated for any fetch a service worker sees.
        let effective_client_id = if !client_id.is_empty() {
            client_id
        } else {
            resulting_client_id
        };

        let url = request.url();
        let is_navigation = request.mode() == web_sys::RequestMode::Navigate;

        // Parse the URL path locally so we can route-decide
        // without touching Rust's http types yet.
        let path = url::Url::parse(&url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();

        let router = self.router.clone();
        let state = self.state.clone();

        future_to_promise(async move {
            log!(
                "onfetch path={} mode={:?} client={:?}",
                path,
                request.mode(),
                effective_client_id,
            );

            match route_for(&path, &effective_client_id, &state).await {
                Route::Handle { rewritten_path } => {
                    handle_via_router(
                        router,
                        request,
                        effective_client_id,
                        rewritten_path,
                    )
                    .await
                }
                Route::Passthrough => passthrough(request, is_navigation).await,
            }
        })
    }

    /// Performs a full sync operation (pull then push) with the upstream remote.
    ///
    /// This method dispatches to the `/api/repository/home/branch/main/sync`
    /// route internally, so the sync logic is not duplicated. It is intended to
    /// be called from the Background Sync API event or as a polyfill.
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to `undefined` on success, or
    /// rejects with an error if the sync failed.
    pub fn sync(&self) -> Promise {
        log!("Background sync triggered, dispatching to /api/repository/home/branch/main/sync");

        let router = self.router.clone();

        future_to_promise(async move {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/repository/home/branch/main/sync")
                .body(Body::empty())
                .expect_throw("Failed to build sync request");

            let response = router
                .lock()
                .await
                .call(request)
                .await
                .expect_throw("Failed to handle sync request");

            if response.status().is_success() {
                Ok(JsValue::UNDEFINED)
            } else {
                Err(JsValue::from_str(&format!(
                    "Sync failed with status: {}",
                    response.status()
                )))
            }
        })
    }
}
