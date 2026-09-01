//! In-process language server, exposed over HTTP.
//!
//! The carry asserted-notation language server lives inside this
//! service worker — same process as the dialog-db, so completion
//! providers (when we add them) can hit live data without crossing
//! a network boundary.
//!
//! One HTTP endpoint, two methods:
//!
//! - `POST /api/language-server` — request/response. The body is
//!   a single JSON-RPC 2.0 message (no LSP `Content-Length`
//!   framing — that only matters for stdio transports). The
//!   response body is the matching JSON-RPC reply, or empty for
//!   notifications.
//!
//! - `GET /api/language-server` (with `Accept: text/event-stream`)
//!   — opens a server-sent-event subscription. Server-initiated
//!   notifications (most importantly
//!   `textDocument/publishDiagnostics`) arrive here.
//!
//! Two methods on one route rather than two routes is a deliberate
//! shape choice: the *client* should care about LSP, not about how
//! it's plumbed. Different operations on the same logical resource
//! (a JSON-RPC channel) belong on the same URL with different
//! verbs. WebSocket would be tidier still — but service workers
//! don't intercept WebSocket connections, so the channel has to
//! ride on `fetch`.

use std::sync::Arc;

use axum::{
    Extension,
    body::{Body, Bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_wasm_macros::wasm_compat;
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use tokio::sync::{Mutex, broadcast};
// `wasm_compat` expands to code that uses `oneshot` by bare path
// on the wasm cfg; on native it isn't referenced. Import it
// unconditionally so both cfgs see the symbol.
#[allow(unused_imports)]
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use tonk_common::log;
use tonk_language_server::Server;

use crate::router::AppState;
use crate::router::lsp_env::LspEnvProvider;
use crate::router::update_pending;

/// Channel capacity for outbound LSP notifications.
///
/// Each event is at most a few hundred bytes (a `publishDiagnostics`
/// payload). The capacity sets an upper bound on how many events a
/// slow consumer can fall behind before we start dropping — once
/// every receiver is at the head, the buffer wraps. 256 is enough
/// for any realistic burst (a `didChange` produces exactly one
/// notification today; even rapid typing won't outrun the SSE
/// stream).
const OUTBOUND_BUFFER: usize = 256;

/// Hub shared across LSP routes. One instance per worker; lives as
/// long as the worker does. Added to the router as an `Extension`
/// rather than wedged into `AppState` so it stays orthogonal to
/// the rest of the app's state shape.
pub struct LspHub {
    /// The language server itself. Sequential — every request
    /// takes the lock for the duration of `handle_message`. That's
    /// fine in a single-threaded SW: there's no real parallelism
    /// to lose, and ordering matters for document state (a
    /// `didChange` mustn't interleave with a `diagnostic` request
    /// from the same client).
    server: Mutex<Server>,
    /// Broadcast channel for server-initiated notifications. Each
    /// subscriber gets a fresh `Receiver`.
    ///
    /// `Option`-wrapped so [`shutdown`] can drop the `Sender`
    /// outright when the worker is being torn down — every
    /// receiver then surfaces `Closed`, the `BroadcastStream`
    /// adapter ends, and each open SSE response body finishes.
    /// That settles the in-flight fetch events the SW spec
    /// otherwise uses to keep the worker alive.
    ///
    /// Subscribers arriving after shutdown get `None`, treated as
    /// "already closed" — the client tears down its LSP session
    /// and rebuilds against whichever worker is the controller
    /// next time.
    outbound: Mutex<Option<broadcast::Sender<Bytes>>>,
}

impl LspHub {
    /// Construct a fresh hub with an empty document set and a
    /// freshly-allocated outbound broadcast channel.
    pub fn new() -> Arc<Self> {
        let (outbound, _drop) = broadcast::channel(OUTBOUND_BUFFER);
        Arc::new(Self {
            server: Mutex::new(Server::new()),
            outbound: Mutex::new(Some(outbound)),
        })
    }

    /// Run an LSP message through the server and queue any
    /// resulting outbound notifications onto the broadcast.
    /// Returns the JSON-RPC response (for requests) or `None`
    /// (for notifications and unparseable messages).
    ///
    /// `env` is the per-request [`LspEnvProvider`] — the language
    /// server resolves diagnostics, completion, and hover against
    /// whatever live branch it opens through it.
    async fn dispatch(&self, raw: &[u8], env: &LspEnvProvider) -> Option<Vec<u8>> {
        let mut server = self.server.lock().await;
        let reply = server.handle_message(raw, env).await;
        let outbound = self.outbound.lock().await;
        for note in server.take_outbound() {
            let bytes = match serde_json::to_vec(&note) {
                Ok(v) => v,
                Err(err) => {
                    log!("[lsp] failed to serialize outbound notification: {err}");
                    continue;
                }
            };
            if let Some(sender) = outbound.as_ref() {
                // `send` errors only when there are zero receivers —
                // harmless, future subscribers will see future events.
                let _ = sender.send(Bytes::from(bytes));
            }
            // After shutdown there are no subscribers anyway; drop.
        }
        reply
    }

    /// Subscribe to the outbound channel, or `None` once
    /// [`shutdown`](Self::shutdown) has run — matching what the
    /// `outbound` field documents.
    ///
    /// This used to hand back a receiver unconditionally, because
    /// `shutdown` installed a FRESH sender instead of emptying the
    /// slot. That made the hub's teardown reversible by any client
    /// that redialed, and the LSP client redials on a flat timer: the
    /// old worker dropped its streams on `updatefound`, then ~5 s
    /// later handed out a brand new one and re-pinned itself, parking
    /// its replacement in `waiting` for good.
    async fn subscribe(&self) -> Option<broadcast::Receiver<Bytes>> {
        Some(self.outbound.lock().await.as_ref()?.subscribe())
    }

    /// Hang up every active SSE subscriber, terminally.
    ///
    /// Called from the worker's `onupdatefound` export when a newer
    /// SW version begins installing. Taking the sender (rather than
    /// swapping in a fresh one) drops it: every receiver surfaces
    /// `Closed`, each `BroadcastStream` ends, and the SSE response
    /// bodies finish — which settles the in-flight fetch events the
    /// spec was using to keep this worker alive.
    ///
    /// Terminal is the point. Installing a fresh sender here left the
    /// hub able to serve a NEW stream moments later, and the LSP
    /// client's reconnect timer did exactly that — re-pinning the
    /// worker this teardown existed to release.
    ///
    /// A worker that turns out not to be replaced after all (a failed
    /// install, a canceled upgrade) is covered by `handle_events`
    /// reading [`update_pending`] live: with no successor waiting it
    /// answers `503` + `Retry-After` instead of a dead 200, and the
    /// client's next dial after the successor activates lands on the
    /// new worker's hub, which has a sender of its own.
    pub async fn shutdown(&self) {
        self.outbound.lock().await.take();
        // The sender drops here — receivers tied to it surface
        // `Closed` on next poll.
    }
}

/// Mount the LSP route onto an axum router. Takes the worker's
/// [`AppState`] so the POST handler can open the live environment
/// for completion / hover / diagnostics. Returns both the router
/// *and* a handle to the [`LspHub`] so the worker entry point can
/// call [`LspHub::shutdown`] when a newer service worker version
/// begins installing.
pub fn lsp_router(state: AppState) -> (axum::Router, Arc<LspHub>) {
    let hub = LspHub::new();
    let router = axum::Router::new()
        .route("/api/language-server", get(handle_events).post(handle_post))
        .layer(Extension(hub.clone()))
        .with_state(state);
    (router, hub)
}

/// `POST /api/language-server` handler. Reads the entire request
/// body as a JSON-RPC message, dispatches it against the live
/// environment opened from the worker state, and returns the
/// reply (empty body for notifications).
#[wasm_compat]
async fn handle_post(
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let bytes = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            log!("[lsp] failed to read request body: {err}");
            return (StatusCode::BAD_REQUEST, "failed to read body").into_response();
        }
    };

    // Always respond `200 OK`, even when the message was a
    // notification (no JSON-RPC reply). Using `204` would be
    // semantically cleaner, but the SW's response adapter
    // (`router::axum`) attaches a body stream for every response,
    // and 204 forbids one — the browser would throw. Empty 200 is
    // the JSON-RPC "no reply" signal anyway.
    let env = LspEnvProvider::new(state);
    let body = hub.dispatch(&bytes, &env).await.unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

/// `GET /api/language-server` handler. Returns a
/// `text/event-stream` response whose body emits one SSE event
/// per outbound notification.
///
/// SSE framing: `data: <json>\n\n`. The client side parses these
/// and hands the JSON to its message dispatcher.
#[wasm_compat]
async fn handle_events(
    Extension(hub): Extension<Arc<LspHub>>,
    State(state): State<AppState>,
) -> Response {
    let retiring = state.read().await.is_retiring();
    handle_events_for(hub, retiring).await
}

async fn handle_events_for(hub: Arc<LspHub>, retiring: bool) -> Response {
    // Refuse to open a long-lived stream while a successor is waiting.
    // An SSE body is a fetch event that never settles, and the spec
    // keeps a worker alive while any fetch event is in flight — so one
    // stream opened here re-pins this retiring worker and parks its
    // replacement in `waiting`. The query-subscription route has
    // refused for the same reason; this one didn't, and the LSP
    // client's reconnect timer made it the reliable way to wedge an
    // update.
    if retiring || update_pending() {
        return retry_later("a newer service worker is waiting to activate");
    }
    // `None` means `shutdown` already ran on this hub. Same answer:
    // the client should come back, and by then the successor will be
    // the controller and will answer with its own live hub.
    let Some(receiver) = hub.subscribe().await else {
        return retry_later("the language server hub has shut down");
    };
    // `BroadcastStream` adapts the receiver into a `Stream`. Lagged
    // items surface as `Err(BroadcastStreamRecvError::Lagged(n))`
    // which we silently filter — a slow consumer just catches up
    // from the newest message; the next edit will refresh the
    // diagnostic state for any document the client cares about.
    // When the hub's sender is dropped (via `LspHub::shutdown`) the
    // receiver yields `Closed` and the stream ends, terminating
    // this response body cleanly.
    let body_stream =
        BroadcastStream::new(receiver).filter_map(|result: Result<Bytes, _>| async move {
            let bytes = result.ok()?;
            let mut framed = Vec::with_capacity(bytes.len() + 8);
            framed.extend_from_slice(b"data: ");
            framed.extend_from_slice(&bytes);
            framed.extend_from_slice(b"\n\n");
            Some(Ok::<_, std::io::Error>(Bytes::from(framed)))
        });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(body_stream))
        .unwrap()
}

