//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    api_router,
    axum::{RequestConversion, ResponseConversion},
};
use axum::{Router, body::Body};
use dialog_capability::Subject;
use dialog_operator::{Operator, Profile};
use dialog_repository::RepositoryExt as _;
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

/// Application state containing the profile and operator.
pub struct TonkState {
    /// The user's persistent profile.
    pub profile: Profile,
    /// The operator derived from the profile.
    pub operator: DefaultOperator,
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
}

#[wasm_bindgen]
impl TonkServiceWorker {
    /// Creates a new service worker instance.
    ///
    /// Initializes the user profile, operator, default repository, and API router.
    ///
    /// On first run:
    /// - Creates a new profile identity
    /// - Derives an operator with full capabilities
    /// - Opens a default repository
    /// - Delegates repository access to the profile
    ///
    /// On subsequent runs:
    /// - Loads the existing profile from IndexedDB (WASM) or filesystem (native)
    /// - Opens the same default repository
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    /// Creates a new service worker instance. Called from the `activate()`
    /// export in tonk-ui's worker binary.
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

        // 4. Open default repository
        let repo = profile
            .repository("home")
            .open()
            .perform(&operator)
            .await
            .map_err(|e| JsError::new(&format!("Failed to open default repo: {}", e)))?;
        log!("Default repo DID: {}", repo.did());

        // 5. Delegate repo access to profile (if it's a signer credential)
        if let Some(access) = repo.try_access() {
            match access
                .claim(&repo)
                .delegate(profile.did())
                .perform(&operator)
                .await
            {
                Ok(chain) => {
                    if let Err(e) = profile.access().save(chain).perform(&operator).await {
                        log!("Warning: failed to save repo delegation: {}", e);
                    }
                }
                Err(e) => {
                    log!("Warning: failed to delegate repo to profile: {}", e);
                }
            }
        }

        // 6. Build state and router
        let state = TonkState { profile, operator };
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
