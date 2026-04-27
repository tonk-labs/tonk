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
    extract::Request,
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
    async fn dispatch(&self, raw: &[u8]) -> Option<Vec<u8>> {
        let mut server = self.server.lock().await;
        let reply = server.handle_message(raw);
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

    /// Subscribe to the outbound channel. The slot is always
    /// populated — `shutdown` swaps in a fresh sender rather than
    /// leaving the slot empty — so this never fails.
    async fn subscribe(&self) -> broadcast::Receiver<Bytes> {
        self.outbound
            .lock()
            .await
            .as_ref()
            .expect("outbound channel always populated")
            .subscribe()
    }

    /// Hang up every active SSE subscriber.
    ///
    /// Called from the worker's `onupdatefound` export when a
    /// newer SW version begins installing. We *swap* the sender
    /// for a fresh one rather than `take`-ing it: the old sender
    /// drops (receivers see `Closed`, response bodies finish),
    /// and the freshly-installed sender means future subscribers
    /// on this same worker get a working channel.
    ///
    /// That last property matters: `onupdatefound` can fire for
    /// reasons that don't actually replace the worker (upgrade
    /// canceled, etc.), and we don't want to leave the hub
    /// permanently dead in those cases. With rebuild-on-drop on
    /// the client, the editor reconnects after a polite delay; if
    /// the worker hasn't been replaced by then, it lands here
    /// with a working subscriber. If it *has* been replaced, the
    /// page's `controllerchange`-equivalent (which we don't even
    /// need to track explicitly) sends the next fetch to the new
    /// worker.
    pub async fn shutdown(&self) {
        let mut slot = self.outbound.lock().await;
        let (fresh, _drop) = broadcast::channel(OUTBOUND_BUFFER);
        *slot = Some(fresh);
        // The previous sender drops at the end of this scope —
        // receivers tied to it surface `Closed` on next poll.
    }
}

/// Mount the LSP route onto an axum router. Returns both the
/// router *and* a handle to the [`LspHub`] so the worker entry
/// point can call [`LspHub::shutdown`] when a newer service worker
/// version begins installing.
pub fn lsp_router() -> (axum::Router, Arc<LspHub>) {
    let hub = LspHub::new();
    let router = axum::Router::new()
        .route("/api/language-server", get(handle_events).post(handle_post))
        .layer(Extension(hub.clone()));
    (router, hub)
}

/// `POST /api/language-server` handler. Reads the entire request
/// body as a JSON-RPC message, dispatches it, and returns the
/// reply (empty body for notifications).
#[wasm_compat]
async fn handle_post(Extension(hub): Extension<Arc<LspHub>>, request: Request) -> Response {
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
    let body = hub.dispatch(&bytes).await.unwrap_or_default();
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
async fn handle_events(Extension(hub): Extension<Arc<LspHub>>) -> Response {
    let receiver = hub.subscribe().await;
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
