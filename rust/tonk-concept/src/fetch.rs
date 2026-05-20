//! Legacy `fetch()`-based transports — used when `globalThis.tonk`
//! is not available (e.g. the shell mounts `<tonk-concept>` or
//! `<tonk-display>` directly in its own DOM, outside an iframe).
//!
//! The shell sets `space` and `branch` HTML attributes so these
//! functions can build absolute
//! `/api/repository/{space}/branch/{branch}/query` URLs that the
//! service worker serves without going through the iframe bridge.
//!
//! `wasm_streams::ReadableStream::from_raw(...)` adapts the JS
//! `ReadableStream` returned by `Response::body()` into a Rust
//! `Stream<Item = Result<JsValue, JsValue>>`. Each chunk is a
//! `Uint8Array`; we accumulate bytes into a buffer, split on
//! `\n\n` (SSE event delimiter), strip the leading `data: ` from
//! each event's payload line, and hand the remaining JSON to the
//! callback.

use bytes::BytesMut;
use futures::StreamExt as _;
use js_sys::Uint8Array;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{
    AbortController, Headers, Request, RequestInit, Response, Window, window as get_window,
};

use crate::error::{ErrorDetail, ErrorKind};

/// POST `body` as JSON to `url`, parse the returned `Vec<Conclusion>`,
/// and return the first row's `this` (concept entity URI) plus its
/// `source` field (the descriptor JSON the next step needs).
pub async fn phase1_lookup(url: &str, body: &str) -> Result<(String, String), ErrorDetail> {
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
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("phase1 HTTP {}", resp.status()),
        ));
    }
    let text = JsFuture::from(
        resp.text()
            .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("text: {e:?}")))?,
    )
    .await
    .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("read body: {e:?}")))?;
    let body_text = text
        .as_string()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::Parse, "body was not a string"))?;
    let conclusions: Vec<tonk_schema::conclusion::Conclusion> = serde_json::from_str(&body_text)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("parse: {e}")))?;
    let first = conclusions
        .into_iter()
        .next()
        .ok_or_else(|| ErrorDetail::new(ErrorKind::UnknownSource, "no concept matched"))?;
    let source = first
        .fields
        .get("source")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            ErrorDetail::new(
                ErrorKind::Descriptor,
                "phase1 row missing `source` field — worker may not be on the AnonymousConceptQuery build",
            )
        })?;
    Ok((first.this, source))
}

/// Open an SSE subscription against `url`, sending `body` as the
/// JSON request body. The future resolves once the initial fetch
/// succeeds; subsequent frames flow through `on_frame`. Cancel by
/// calling `.abort()` on the returned [`AbortController`].
///
/// Errors during streaming are reported via `on_error`; the future
/// returns `Err` only for the initial fetch failure.
pub async fn open_sse(
    url: &str,
    body: &str,
    on_frame: impl FnMut(&str) + 'static,
    on_error: impl FnMut(ErrorDetail) + 'static,
) -> Result<AbortController, ErrorDetail> {
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

fn window_handle() -> Result<Window, ErrorDetail> {
    get_window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no `window` available"))
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
