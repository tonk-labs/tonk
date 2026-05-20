//! Subscription helper — selects bridge or fetch transport at
//! runtime and exposes a uniform `SubscriptionAbort` handle.
//!
//! When `globalThis.tonk` is present (the iframe shell has loaded
//! `bridge.js`) the bridge path is used: `globalThis.tonk.subscribe`
//! routes every data request over postMessage to the service worker.
//!
//! When `globalThis.tonk` is absent (the shell mounts these elements
//! directly in its own DOM, outside any iframe) the legacy
//! `fetch()`-based SSE reader in [`crate::fetch`] is used instead.
//! The caller is responsible for supplying a non-empty `url` in
//! that case (built from the `space`/`branch` HTML attributes).

use std::rc::Rc;

use web_sys::AbortController;

use crate::bridge::{SubscribeHandle, subscribe};
use crate::error::{ErrorDetail, ErrorKind};

/// Transport handle returned by [`open_sse`].
///
/// Dropping this value cancels the subscription regardless of which
/// transport is active.
pub enum SubscriptionAbort {
    /// Bridge path — `Drop` calls the bridge's unsubscribe function.
    Bridge(SubscribeHandle),
    /// Fetch/SSE path — `Drop` calls `AbortController::abort()`.
    Fetch(AbortController),
}

impl Drop for SubscriptionAbort {
    fn drop(&mut self) {
        match self {
            SubscriptionAbort::Bridge(_handle) => {
                // SubscribeHandle's own Drop impl calls unsub().
            }
            SubscriptionAbort::Fetch(ctrl) => {
                ctrl.abort();
            }
        }
    }
}

/// Pick a transport. Prefer the iframe bridge if it's loaded
/// (`globalThis.tonk` is defined and not null); otherwise fall back
/// to direct fetch against `url`. The shell mounts these elements
/// with `space`/`branch` attributes set; the iframe-wrapped body
/// mounts them without (and has `globalThis.tonk` available).
pub fn use_bridge() -> bool {
    use js_sys::Reflect;
    use wasm_bindgen::JsValue;
    let Some(win) = web_sys::window() else {
        return false;
    };
    let Ok(tonk) = Reflect::get(&win, &JsValue::from_str("tonk")) else {
        return false;
    };
    !tonk.is_undefined() && !tonk.is_null()
}

/// Open a streaming subscription using whichever transport is
/// available.
///
/// `url` is only used on the legacy fetch path. `body` is the
/// query as a `serde_json::Value`; on the fetch path it is
/// stringified once for the request body.
///
/// `on_frame` is called for each emitted frame with the raw JSON
/// string of a `Vec<Conclusion>`. `on_error` is called on transport
/// errors.
///
/// Returns a [`SubscriptionAbort`] that the caller must keep alive;
/// dropping it cancels the subscription.
pub async fn open_sse(
    url: &str,
    body: &serde_json::Value,
    on_frame: impl Fn(&str) + 'static,
    on_error: impl Fn(ErrorDetail) + 'static,
) -> Result<SubscriptionAbort, ErrorDetail> {
    if use_bridge() {
        let on_error = Rc::new(on_error);
        let on_error_for_frame = on_error.clone();
        let handle = subscribe(
            body,
            move |frame_value| match serde_json::to_string(&frame_value) {
                Ok(s) => on_frame(&s),
                Err(e) => {
                    on_error_for_frame(ErrorDetail::new(
                        ErrorKind::Parse,
                        format!("frame stringify: {e}"),
                    ));
                }
            },
            move |message| {
                on_error(ErrorDetail::new(ErrorKind::Network, message));
            },
        )?;
        Ok(SubscriptionAbort::Bridge(handle))
    } else {
        let body_str = serde_json::to_string(body)
            .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body stringify: {e}")))?;
        // on_frame / on_error must be FnMut for the SSE reader loop.
        let on_frame_cell = std::cell::RefCell::new(on_frame);
        let on_error_cell = std::cell::RefCell::new(on_error);
        let ctrl = crate::fetch::open_sse(
            url,
            &body_str,
            move |frame| (on_frame_cell.borrow())(frame),
            move |err| (on_error_cell.borrow())(err),
        )
        .await?;
        Ok(SubscriptionAbort::Fetch(ctrl))
    }
}
