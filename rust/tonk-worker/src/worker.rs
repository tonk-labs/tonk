//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    Identity, Session, api_router,
    axum::{RequestConversion, ResponseConversion},
};
use axum::{Router, body::Body};
use js_sys::Promise;
use tokio::sync::Mutex;
use tonk_common::log;
use tonk_space::Operator;
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Request, Response};

/// Application state containing the user's identity and active session.
pub struct TonkState {
    /// The user's persistent identity.
    pub identity: Arc<Identity>,
    /// The currently active session.
    pub session: Session,
}

// SAFETY: Web browsers run Wasm in a single thread only. The interior types
// (Operator, Space, Identity) contain `web_sys::CryptoKey` handles (via
// Ed25519SigningKey::WebCrypto) which are !Send/!Sync, but cross-thread access
// cannot occur in a single-threaded browser context. This follows the same
// pattern used for ServiceWorkerStorageBackend and KeyStore in this crate.
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
}

#[wasm_bindgen]
impl TonkServiceWorker {
    /// Creates a new service worker instance.
    ///
    /// Initializes the user identity, session, and API router.
    ///
    /// On first run:
    /// - Creates a new random identity for the user
    /// - Creates a new space with a delegation granting the user ownership
    ///
    /// On subsequent runs:
    /// - Loads the existing identity from IndexedDB
    /// - Opens the first known space (or creates one if none exist)
    ///
    /// The worker creates two keypairs:
    /// - **Space keypair**: Represents the space identity
    /// - **Operator keypair**: Used for signing operations
    ///
    /// A delegation is created from the space to the operator, granting
    /// full capabilities. This delegation is used when setting up upstream sync.
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    pub async fn new() -> Result<Self, JsError> {
        log!("Tonk worker initializing...");

        // 1. Load or create user identity
        let mut identity = Identity::load_or_create()
            .await
            .expect_throw("Could not initialize identity");
        log!("User DID: {}", identity.did());

        // 2. Get known spaces, or create if none exist
        let known_spaces = identity
            .account()
            .known_spaces()
            .await
            .expect_throw("Could not query known spaces");

        let session = if let Some(space_did) = known_spaces.first() {
            log!("Opening known space: {}", space_did);
            identity
                .open_session(space_did)
                .await
                .expect_throw("Could not open session")
        } else {
            log!("No known spaces, join publish shared space");
            let shared_space = Operator::from_passphrase("public tonk space").await;
            identity
                .join_session(shared_space)
                .await
                .expect_throw("Could not create session")
        };
        log!("Space DID: {}", session.space_did());

        // 3. Build state and router
        let state = TonkState {
            identity: Arc::new(identity),
            session,
        };
        let router = Arc::new(Mutex::new(api_router(state)));

        Ok(Self { router })
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
    /// This method dispatches to the `/api/sync` route internally, so the sync
    /// logic is not duplicated. It is intended to be called from the Background
    /// Sync API event or as a polyfill.
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to `undefined` on success, or
    /// rejects with an error if the sync failed.
    pub fn sync(&self) -> Promise {
        log!("Background sync triggered, dispatching to /api/sync");

        let router = self.router.clone();

        future_to_promise(async move {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/sync")
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
