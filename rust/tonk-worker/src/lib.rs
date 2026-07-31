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

/// Patch IDBDatabase.prototype.onversionchange to use a JS-native handler
/// instead of wasm-bindgen's Closure::once. The `idb` crate registers a
/// Closure::once for onversionchange that panics if the wasm instance is
/// replaced (e.g., service worker update) but the native IDBDatabase still
/// receives the event. This patch intercepts the setter so every database
/// gets a JS handler that simply closes the connection.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function patch_idb_versionchange() {
    const desc = Object.getOwnPropertyDescriptor(
        IDBDatabase.prototype, 'onversionchange'
    );
    if (!desc || !desc.set) return;
    const origSet = desc.set;
    Object.defineProperty(IDBDatabase.prototype, 'onversionchange', {
        set(handler) {
            origSet.call(this, function() { this.close(); });
        },
        get: desc.get,
        configurable: true,
    });
}
"#)]
extern "C" {
    /// Apply the IDB versionchange workaround. Must be called before any
    /// IDB operations. Called automatically by `TonkServiceWorker::new()`
    /// and should be called at the start of wasm tests.
    pub fn patch_idb_versionchange();
}

mod broadcast;
pub use broadcast::*;

mod axum;
pub use axum::*;

mod router;
pub use router::*;

mod credential;

mod error;
pub use error::*;

mod worker;
pub use worker::*;

pub mod device;
pub mod session;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod cache;

mod r#async;
pub use r#async::*;

pub use dialog_reactor as reactor;
pub use dialog_reactor::*;

#[cfg(any(test, feature = "helpers"))]
pub mod helpers;
