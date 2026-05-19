//! Iframe-side bridge interop. Calls into `globalThis.tonk` (set
//! by `/__tonk/bridge.js`) for query and subscribe — replacing the
//! previous direct `fetch()` calls so the bridge can route every
//! data-plane request through postMessage to the service worker.

use crate::error::{ErrorDetail, ErrorKind};
use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::window;

/// Look up the `tonk` binding on the global object. Returns an
/// error if the bridge module hasn't loaded yet — the wrapper
/// injected by `host::wrap_html_body` loads bridge.js before this
/// crate's runtime, so the only way this should fail is if a
/// caller mounted `<tonk-concept>` outside the iframe shell.
fn tonk_global() -> Result<JsValue, ErrorDetail> {
    let win = window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no window"))?;
    let tonk = Reflect::get(&win, &JsValue::from_str("tonk"))
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk lookup: {e:?}")))?;
    if tonk.is_undefined() || tonk.is_null() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            "globalThis.tonk is not defined — is the bridge module loaded?",
        ));
    }
    Ok(tonk)
}

fn tonk_method(name: &str) -> Result<Function, ErrorDetail> {
    let tonk = tonk_global()?;
    let method = Reflect::get(&tonk, &JsValue::from_str(name))
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.{name} lookup: {e:?}")))?;
    method
        .dyn_into::<Function>()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, format!("tonk.{name} is not a function")))
}

/// `tonk.query(body)` — one-shot. Body is a `serde_json::Value`.
/// Returns the result as a `serde_json::Value` (typically
/// `Vec<Conclusion>`-shaped — caller deserialises).
pub async fn query(body: &serde_json::Value) -> Result<serde_json::Value, ErrorDetail> {
    let tonk = tonk_global()?;
    let method = tonk_method("query")?;
    let body_js = serde_wasm_bindgen::to_value(body)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body serialise: {e}")))?;
    let promise_value = method
        .call1(&tonk, &body_js)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.query call: {e:?}")))?;
    let promise: Promise = promise_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "tonk.query did not return Promise"))?;
    let result_js = JsFuture::from(promise)
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.query await: {e:?}")))?;
    serde_wasm_bindgen::from_value(result_js)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("query result deserialise: {e}")))
}

/// `tonk.subscribe(body, onFrame, onError)` — streaming. Returns a
/// [`SubscribeHandle`] that the caller must keep alive for the
/// duration of the subscription.
///
/// `on_frame` is invoked per emission with a `serde_json::Value`.
/// `on_error` is invoked with a `String` on bridge-reported errors.
///
/// Dropping the handle calls the unsubscribe function automatically.
pub fn subscribe(
    body: &serde_json::Value,
    on_frame: impl Fn(serde_json::Value) + 'static,
    on_error: impl Fn(String) + 'static,
) -> Result<SubscribeHandle, ErrorDetail> {
    let tonk = tonk_global()?;
    let method = tonk_method("subscribe")?;
    let body_js = serde_wasm_bindgen::to_value(body)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body serialise: {e}")))?;

    // `on_error` is shared between both closures so we wrap it in
    // Rc<dyn Fn(String)> to avoid a move conflict.
    let on_error = std::rc::Rc::new(on_error);
    let on_error_for_frame = on_error.clone();

    let on_frame_cb: Closure<dyn FnMut(JsValue)> = Closure::new(move |frame_js: JsValue| {
        match serde_wasm_bindgen::from_value::<serde_json::Value>(frame_js) {
            Ok(v) => on_frame(v),
            Err(e) => on_error_for_frame(format!("frame deserialise: {e}")),
        }
    });
    let on_error_cb: Closure<dyn FnMut(JsValue)> = Closure::new(move |err_js: JsValue| {
        let message = err_js.as_string().unwrap_or_else(|| format!("{err_js:?}"));
        on_error(message);
    });

    let unsub_value = method
        .call3(
            &tonk,
            &body_js,
            on_frame_cb.as_ref().unchecked_ref(),
            on_error_cb.as_ref().unchecked_ref(),
        )
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.subscribe call: {e:?}")))?;
    let unsub: Function = unsub_value
        .dyn_into()
        .map_err(|_| {
            ErrorDetail::new(
                ErrorKind::Network,
                "tonk.subscribe did not return a function",
            )
        })?;

    Ok(SubscribeHandle {
        unsub,
        _on_frame: on_frame_cb,
        _on_error: on_error_cb,
    })
}

/// Handle returned by [`subscribe`]. Holds the unsubscribe function
/// and the closures so they remain live while JS might invoke them.
///
/// Dropping this value calls the unsubscribe function automatically.
pub struct SubscribeHandle {
    unsub: Function,
    _on_frame: Closure<dyn FnMut(JsValue)>,
    _on_error: Closure<dyn FnMut(JsValue)>,
}

impl Drop for SubscribeHandle {
    fn drop(&mut self) {
        // Best-effort: ignore the result.
        let _ = self.unsub.call0(&JsValue::UNDEFINED);
    }
}
