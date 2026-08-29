//! Low-level HTTP / SSE primitives the host uses to talk to
//! the worker.
//!
//! These functions are the transport, everywhere: in a sealed guest
//! `window.fetch` is the portal bootstrap's override, which relays
//! each host-relative request to the outer frame and streams the
//! response back.

use bytes::BytesMut;
use futures::StreamExt as _;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{AbortController, Headers, RequestInit, Response, Window, window};

use crate::error::{ErrorDetail, ErrorKind};
use crate::ready;

fn window_handle() -> Result<Window, ErrorDetail> {
    window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no `window` available"))
}

/// Append the request-context headers (`X-Tonk-Path`/`X-Tonk-Hash`/
/// `X-Tonk-Session`) so the SW can tie the request to its originating document.
/// Best-effort: a failed append never blocks the request.
fn append_context_headers(headers: &Headers) {
    for (name, value) in crate::bridge::context_headers() {
        let _ = headers.append(name, &value);
    }
}

/// GET JSON from a bare host-relative `url` and return the response body text.
///
/// A sealed guest's `window.fetch` override relays the unexpanded `/api/...`
/// string to the top document. Non-2xx responses retain their status and body
/// in the returned network error.
pub async fn get_json(url: &str) -> Result<String, ErrorDetail> {
    ready::wait().await;
    let init = RequestInit::new();
    init.set_method("GET");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("accept", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    append_context_headers(&headers);
    init.set_headers(&headers);

    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    response_text(resp_value).await
}

/// POST JSON `body` to a bare host-relative `url` and return the response body
/// text.
///
/// A sealed guest's `window.fetch` override relays the unexpanded `/api/...`
/// string to the top document. Non-2xx responses retain their status and body
/// in the returned network error.
pub async fn post_json(url: &str, body: &str) -> Result<String, ErrorDetail> {
    // Gate every `/api/*` request on service-worker activation.
    // Without this, an early call lands on the static-asset
    // server and comes back as 405. Idempotent — after the first
    // wait the gate is open for the page lifetime.
    ready::wait().await;
    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("content-type", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("content-type: {e:?}")))?;
    headers
        .append("accept", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    append_context_headers(&headers);
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));

    // Fetch the relative URL as a STRING, not a `Request` — a `Request`
    // resolves `url` against `document.baseURI` at construction, turning the
    // host-relative path absolute; inside a sealed guest the overridden
    // `window.fetch` relays only host-relative strings reliably (the
    // absolute form needs the bridge context's origin, which races the
    // `ready` envelope). See `post_site_to`.
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    response_text(resp_value).await
}

async fn response_text(resp_value: JsValue) -> Result<String, ErrorDetail> {
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return Response"))?;
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text: {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read body: {e:?}")))?;
    let body_text = text
        .as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "body was not a string"))?;
    if !resp.ok() {
        // `409` from the version handshake means this page is talking to
        // a worker from a different build (`skipWaiting` + `claim` swap
        // the worker underneath a running page). Nothing the caller can
        // do about it, and retrying will fail identically — so raise the
        // same update prompt the version probe raises and let the user
        // reload onto a matched pair.
        if resp.status() == 409 && body_text.contains("stale-build") {
            announce_update();
        }
        return Err(ErrorDetail::http(
            resp.status(),
            format!("HTTP {}: {body_text}", resp.status()),
        ));
    }
    Ok(body_text)
}

/// Raise the boot script's "update ready" prompt.
///
/// Dispatched as an event rather than called directly because the
/// prompt lives in `index.html`'s boot script — deliberately outside
/// the app wasm, since the app wasm is one of the things that can be
/// the stale half.
fn announce_update() {
    let Some(win) = window() else { return };
    if let Ok(event) = web_sys::Event::new("tonk-update-available") {
        let _ = win.dispatch_event(&event);
    }
}

/// `POST /api/site` to register this document's site and read back the assigned
/// `site:<client-id>` entity. The SW matches the route from `X-Tonk-Path`, so the
/// caller passes the **route's** path explicitly rather than relying on
/// `window.location` — on a client-side navigation the resource fires before the
/// router has committed the new URL, so reading `window.location` would carry the
/// stale (previous) path and the SW would resolve the wrong route. The SW keys
/// the site on the requesting client id, so no body is needed. Returns the `site`
/// field of the JSON response.
pub(crate) async fn post_site(path: &str) -> Result<String, ErrorDetail> {
    post_site_to("/api/site", path).await
}

