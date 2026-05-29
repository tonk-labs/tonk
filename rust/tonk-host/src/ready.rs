//! Service-worker readiness gate.
//!
//! Every IO path the host opens (`post_json`, `post_text`,
//! `open_sse`, and the bridge equivalents) needs to wait until
//! the service worker has activated. Without that, an `/api/*`
//! fetch fired during cold start lands on the static-asset
//! server and comes back as 405 Method Not Allowed.
//!
//! The shell exposes the wait point as a global Promise factory
//! `globalThis.serviceWorkerActivates()`. This module memoizes
//! the wait so the first IO awaits it once, every subsequent
//! call returns immediately. Consumer elements no longer need
//! to thread their own readiness signal through Leptos
//! contexts.
//!
//! On native targets, `wait()` is an immediate no-op so shared
//! code paths (e.g. the UI crate's `api.rs`, which is wasm in
//! production but reachable from native test builds) compile
//! without conditional callers.

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::window;

    thread_local! {
        /// Cached "service worker is up" flag. Once `wait()` has
        /// returned at least once the gate stays open for the page's
        /// lifetime, so we skip the global-lookup + promise-await on
        /// every subsequent call.
        static SW_READY: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    }

    /// Resolve once the service worker is activated. Idempotent
    /// and cheap to call repeatedly — after the first successful
    /// await, returns immediately. Silently no-ops when the shell
    /// hook isn't installed (test harness, embeds) so callers
    /// don't hang in those environments.
    pub async fn wait() {
        // Fast path: already known ready.
        let cached = SW_READY.with(|cell| *cell.borrow());
        if cached {
            return;
        }

        // Slow path: probe and await the global. Each failure mode
        // (no window, missing global, not a function, not a promise)
        // is treated as "already ready" so the call doesn't hang in
        // environments without the shell hook (test harness, embeds).
        let Some(win) = window() else {
            SW_READY.with(|cell| *cell.borrow_mut() = true);
            return;
        };
        let Ok(activates_val) = Reflect::get(&win, &JsValue::from_str("serviceWorkerActivates"))
        else {
            SW_READY.with(|cell| *cell.borrow_mut() = true);
            return;
        };
        let Ok(activates) = activates_val.dyn_into::<Function>() else {
            SW_READY.with(|cell| *cell.borrow_mut() = true);
            return;
        };
        let Ok(result) = activates.call0(&JsValue::UNDEFINED) else {
            SW_READY.with(|cell| *cell.borrow_mut() = true);
            return;
        };
        let Ok(promise) = result.dyn_into::<Promise>() else {
            SW_READY.with(|cell| *cell.borrow_mut() = true);
            return;
        };
        let _ = JsFuture::from(promise).await;
        SW_READY.with(|cell| *cell.borrow_mut() = true);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    /// Native no-op so non-wasm builds that include this module
    /// (e.g. test compilation of higher-level crates) still
    /// resolve the symbol.
    pub async fn wait() {}
}

pub use imp::wait;
