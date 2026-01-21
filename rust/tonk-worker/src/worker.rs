//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    Identity, Workspace, api_router,
    axum::{RequestConversion, ResponseConversion},
    workspace::WorkspaceError,
};
use axum::{Router, body::Body};
use js_sys::Promise;
use tokio::sync::Mutex;
use tonk_common::log;
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Request, Response};

/// Application state containing the user's identity and active workspace.
pub struct TonkState {
    /// The user's persistent identity.
    pub identity: Arc<Identity>,
    /// The currently active workspace.
    pub workspace: Workspace,
}

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
    /// Initializes the user identity, workspace, and API router.
    ///
    /// On first run:
    /// - Creates a new random identity for the user
    /// - Creates a new space with a delegation granting the user ownership
    ///
    /// On subsequent runs:
    /// - Loads the existing identity from IndexedDB
    /// - Opens the default workspace
    ///
    /// The worker creates two keypairs:
    /// - **Space keypair**: Represents the space identity (from "public tonk space" passphrase)
    /// - **Operator keypair**: Used for signing operations (from "public tonk operator" passphrase)
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
        let identity = Identity::load_or_create()
            .await
            .expect_throw("Could not initialize identity");
        log!("User DID: {}", identity.did());

        // 2. Open default workspace, or create if none exists
        let workspace = match identity.open_workspace(None).await {
            Ok(ws) => ws,
            Err(WorkspaceError::NoDefaultSpace) => {
                log!("No default space, creating...");
                identity
                    .create_workspace()
                    .await
                    .expect_throw("Could not create workspace")
            }
            Err(e) => {
                return Err(JsError::new(&format!("Could not open workspace: {}", e)));
            }
        };
        log!("Space DID: {}", workspace.space_did());

        // 3. Build state and router
        let state = TonkState {
            identity: Arc::new(identity),
            workspace,
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
}
