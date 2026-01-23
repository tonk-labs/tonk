//! Service worker main implementation.
//!
//! This module defines all JavaScript-visible bindings for the Tonk service worker.

use std::sync::Arc;

use crate::{
    ServiceWorkerStorageBackend, api_router,
    axum::{RequestConversion, ResponseConversion},
};
use axum::{Router, body::Body};
use js_sys::Promise;
use tokio::sync::Mutex;
use tonk_common::log;
use tonk_space::DelegatedSubject;
use tonk_space::{Delegation, Ed25519Signer, Operator, Space};
use tower_service::Service;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Request, Response};

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
    /// Initializes the storage backend, space, and API router.
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

        // Generate space keypair - this determines the space's DID
        let space_keypair = Operator::from_passphrase("public tonk space").await;
        let space_did = space_keypair.did().to_string();

        // Generate operator keypair - this will sign operations
        let operator = Operator::from_passphrase("public tonk operator").await;

        log!(
            "Opening space: {} (operator: {})",
            space_did,
            operator.did()
        );

        // Create delegation from space to operator
        let delegation = Delegation::builder()
            .issuer(Ed25519Signer::from(&space_keypair))
            .audience(*operator.did())
            .subject(DelegatedSubject::Specific(*space_keypair.did()))
            .command(vec!["*".to_string()])
            .try_build()
            .expect_throw("Failed to build delegation");

        let delegation = Delegation::from(delegation);
        log!(
            "Created delegation: {} -> {}",
            space_keypair.did(),
            operator.did()
        );

        let backend = ServiceWorkerStorageBackend::new(&space_did).await;
        let space: Space<ServiceWorkerStorageBackend> = Space::open(space_did, &operator, backend)
            .await
            .expect_throw("Could not open space");

        let router = Arc::new(Mutex::new(api_router(space, operator, delegation)));

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
