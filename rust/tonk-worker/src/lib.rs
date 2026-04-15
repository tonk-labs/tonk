#![warn(missing_docs)]
//! Service worker implementation for Tonk.
//!
//! This crate provides a Wasm-based service worker that runs in the browser and
//! handles API requests using Axum.
//!
//! The worker is designed around an [`axum::Router`]. The mental model of
//! authoring routes is the same as though they were being authored in a server
//! context, but in this case the "server" is just a service worker running in a
//! browser tab.
//!
//! To extend the worker with support for a new route, add one to the `router/`
//! directory and then include it in the router configuration in `router.rs`.
//!
//! To extend the JavaScript-visible API surface area, extend the struct found
//! in `worker.rs`.
//!
//! # Deploying the worker
//!
//! The substantial business logic of the service worker is implemented in Rust.
//! However, it is necessary to have a JavaScript shim to load it in a web
//! browser because Wasm initialization is async by necessity _however_ service
//! worker requires event-timing-sensitive initialization when the worker
//! installs and activates. Refer to the `service_worker.js` implementation in
//! `tonk-ui` for an example of how to implement a suitable shim.

mod axum;
pub use axum::*;

// TODO: These modules are disabled while migrating from tonk-space to
// dialog-repository. They will be rewritten in a follow-up PR.
// mod router;
// pub use router::*;

mod error;
pub use error::*;

// mod worker;
// pub use worker::*;

mod storage;
pub use storage::*;

// mod account;
// pub use account::*;

// mod key_store;
// pub use key_store::*;

// mod identity;
// pub use identity::*;

// mod session;
// pub use session::*;

mod r#async;
pub use r#async::*;

// Stub types while migrating to dialog-repository.
// These maintain the public API surface so tonk-ui compiles.

/// Placeholder — will be reimplemented with dialog-repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatusResponse {
    /// Space DID.
    pub space_did: String,
    /// Whether an upstream remote is configured.
    pub has_upstream: bool,
}

/// Placeholder — will be reimplemented with dialog-repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthorizeResponse {
    /// Whether authorization succeeded.
    pub ok: bool,
}

/// Placeholder — will be reimplemented with dialog-repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DelegationsResponse {
    /// List of delegation CIDs.
    pub delegations: Vec<String>,
}

/// Placeholder — will be reimplemented with dialog-repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IdentifyResponse {
    /// User DID.
    pub did: String,
}

/// Placeholder — will be reimplemented with dialog-repository.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyncResponse {
    /// Whether sync succeeded.
    pub success: bool,
}

/// Placeholder — will be reimplemented with dialog-repository.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen]
pub struct TonkServiceWorker;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen]
impl TonkServiceWorker {
    /// Create a new service worker instance.
    #[wasm_bindgen(constructor)]
    pub async fn new() -> Result<TonkServiceWorker, wasm_bindgen::JsError> {
        Err(wasm_bindgen::JsError::new(
            "TonkServiceWorker not yet reimplemented with dialog-repository",
        ))
    }
}
