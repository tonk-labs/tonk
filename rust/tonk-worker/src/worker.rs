//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    RepoIndex, api_router,
    axum::{RequestConversion, ResponseConversion},
};
use axum::{Router, body::Body};
use dialog_capability::Subject;
use dialog_repository::Operator;
use dialog_repository::profile::Profile;
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

/// Application state containing the profile, operator, and the index
/// of repos the profile has access to. The index is always present but
/// may be empty; see [`RepoIndex`] for its persistence behavior.
pub struct TonkState {
    /// The user's persistent profile.
    pub profile: Profile,
    /// The operator derived from the profile.
    pub operator: DefaultOperator,
    /// Cache of repo metadata; written on create/claim, read on list.
    pub repo_index: RepoIndex,
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
    /// Creates a new service worker instance. Called from the `activate()`
    /// export in tonk-ui's worker binary.
    ///
    /// Opens (or creates) the persistent profile, derives an operator with
    /// full capabilities, and restores the repo index from storage. No
    /// repos are auto-created — the UI drives create/claim explicitly.
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    pub async fn new() -> Result<Self, JsError> {
        log!("Tonk worker initializing...");

        // Patch IDB versionchange handling before any IDB operations.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        crate::patch_idb_versionchange();

        let storage = Storage::<DefaultSpace>::default();

        let profile = Profile::open("tonk")
            .perform(&storage)
            .await
            .map_err(|e| JsError::new(&format!("Failed to open profile: {}", e)))?;
        log!("Profile DID: {}", profile.did());

        let operator = profile
            .derive(b"worker")
            .allow(Subject::any())
            .build(storage)
            .await
            .map_err(|e| JsError::new(&format!("Failed to build operator: {}", e)))?;

        let repo_index = RepoIndex::restore().await;
        log!("Restored repo index ({} entries)", repo_index.list().len());

        let state = TonkState {
            profile,
            operator,
            repo_index,
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

    /// Background Sync API handler.
    ///
    /// Previously this dispatched to a hardcoded `/api/repository/home/...`
    /// route. Under the multi-repo model there is no implicit default, and
    /// sync needs to iterate the repo index — a follow-up concern. Until
    /// then the handler resolves without work so the `self.onsync` binding
    /// in `service_worker.js` keeps functioning instead of throwing.
    pub fn sync(&self) -> Promise {
        log!("Background sync event received (multi-repo sync iteration not yet wired)");
        future_to_promise(async move { Ok(JsValue::UNDEFINED) })
    }
}