/// `POST` a per-branch `/site` endpoint (`url`) to register this document's site
/// on an explicit branch and read back the `site:<client-id>` entity. Like
/// [`post_site`] but the branch is named in `url` (e.g.
/// `/api/profile/branch/main/site`), so the SW does no document-path routing —
/// it matches `path` against that branch's route table. The path rides both the
/// `X-Tonk-Path` header (legacy `/api/site` reads it there) and the JSON body
/// (the per-branch endpoint reads `{path}`), so one builder serves both.
pub(crate) async fn post_site_to(url: &str, path: &str) -> Result<String, ErrorDetail> {
    ready::wait().await;
    let init = RequestInit::new();
    init.set_method("POST");
    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &JsValue::from_str("path"), &JsValue::from_str(path));
    let body = js_sys::JSON::stringify(&obj)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_else(|| "{}".to_owned());
    init.set_body(&JsValue::from_str(&body));
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("accept", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    let _ = headers.append("content-type", "application/json");
    // The authoritative path comes from the caller (the route), not the context
    // headers' `window.location` read — see the doc comment. Carried as a header
    // for the legacy `/api/site` and in the body for the per-branch endpoint.
    let _ = headers.append("x-tonk-path", path);
    init.set_headers(&headers);

    // Fetch the relative URL as a STRING, not a `Request`. A `Request` resolves
    // `url` against `document.baseURI` at construction; inside a sealed guest
    // that baseURI is the host's real origin, so the relative `/api/...` becomes
    // a fully-qualified cross-origin URL that the guest's overridden
    // `window.fetch` may not strip (origin `null` → CORS block). Passing the
    // bare string lets the override catch the host-relative `/…` and relay it
    // through `window.tonk.fetch` to the parent. The nested `<tonk-site>` is a
    // sealed guest that calls this, so the opaque-origin caveat DOES apply.
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return Response"))?;
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text: {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read body: {e:?}")))?;
    let body_text = text
        .as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "body was not a string"))?;
    if !resp.ok() {
        return Err(ErrorDetail::http(
            resp.status(),
            format!("HTTP {}: {body_text}", resp.status()),
        ));
    }
    // Pull the `site` field out of `{"site":"site:<id>"}` without a serde dep.
    let value: js_sys::Object = js_sys::JSON::parse(&body_text)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("parse /api/site: {e:?}")))?
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Parse, "/api/site body not an object"))?;
    js_sys::Reflect::get(&value, &JsValue::from_str("site"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "/api/site response missing `site`"))
}

/// Open an SSE subscription against `url`, sending `body` as the
/// JSON request body, and return it as a uniform **frame stream**
/// plus a teardown closure for [`crate::sse::Subscription`].
///
/// The future resolves once the initial fetch succeeds (so an
/// initial failure is an `Err` here, not a stream item). Each stream
/// item is one complete SSE frame's JSON payload, or a transport
/// error. The teardown aborts the in-flight fetch; the
/// [`crate::sse::Subscription`] reader stops on its drop signal
/// before the resulting abort-rejected read is observed, and the
/// `is_abort_error` filter below drops any abort rejection that still
/// races through — so a deliberate teardown never yields an error
/// item.
pub(crate) async fn frame_stream(
    url: &str,
    body: &str,
) -> Result<
    (
        futures::stream::LocalBoxStream<'static, Result<String, ErrorDetail>>,
        impl FnOnce() + 'static,
    ),
    ErrorDetail,
> {
    ready::wait().await;
    let abort = AbortController::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("AbortController: {e:?}")))?;

    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers::new: {e:?}")))?;
    headers
        .append("accept", "text/event-stream")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    headers
        .append("content-type", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("content-type: {e:?}")))?;
    append_context_headers(&headers);
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));
    init.set_signal(Some(&abort.signal()));

    // Fetch the relative URL as a STRING, not a `Request` — a `Request`
    // resolves `url` against `document.baseURI` at construction, turning the
    // host-relative path absolute; inside a sealed guest the overridden
    // `window.fetch` relays only host-relative strings reliably (the
    // absolute form needs the bridge context's origin, which races the
    // `ready` envelope). See `post_site_to`.
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return a Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::http(
            resp.status(),
            format!("HTTP {}", resp.status()),
        ));
    }

    let body_stream = resp
        .body()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "response has no body stream"))?;
    let byte_stream = ReadableStream::from_raw(body_stream).into_stream();
    let frames = sse_frames(byte_stream).boxed_local();
    let teardown = move || abort.abort();
    Ok((frames, teardown))
}

