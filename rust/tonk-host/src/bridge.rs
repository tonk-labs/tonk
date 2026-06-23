//! Iframe-side bridge interop. Calls into `globalThis.tonk` (set
//! by `/__tonk/bridge.js`) for query and subscribe — replacing the
//! previous direct `fetch()` calls so the bridge can route every
//! data-plane request through postMessage to the service worker.

use std::cell::Cell;

use crate::error::{ErrorDetail, ErrorKind};
use crate::ready;
use ipld_core::ipld::Ipld;
use js_sys::{Function, Promise, Reflect};
use uuid::Uuid;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{ReadableStream, ReadableStreamDefaultReader, window};

/// Look up the `tonk` binding on the global object. Returns an
/// error if the bridge module hasn't loaded yet — the wrapper
/// injected by `host::wrap_html_body` loads bridge.js before this
/// crate's runtime, so the only way this should fail is if a
/// caller mounted a bridge consumer outside the iframe shell.
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

/// Read a string field off the bridge `window.tonk.context`, if present and
/// non-empty. The host populates the context (`{this, model, origin, repo,
/// branch}`) in its `ready` envelope; guest controls read routing they can't
/// resolve from the DOM (the `<tonk-repository>`/`<tonk-branch>` ancestors
/// live outside the iframe) from here.
pub fn context_field(name: &str) -> Option<String> {
    let win = window()?;
    let tonk = Reflect::get(&win, &JsValue::from_str("tonk")).ok()?;
    if tonk.is_undefined() || tonk.is_null() {
        return None;
    }
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    Reflect::get(&context, &JsValue::from_str(name))
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

/// The host page's real origin, as the bridge reports it in
/// `window.tonk.context.origin`. In a sealed guest `window.location.origin`
/// is `"null"` (opaque origin), so anything that needs a same-origin URL —
/// the invite link, the sync `/api` route — must read the origin from the
/// bridge context the host supplies. Falls back to `window.location.origin`
/// when there is no bridge (the element running in the real top document).
pub fn context_origin() -> Option<String> {
    if let Some(origin) = context_field("origin") {
        return Some(origin);
    }
    window()?
        .location()
        .origin()
        .ok()
        .filter(|o| !o.is_empty() && o != "null")
}

thread_local! {
    /// A stable id for this document instance (top page, or one sealed guest
    /// iframe). Each document runs its own wasm instance, so a per-instance
    /// cell gives a distinct id per tab/iframe. The SW keys the session entity
    /// (route + context facts) by this id. Minted lazily on first use.
    static SESSION_ID: std::cell::OnceCell<String> = const { std::cell::OnceCell::new() };
}

/// This document's session id, minted once per instance. A random v4 UUID,
/// distinct per live document.
pub fn session_id() -> String {
    SESSION_ID.with(|cell| {
        cell.get_or_init(|| format!("host:{}", Uuid::new_v4()))
            .clone()
    })
}

/// The request-context headers every host-relative `/api` request carries, so
/// the SW can tie the request to its originating document and route/contain it:
/// `X-Tonk-Hash`, `X-Tonk-Session`. The host does not interpret these; the SW
/// decides how to use them.
///
/// The document path is not stamped: it rides `Referer`, which the browser sets
/// to the host document's same-origin URL (and the host URL mirrors the guest
/// route by design). The fragment never rides `Referer`, so the hash is stamped
/// explicitly, and only when there is one.
///
/// The hash comes from the bridge context in a sealed guest (its
/// `window.location` is `about:srcdoc`, useless) and from `window.location` in
/// the top document — the same source split as [`context_origin`].
pub fn context_headers() -> Vec<(&'static str, String)> {
    let hash = context_field("hash")
        .or_else(|| window().and_then(|w| w.location().hash().ok()))
        .filter(|hash| !hash.is_empty());

    let mut headers = vec![("x-tonk-session", session_id())];
    if let Some(hash) = hash {
        headers.push(("x-tonk-hash", hash));
    }
    headers
}

/// GET a host-relative path and return its body text. When the bridge is
/// present (sealed guest), the fetch is performed by the HOST over the
/// `window.tonk.fetch` relay — the opaque guest can't reach a same-origin,
/// SW-routed `/api/...` endpoint itself. Without a bridge (the top document),
/// it falls back to a direct `window.fetch`.
pub async fn host_fetch_text(path: &str) -> Result<String, ErrorDetail> {
    if let Ok(method) = tonk_method("fetch") {
        let tonk = tonk_global()?;
        let promise = method
            .call1(&tonk, &JsValue::from_str(path))
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("tonk.fetch call: {e:?}")))?;
        let body = await_promise(promise, "tonk.fetch").await?;
        return body
            .as_string()
            .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "tonk.fetch body not a string"));
    }
    // No bridge: direct same-origin fetch (the element is in the top document).
    let win = window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no window"))?;
    let resp_value = JsFuture::from(win.fetch_with_str(path))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch {path}: {e:?}")))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch: not a Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("fetch {path}: {}", resp.status()),
        ));
    }
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text(): {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("await text: {e:?}")))?;
    text.as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "fetch body not a string"))
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
    ready::wait().await;
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
pub async fn subscribe(body: &Ipld) -> Result<Subscription, ErrorDetail> {
    ready::wait().await;
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
    Ok(Subscription {
        reader,
        cancelled: Cell::new(false),
    })
}

