#![warn(missing_docs)]
//! Service worker implementation for Tonk.
//!
//! Storage rides dialog's IndexedDB adapters; their transaction settling
//! is race-armed and watchdogged (see dialog-storage's `settle`).
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

// Guard IDB event handlers against a torn-down wasm instance. When a
// worker is killed or replaced mid-transaction (an update swap, a
// DevTools stop), IndexedDB still delivers the transaction's terminal
// events — into wasm-bindgen shims whose instance is gone, which throw
// `closure invoked recursively or after being dropped` as uncaught
// errors. The wasm side cannot fix this: the code that owns the
// closure no longer exists. So the handler SETTERS are wrapped here,
// and an event that lands on a dead shim is logged quietly instead of
// thrown — semantically it is a no-op addressed to a dead instance.
export function patch_idb_dead_shims() {
    const guard = (proto, name) => {
        const desc = Object.getOwnPropertyDescriptor(proto, name);
        if (!desc || !desc.set) return;
        const origSet = desc.set;
        Object.defineProperty(proto, name, {
            set(handler) {
                if (typeof handler !== 'function') {
                    origSet.call(this, handler);
                    return;
                }
                origSet.call(this, function (...args) {
                    try {
                        return handler.apply(this, args);
                    } catch (e) {
                        if (String(e && e.message || e).includes('closure invoked')) {
                            console.debug(
                                `idb ${name}: event for a torn-down wasm instance ignored`
                            );
                            return;
                        }
                        throw e;
                    }
                });
            },
            get: desc.get,
            configurable: true,
        });
    };
    for (const name of ['oncomplete', 'onerror', 'onabort']) {
        guard(IDBTransaction.prototype, name);
    }
    for (const name of ['onsuccess', 'onerror']) {
        guard(IDBRequest.prototype, name);
    }
    guard(IDBOpenDBRequest.prototype, 'onupgradeneeded');
}
"#)]
extern "C" {
    /// Apply the IDB versionchange workaround. Must be called before any
    /// IDB operations. Called automatically by `TonkServiceWorker::new()`
    /// and should be called at the start of wasm tests.
    pub fn patch_idb_versionchange();

    /// Wrap IDB handler setters so events addressed to a torn-down wasm
    /// instance are ignored instead of throwing. Must be called before
    /// any IDB operations, alongside [`patch_idb_versionchange`].
    pub fn patch_idb_dead_shims();
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod idb_lifecycle_tests {
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[wasm_bindgen(inline_js = r#"
export function dropped_idb_request_handler() {
    const request = indexedDB.deleteDatabase(
        `tonk-dropped-idb-handler-${Date.now()}-${Math.random()}`
    );
    request.onsuccess = () => {
        throw new Error('closure invoked recursively or after being dropped');
    };
    const handler = request.onsuccess;
    try {
        handler.call(request, new Event('success'));
        return 'ignored';
    } catch (error) {
        return String(error && error.message || error);
    } finally {
        request.onsuccess = null;
    }
}
"#)]
    extern "C" {
        fn dropped_idb_request_handler() -> String;
    }

    /// A browser may still deliver an event after its originating wasm
    /// instance has been torn down. That one known stale-shim exception is a
    /// no-op addressed to dead code, while every other handler error remains
    /// visible.
    #[dialog_common::test]
    fn it_ignores_an_idb_event_for_a_dropped_wasm_closure() {
        crate::patch_idb_dead_shims();
        assert_eq!(dropped_idb_request_handler(), "ignored");
    }
}

mod broadcast;
pub use broadcast::*;

mod axum;
pub use axum::*;

mod router;
pub use router::*;

mod credential;

mod onboarding;

mod error;
pub use error::*;

mod worker;
pub use worker::*;

pub mod device;
pub mod session;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod cache;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use cache::{set_asset_paths, set_build_id};

mod r#async;
pub use r#async::*;

pub use dialog_reactor as reactor;
pub use dialog_reactor::*;

#[cfg(any(test, feature = "helpers"))]
pub mod helpers;
