//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    LspHub, api_router,
    axum::{RequestConversion, ResponseConversion},
    bootstrap_profile_meta,
};
use axum::{Router, body::Body};
use dialog_capability::Subject;
use dialog_operator::{Operator, Profile};
use dialog_storage::provider::storage::Storage;
use js_sys::Promise;
use tokio::sync::Mutex;
use tonk_common::log;
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Request, Response};

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
        let state = TonkState {
            profile,
            operator,
            profile_name: PROFILE_NAME.to_string(),
        };
        bootstrap_profile_meta(&state, PROFILE_NAME)
            .await
            .map_err(|e| JsError::new(&format!("Failed to bootstrap profile meta: {}", e)))?;

        // 5. Wrap state in the router. `api_router` returns the LSP
        // hub alongside it so the worker can address it directly
        // (independent of the request-handling path) when the SW
        // lifecycle requires releasing in-flight streams.
        let (router, lsp) = api_router(state);
        let router = Arc::new(Mutex::new(router));

        Ok(Self { router, lsp })
    }

    /// Hook the SW's `updatefound` event from JavaScript.
    ///
    /// When the registration sees a newer worker entering the
    /// `installing` state, this active worker is on its way out.
    /// We use the moment to close every long-lived stream we're
    /// serving — chiefly `/api/lsp/events` SSE — so the in-flight
    /// fetch events settle and the new worker can activate.
    ///
    /// Without this the SW spec keeps the active worker alive
    /// while any of its fetches are open, so a freshly-installed
    /// worker would sit in `waiting` until every browsing context
    /// hosting the page closed.
    #[wasm_bindgen(js_name = "onupdatefound")]
    pub fn on_update_found(&self) -> Promise {
        log!("Update found — releasing in-flight streams");
        let lsp = self.lsp.clone();
        future_to_promise(async move {
            lsp.shutdown().await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// Handles incoming fetch events from the browser.
    ///
    /// Converts the browser's `Request` to an Axum request, processes it through
    /// the router, and converts the response back to a browser `Response`.
    ///
    /// # Parameters
    ///
    /// - `request`: The incoming browser fetch request
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to the response
    #[wasm_bindgen(js_name = "onfetch")]
    pub fn on_fetch(&self, request: Request) -> Promise {
        log!("Handling fetch!");

        let router = self.router.clone();
        let request: axum::http::Request<Body> =
            RequestConversion::from(request).try_into().unwrap_throw();

        future_to_promise(async move {
            let response = router
                .lock()
                .await
                .call(request)
                .await
                .expect_throw("Failed to handle API request");
            ResponseConversion::from(response)
                .try_into()
                .map(|value: Response| JsValue::from(value))
                .map_err(JsValue::from)
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
