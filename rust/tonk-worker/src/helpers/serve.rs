//! Serve a real worker router from inside the page.
//!
//! The service worker answers `/api/...` by converting a browser
//! `Request` into an axum one, running the router, and converting the
//! response back. A DOM test cannot install a service worker, but it can
//! do exactly that conversion in-page: override `fetch`, and anything
//! that reaches for `/api/...` — the real `tonk-host`, unmodified —
//! talks to a real router over a real `Request`/`Response` pair.
//!
//! The conversion is [`crate::axum`]'s, the same one the worker uses, so
//! a test is not checking its own idea of the wire. That includes
//! streamed bodies, which is what makes subscriptions work: the host's
//! SSE transport reads a `ReadableStream` off the response, and
//! `ResponseConversion` produces one.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use axum::Router;
use axum::http::Method;
use tower::ServiceExt as _;
use wasm_bindgen::{JsCast, JsValue, prelude::Closure};
use web_sys::{Request, RequestInit, Response};

use crate::axum::{RequestConversion, ResponseConversion};

/// Replace `fetch` with one that routes into `app`.
///
/// Returns the previous `fetch`, which the override falls back to for
/// anything outside `/api/` — the test runner's own page assets keep
/// loading.
pub fn install_fetch(app: Router) -> JsValue {
    let global = js_sys::global();
    let previous = js_sys::Reflect::get(&global, &"fetch".into()).unwrap_or(JsValue::UNDEFINED);
    let fallback = previous.clone();

    let handler = Closure::<dyn Fn(JsValue, JsValue) -> js_sys::Promise>::new(
        move |input: JsValue, init: JsValue| {
            let app = app.clone();
            let fallback = fallback.clone();
            wasm_bindgen_futures::future_to_promise(async move {
                let request = to_request(&input, &init)?;
                if !request.url().contains("/api/") {
                    // Not ours: hand it back to the real `fetch`, or
                    // fail loudly rather than silently returning nothing.
                    let f: js_sys::Function = fallback
                        .dyn_into()
                        .map_err(|_| JsValue::from_str("no fetch to fall back to"))?;
                    let promise: js_sys::Promise =
                        f.call1(&js_sys::global(), &request.into())?.dyn_into()?;
                    return wasm_bindgen_futures::JsFuture::from(promise).await;
                }
                let method = Method::from_bytes(request.method().as_bytes())
                    .map_err(|e| JsValue::from_str(&format!("bad method: {e}")))?;
                let axum_request = RequestConversion::from(request)
                    .into_axum_request()
                    .await
                    .map_err(JsValue::from)?;
                let axum_response = app
                    .oneshot(axum_request)
                    .await
                    .map_err(|e| JsValue::from_str(&format!("router failed: {e}")))?;
                let response: Response = ResponseConversion::new(method, axum_response)
                    .try_into()
                    .map_err(JsValue::from)?;
                Ok(response.into())
            })
        },
    );
    let _ = js_sys::Reflect::set(&global, &"fetch".into(), handler.as_ref().unchecked_ref());
    // The override lives for the rest of the page.
    handler.forget();
    previous
}

/// Normalise `fetch`'s two call shapes into a `Request`.
fn to_request(input: &JsValue, init: &JsValue) -> Result<Request, JsValue> {
    if let Some(request) = input.dyn_ref::<Request>() {
        // `Request::clone` is the spec's clone — it re-tees the body so
        // the original stays usable — and can fail on a consumed one.
        return request.clone();
    }
    let url = input
        .as_string()
        .ok_or_else(|| JsValue::from_str("fetch input is neither a Request nor a URL"))?;
    if init.is_undefined() || init.is_null() {
        return Request::new_with_str(&url);
    }
    let init: RequestInit = init.clone().unchecked_into();
    Request::new_with_str_and_init(&url, &init)
}
