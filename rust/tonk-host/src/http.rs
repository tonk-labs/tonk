//! Low-level HTTP / SSE primitives the host uses to talk to
//! the worker.
//!
//! These functions are the fetch-transport path. The
//! bridge-transport path (via `globalThis.tonk`) lives in
//! [`crate::bridge`]; [`crate::sse::open_sse`] picks the right
//! one at runtime.

use bytes::BytesMut;
use futures::StreamExt as _;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{AbortController, Headers, Request, RequestInit, Response, Window, window};

use crate::error::{ErrorDetail, ErrorKind};
use crate::ready;

fn window_handle() -> Result<Window, ErrorDetail> {
    window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no `window` available"))
}

/// POST JSON `body` to `url` with `accept: application/json`,
/// return the response body text. Errors out on non-2xx with a
/// `Network` kind error carrying the status code.
pub(crate) async fn post_json(url: &str, body: &str) -> Result<String, ErrorDetail> {
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
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));

    let request = Request::new_with_str_and_init(url, &init)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Request: {e:?}")))?;
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
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
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("HTTP {}: {body_text}", resp.status()),
        ));
    }
    Ok(body_text)
}

/// Open an SSE subscription against `url`, sending `body` as the
/// JSON request body. The future resolves once the initial fetch
/// succeeds; subsequent frames flow through `on_frame`. Cancel by
/// calling `.abort()` on the returned `AbortController`.
///
/// Errors during streaming are reported via `on_error`; the future
/// returns `Err` only for the initial fetch failure.
pub(crate) async fn open_sse(
    url: &str,
    body: &str,
    on_frame: impl FnMut(&str) + 'static,
    on_error: impl FnMut(ErrorDetail) + 'static,
) -> Result<AbortController, ErrorDetail> {
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
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));
    init.set_signal(Some(&abort.signal()));

    let request = Request::new_with_str_and_init(url, &init).map_err(|e| {
        ErrorDetail::new(ErrorKind::Network, format!("Request construction: {e:?}"))
    })?;

    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return a Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("HTTP {}", resp.status()),
        ));
    }

    let body_stream = resp
        .body()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "response has no body stream"))?;
    let stream = ReadableStream::from_raw(body_stream).into_stream();
    spawn_reader(stream, on_frame, on_error);
    Ok(abort)
}

fn spawn_reader(
    mut stream: impl futures::Stream<Item = Result<JsValue, JsValue>> + Unpin + 'static,
    mut on_frame: impl FnMut(&str) + 'static,
    mut on_error: impl FnMut(ErrorDetail) + 'static,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let mut buffer = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(value) => {
                    let array: Uint8Array = match value.dyn_into() {
                        Ok(a) => a,
                        Err(_) => continue,
                    };
                    buffer.extend_from_slice(&array.to_vec());
                    drain_frames(&mut buffer, &mut on_frame);
                }
                Err(e) => {
                    on_error(ErrorDetail::new(
                        ErrorKind::Network,
                        format!("stream read failed: {e:?}"),
                    ));
                    break;
                }
            }
        }
    });
}

/// Pull every complete SSE event (terminated by `\n\n`) out of
/// `buffer`, strip its `data: ` prefix, and invoke `on_frame` with
/// the inner JSON. Whatever bytes are left after the last `\n\n`
/// stay in the buffer for the next chunk.
fn drain_frames(buffer: &mut BytesMut, on_frame: &mut impl FnMut(&str)) {
    while let Some(idx) = find_double_newline(buffer) {
        let frame = buffer.split_to(idx);
        let _ = buffer.split_to(2); // discard "\n\n"
        if let Ok(text) = std::str::from_utf8(&frame)
            && let Some(payload) = strip_sse_data_prefix(text)
        {
            on_frame(payload);
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
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(body));

    let request = Request::new_with_str_and_init(url, &init)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Request: {e:?}")))?;
    let win = window_handle()?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
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
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("HTTP {}: {body_text}", resp.status()),
        ));
    }
    Ok(body_text)
}