/// Tell the client to come back rather than handing it a stream this
/// worker must not open. `Retry-After` is advisory; the client's own
/// held reconnect (it waits for `controllerchange`) is what actually
/// paces the redial onto the successor.
fn retry_later(reason: &str) -> Response {
    log!("[lsp] refusing SSE subscription: {reason}");
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header(header::RETRY_AFTER, "5")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "control": "update-pending", "reason": reason }).to_string(),
        ))
        .expect("response builder failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// `shutdown` must be TERMINAL. It used to install a fresh sender,
    /// so a client that redialed moments later got a working stream on
    /// a worker that was trying to retire — and an SSE body is a fetch
    /// event that never settles, so that stream re-pinned the outgoing
    /// worker and parked its replacement in `waiting` indefinitely.
    /// That is the "Safari keeps the old version through every reload"
    /// symptom: the reloads land on the old ACTIVE worker.
    #[dialog_common::test]
    async fn it_refuses_to_subscribe_after_shutdown() {
        let hub = LspHub::new();
        assert!(
            hub.subscribe().await.is_some(),
            "a live hub hands out receivers"
        );

        hub.shutdown().await;
        assert!(
            hub.subscribe().await.is_none(),
            "a hub that has shut down must not hand out a new receiver"
        );

        // And it stays terminal: a second dial doesn't revive it either.
        assert!(
            hub.subscribe().await.is_none(),
            "shutdown is one-way for this hub's lifetime"
        );
    }

    /// The route — not just the hub — must decline after shutdown, and
    /// it must decline in the shape the client can act on: a `503`
    /// carrying `update-pending`, so the consumer HOLDS its reconnect
    /// for `controllerchange` instead of redialing this worker on a
    /// timer. A plain error would be indistinguishable from a network
    /// blip and would be retried on the short backoff, which is what
    /// re-pinned the outgoing worker.
    #[dialog_common::test]
    async fn it_answers_retry_later_after_shutdown() {
        let hub = LspHub::new();
        hub.shutdown().await;

        let response = handle_events_for(hub, false).await;

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a retiring worker declines rather than opening a stream"
        );
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("5"),
            "the client is told to come back"
        );
        assert_ne!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream"),
            "declining must NOT hand back a stream — that is the whole bug"
        );

        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("update-pending"),
            "the client distinguishes this from a blip by the control \
             signal, and holds its reconnect on it: {text}"
        );
    }

    /// A live hub still opens a real stream. The refusal must be
    /// conditional — a worker that is not retiring has to keep serving
    /// diagnostics, or this "fix" would simply break the LSP.
    #[dialog_common::test]
    async fn it_opens_a_stream_while_not_retiring() {
        let hub = LspHub::new();

        let response = handle_events_for(hub, false).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream"),
            "the normal path is still a live SSE subscription"
        );
    }

    /// The synchronous generation latch is sufficient to refuse a reconnect
    /// even before the asynchronous hub drain acquires its state lock.
    #[dialog_common::test]
    async fn it_refuses_a_stream_once_the_worker_is_retiring() {
        let hub = LspHub::new();

        let response = handle_events_for(hub, true).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("5")
        );
    }

    /// Shutting down ends the streams already open, which is what
    /// settles the in-flight fetch events keeping the worker alive.
    #[dialog_common::test]
    async fn it_closes_open_subscribers_on_shutdown() {
        let hub = LspHub::new();
        let mut receiver = hub.subscribe().await.expect("live hub");

        hub.shutdown().await;

        assert!(
            matches!(
                receiver.recv().await,
                Err(broadcast::error::RecvError::Closed)
            ),
            "an open receiver must see Closed so its SSE response body ends"
        );
    }
}
