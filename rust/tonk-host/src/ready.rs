//! Service-worker readiness gate.
//!
//! Every IO path the host opens (`post_json`, `post_text`,
//! `open_sse`, and the bridge equivalents) needs to wait until
//! the service worker has activated. Without that, an `/api/*`
//! fetch fired during cold start lands on the static-asset
//! server and comes back as 405 Method Not Allowed.
//!
//! The shell exposes the wait point as a global Promise factory
//! `globalThis.serviceWorkerActivates()`. Recheck that factory before
//! each IO: an installed successor can close the gate again while
//! the incumbent retires. Consumer elements do not need
//! to thread their own readiness signal through Leptos
//! contexts.
//!
//! On native targets, `wait()` is an immediate no-op so shared
//! code paths (e.g. the UI crate's `api.rs`, which is wasm in
//! production but reachable from native test builds) compile
//! without conditional callers.

#[cfg(target_arch = "wasm32")]
mod imp {
    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::window;

    /// Require the service worker to be ready for new IO, including
    /// handoff of a previously ready page to an installed successor.
    /// Missing shell globals remain a
    /// successful no-op for test harnesses and embeds; rejection of an
    /// installed readiness hook is returned without opening the gate.
    pub async fn require() -> Result<(), JsValue> {
        // Probe and await the current gate. Each failure mode
        // (no window, missing global, not a function, not a promise)
        // is treated as "already ready" so the call doesn't hang in
        // environments without the shell hook (test harness, embeds).
        let Some(win) = window() else {
            return Ok(());
        };
        let Ok(activates_val) = Reflect::get(&win, &JsValue::from_str("serviceWorkerActivates"))
        else {
            return Ok(());
        };
        let Ok(activates) = activates_val.dyn_into::<Function>() else {
            return Ok(());
        };
        let result = activates.call0(&JsValue::UNDEFINED)?;
        let Ok(promise) = result.dyn_into::<Promise>() else {
            return Ok(());
        };
        JsFuture::from(promise).await?;
        Ok(())
    }

    /// Tolerant compatibility gate for existing host IO callers.
    pub async fn wait() {
        let _ = require().await;
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    /// Native no-op so non-wasm builds that include this module
    /// (e.g. test compilation of higher-level crates) still
    /// resolve the symbol.
    pub async fn wait() {}
}

#[cfg(target_arch = "wasm32")]
pub use imp::require;
pub use imp::wait;

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn readiness_can_close_again_for_a_worker_handoff() {
        let window = web_sys::window().unwrap();
        let key = JsValue::from_str("serviceWorkerActivates");
        let old = Reflect::get(&window, &key).unwrap();
        let hook = Function::new_no_args("return globalThis.__tonkReadyTestPromise;");
        Reflect::set(&window, &key, &hook).unwrap();
        let gate_key = JsValue::from_str("__tonkReadyTestPromise");
        Reflect::set(&window, &gate_key, &Promise::resolve(&JsValue::UNDEFINED)).unwrap();
        super::require().await.unwrap();

        let mut release = None;
        let handoff = Promise::new(&mut |resolve, _| release = Some(resolve));
        Reflect::set(&window, &gate_key, &handoff).unwrap();
        let mut waiting = Box::pin(super::require());
        let blocked = futures::poll!(waiting.as_mut()).is_pending();
        release.unwrap().call0(&JsValue::UNDEFINED).unwrap();
        JsFuture::from(handoff).await.unwrap();
        if blocked {
            waiting.await.unwrap();
        }
        Reflect::set(&window, &key, &old).unwrap();
        Reflect::delete_property(&window, &gate_key).unwrap();
        assert!(
            blocked,
            "a prior successful wait must not bypass a later handoff"
        );
    }
}
