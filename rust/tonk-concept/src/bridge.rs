//! Iframe-side bridge interop. Calls into `globalThis.tonk` (set
//! by `/__tonk/bridge.js`) for query and subscribe — replacing the
//! previous direct `fetch()` calls so the bridge can route every
//! data-plane request through postMessage to the service worker.

use crate::error::{ErrorDetail, ErrorKind};
use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{ReadableStream, ReadableStreamDefaultReader, window};

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

/// Await a JS Promise and downcast its resolved value.
async fn await_promise(promise_value: JsValue, what: &str) -> Result<JsValue, ErrorDetail> {
    let promise: Promise = promise_value.dyn_into().map_err(|_| {
        ErrorDetail::new(ErrorKind::Network, format!("{what} did not return Promise"))
    })?;
    JsFuture::from(promise)
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("{what} await: {e:?}")))
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
    let result_js = await_promise(promise_value, "tonk.query").await?;
    serde_wasm_bindgen::from_value(result_js)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("query result deserialise: {e}")))
}

/// `tonk.subscribe(body)` — streaming. Resolves to a
/// [`Subscription`] backed by the underlying `ReadableStream`.
///
/// The caller drives the subscription via [`Subscription::next`].
/// Dropping the [`Subscription`] cancels the stream, which posts
/// the corresponding `unsubscribe` envelope to the SW.
pub async fn subscribe(body: &serde_json::Value) -> Result<Subscription, ErrorDetail> {
    let tonk = tonk_global()?;
    let method = tonk_method("subscribe")?;
    let body_js = serde_wasm_bindgen::to_value(body)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("body serialise: {e}")))?;
    let promise_value = method
        .call1(&tonk, &body_js)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.subscribe call: {e:?}")))?;
    let stream_js = await_promise(promise_value, "tonk.subscribe").await?;
    let stream: ReadableStream = stream_js.dyn_into().map_err(|_| {
        ErrorDetail::new(
            ErrorKind::Network,
            "tonk.subscribe did not yield a ReadableStream",
        )
    })?;
    let reader: ReadableStreamDefaultReader = stream.get_reader().dyn_into().map_err(|_| {
        ErrorDetail::new(
            ErrorKind::Network,
            "stream.getReader did not return a default reader",
        )
    })?;
    Ok(Subscription { reader })
}

/// A live subscription. `next` yields one frame; `Ok(None)` signals
/// the stream closed normally. Dropping the value cancels the
/// stream and posts an `unsubscribe` envelope.
pub struct Subscription {
    reader: ReadableStreamDefaultReader,
}

impl Subscription {
    /// Pull the next frame. `Ok(None)` means the stream closed.
    pub async fn next(&self) -> Result<Option<serde_json::Value>, ErrorDetail> {
        let promise = self.reader.read();
        let result_js = JsFuture::from(promise)
            .await
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("reader.read: {e:?}")))?;
        let done = Reflect::get(&result_js, &JsValue::from_str("done"))
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read.done: {e:?}")))?
            .as_bool()
            .unwrap_or(false);
        if done {
            return Ok(None);
        }
        let value_js = Reflect::get(&result_js, &JsValue::from_str("value"))
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read.value: {e:?}")))?;
        let value = serde_wasm_bindgen::from_value(value_js)
            .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("frame deserialise: {e}")))?;
        Ok(Some(value))
    }

    /// Cancel the underlying stream. Idempotent — the JS `cancel`
    /// returns a Promise that we discard.
    pub fn cancel(&self) {
        let _ = self.reader.cancel();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}