/// Adapt a raw byte stream of SSE bytes into a stream of complete
/// frame payloads. Buffers across chunks, splits on `\n\n`, strips
/// the `data:` prefix. A read that fails because we aborted the
/// fetch ourselves (`AbortError`) ends the stream silently; any
/// other read failure is forwarded as a transport error.
fn sse_frames(
    byte_stream: impl futures::Stream<Item = Result<JsValue, JsValue>> + Unpin + 'static,
) -> impl futures::Stream<Item = Result<String, ErrorDetail>> {
    let state = (
        byte_stream,
        BytesMut::new(),
        std::collections::VecDeque::new(),
    );
    futures::stream::unfold(
        state,
        |(mut byte_stream, mut buffer, mut ready)| async move {
            loop {
                // Drain any already-buffered complete frames first.
                if let Some(frame) = ready.pop_front() {
                    return Some((Ok(frame), (byte_stream, buffer, ready)));
                }
                match byte_stream.next().await {
                    Some(Ok(value)) => {
                        if let Ok(array) = value.dyn_into::<Uint8Array>() {
                            buffer.extend_from_slice(&array.to_vec());
                            collect_frames(&mut buffer, &mut ready);
                        }
                        // Loop to emit a newly-completed frame (or read more).
                    }
                    Some(Err(e)) => {
                        if is_abort_error(&e) {
                            // Deliberate teardown — end the stream cleanly.
                            return None;
                        }
                        return Some((
                            Err(ErrorDetail::new(
                                ErrorKind::Network,
                                format!("stream read failed: {e:?}"),
                            )),
                            (byte_stream, buffer, ready),
                        ));
                    }
                    None => return None,
                }
            }
        },
    )
}

/// True when a rejected stream read is the `AbortError` raised by
/// our own `AbortController::abort()` (the subscription was torn
/// down deliberately), rather than a real transport failure.
///
/// An aborted `fetch` body read rejects with a `DOMException` whose
/// `name` is `"AbortError"`. We match on that name; anything else
/// (or a non-exception value) is treated as a genuine error.
fn is_abort_error(value: &JsValue) -> bool {
    value
        .dyn_ref::<web_sys::DomException>()
        .is_some_and(|exception| exception.name() == "AbortError")
}

/// Pull every complete SSE event (terminated by `\n\n`) out of
/// `buffer`, strip its `data: ` prefix, and invoke `on_frame` with
/// the inner JSON. Whatever bytes are left after the last `\n\n`
/// stay in the buffer for the next chunk.
fn collect_frames(buffer: &mut BytesMut, out: &mut std::collections::VecDeque<String>) {
    while let Some(idx) = find_double_newline(buffer) {
        let frame = buffer.split_to(idx);
        let _ = buffer.split_to(2); // discard "\n\n"
        if let Ok(text) = std::str::from_utf8(&frame)
            && let Some(payload) = strip_sse_data_prefix(text)
        {
            out.push_back(payload.to_owned());
        }
    }
}

fn find_double_newline(b: &[u8]) -> Option<usize> {
    b.windows(2).position(|w| w == b"\n\n")
}

/// `data: {…}\n` → `{…}`. Multi-line `data:` events are joined
/// with literal `\n` per the SSE spec, but the worker emits
/// single-line frames so the simple form is enough.
fn strip_sse_data_prefix(frame: &str) -> Option<&str> {
    frame
        .strip_prefix("data: ")
        .or_else(|| frame.strip_prefix("data:"))
}

/// POST raw `body` (asserted-notation text) to `url` with
/// `content-type: text/x-tonk-notation` (or similar; using
/// `application/octet-stream` for now — the worker reads the
/// body bytes, content-type is not consulted).
pub(crate) async fn post_text(
    url: &str,
    body: &str,
    content_type: &str,
) -> Result<String, ErrorDetail> {
    ready::wait().await;
    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("content-type", content_type)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("content-type: {e:?}")))?;
    headers
        .append("accept", "application/json")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("accept: {e:?}")))?;
    append_context_headers(&headers);
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));

    // Fetch the relative URL as a STRING, not a `Request` — a `Request`
    // resolves `url` against `document.baseURI` at construction, turning the
    // host-relative path absolute; inside a sealed guest the overridden
    // `window.fetch` relays only host-relative strings reliably (the
    // absolute form needs the bridge context's origin, which races the
    // `ready` envelope). See `post_site_to`.
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return Response"))?;
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text: {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read body: {e:?}")))?;
    let body_text = text
        .as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "body was not a string"))?;
    if !resp.ok() {
        return Err(ErrorDetail::http(
            resp.status(),
            format!("HTTP {}: {body_text}", resp.status()),
        ));
    }
    Ok(body_text)
}
