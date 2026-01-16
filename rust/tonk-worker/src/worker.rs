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
use tonk_space::{Operator, Space};
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
    /// # Errors
    ///
    /// Returns a `JsError` if the service worker cannot be initialized.
    #[wasm_bindgen(constructor)]
    pub async fn new() -> Result<Self, JsError> {
        log!("Tonk worker initializing...");

        let operator = Operator::from_passphrase("public tonk").await;
        let space_did = operator.did().to_string();

        log!("Opening space: {}", space_did);

        let backend = ServiceWorkerStorageBackend::new(&space_did).await;
        let space: Space<ServiceWorkerStorageBackend> = Space::open(space_did, &operator, backend)
            .await
            .expect_throw("Could not open space");

        let router = Arc::new(Mutex::new(api_router(space)));

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

    /// Pushes local changes to the upstream remote.
    ///
    /// This method can be called from JavaScript via `event.waitUntil()` to
    /// perform push operations in the background without blocking the response.
    ///
    /// # Returns
    ///
    /// A JavaScript `Promise` that resolves to a boolean indicating success
    #[wasm_bindgen(js_name = "push")]
    pub fn push(&self) -> Promise {
        log!("Background push requested");

        let router = self.router.clone();

        future_to_promise(async move {
            // Create a push request internally
            let request = axum::http::Request::builder()
                .uri("/api/sync/push")
                .method("POST")
                .body(Body::empty())
                .expect("Failed to build push request");

            let response = router
                .lock()
                .await
                .call(request)
                .await
                .expect_throw("Failed to handle push request");

            let status = response.status().as_u16();
            log!("Push completed with status: {}", status);
            Ok(JsValue::from_bool(status >= 200 && status < 300))
        })
    }
}