/// Open a bridge subscription as a uniform **frame stream** plus a
/// teardown closure for [`crate::sse::Subscription`].
///
/// Each `tonk.subscribe` value is a parsed conclusion batch; we
/// re-stringify it so the frame shape matches the fetch path (raw
/// JSON string). The teardown cancels the underlying
/// `ReadableStream`, which posts the `unsubscribe` envelope to the
/// SW. Cancelling resolves the reader's pending `read()` with
/// `done = true`, so the stream ends without yielding an error.
pub(crate) async fn frame_stream(
    body: &Ipld,
) -> Result<
    (
        futures::stream::LocalBoxStream<'static, Result<String, ErrorDetail>>,
        impl FnOnce() + 'static,
    ),
    ErrorDetail,
> {
    use futures::StreamExt as _;

    let subscription = std::rc::Rc::new(subscribe(body).await?);
    let teardown_handle = subscription.clone();
    let frames = futures::stream::unfold(subscription, |sub| async move {
        match sub.next().await {
            Ok(Some(frame)) => Some((Ok(frame), sub)),
            Ok(None) => None,
            Err(e) => Some((Err(e), sub)),
        }
    })
    .boxed_local();
    let teardown = move || teardown_handle.cancel();
    Ok((frames, teardown))
}

/// A live subscription. `next` yields one frame as its raw JSON
/// string; `Ok(None)` signals the stream closed normally. Dropping
/// the value cancels the stream and posts an `unsubscribe` envelope.
pub struct Subscription {
    reader: ReadableStreamDefaultReader,
    // Set on first `cancel()` so the second call (from the other
    // `Drop` path, when both `SubscriptionAbort::Bridge` and the
    // last `Rc<Subscription>` go away) doesn't allocate a redundant
    // `reader.cancel()` Promise.
    cancelled: Cell<bool>,
}

impl Subscription {
    /// Pull the next frame as its JSON-stringified form. `Ok(None)`
    /// means the stream closed. The string is produced by
    /// `JSON.stringify` on the raw `value` so the JSON tree never
    /// has to be walked in Rust.
    pub async fn next(&self) -> Result<Option<String>, ErrorDetail> {
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
        let stringified = js_sys::JSON::stringify(&value_js)
            .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("JSON.stringify: {e:?}")))?;
        let s = stringified.as_string().ok_or_else(|| {
            ErrorDetail::new(ErrorKind::Parse, "JSON.stringify yielded non-string")
        })?;
        Ok(Some(s))
    }

    /// Cancel the underlying stream. Safe to call multiple times;
    /// subsequent calls are no-ops.
    pub fn cancel(&self) {
        if self.cancelled.replace(true) {
            return;
        }
        let _ = self.reader.cancel();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancel();
    }
}
