//! The preview daemon spine: a capability-routed broker between
//! one-shot CLI clients and a connected browser harness page.
//!
//! The core ([`Daemon`]) is transport-free — pages attach as
//! channel pairs — so dispatch logic is tested without sockets.
//! The HTTP/WS surface (`router`/`serve`) is thin glue over it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::preview::protocol::{CAPABILITY_RENDER_PREVIEW, PageReply, PageRequest};

/// How long a dispatched request waits for the page to answer.
const DEFAULT_PAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a dispatch failed. Mapped onto HTTP statuses by the router.
#[derive(Debug)]
pub enum DispatchError {
    /// No handler registered for this capability name.
    UnknownCapability(String),
    /// No browser page is connected to the daemon.
    NoPage,
    /// The page connection dropped while the request was in flight.
    PageGone,
    /// The page did not answer within the timeout.
    Timeout,
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCapability(name) => write!(f, "unknown capability '{name}'"),
            Self::NoPage => write!(
                f,
                "no preview page connected — open the daemon URL in a browser first"
            ),
            Self::PageGone => write!(f, "the preview page disconnected mid-request"),
            Self::Timeout => write!(f, "the preview page did not answer in time"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// The capability broker. Cheap to clone; all clones share state.
#[derive(Clone)]
pub struct Daemon {
    inner: Arc<Inner>,
}

struct Inner {
    /// Outbound channel to the currently connected page, if any.
    page: Mutex<Option<mpsc::Sender<String>>>,
    /// In-flight requests awaiting a [`PageReply`].
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
    next_id: AtomicU64,
    page_timeout: Duration,
}

impl Daemon {
    /// A daemon with the default page timeout.
    pub fn new() -> Self {
        Self::with_timeout(DEFAULT_PAGE_TIMEOUT)
    }

    /// A daemon with an explicit page timeout (tests use a short one).
    pub fn with_timeout(page_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                page: Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                page_timeout,
            }),
        }
    }

    /// Register the (single) page connection, returning the
    /// receiver the transport pumps to the page. A new page
    /// replaces the previous one.
    pub async fn attach_page(&self) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel(16);
        *self.inner.page.lock().await = Some(tx);
        rx
    }

    /// Route a message from the page (a serialized [`PageReply`])
    /// to the dispatch awaiting it. Unknown ids are ignored — the
    /// request may have timed out already.
    pub async fn handle_page_message(&self, text: &str) -> Result<(), serde_json::Error> {
        let reply: PageReply = serde_json::from_str(text)?;
        if let Some(waiter) = self.inner.pending.lock().await.remove(&reply.id) {
            let _ = waiter.send(reply.payload);
        }
        Ok(())
    }

    /// Dispatch a capability request. `render-preview` forwards to
    /// the connected page; future capabilities register here.
    pub async fn dispatch(
        &self,
        capability: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DispatchError> {
        match capability {
            CAPABILITY_RENDER_PREVIEW => self.forward_to_page(capability, payload).await,
            other => Err(DispatchError::UnknownCapability(other.to_string())),
        }
    }

    async fn forward_to_page(
        &self,
        capability: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DispatchError> {
        let sender = self
            .inner
            .page
            .lock()
            .await
            .clone()
            .ok_or(DispatchError::NoPage)?;

        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);

        let envelope = PageRequest {
            id,
            capability: capability.to_string(),
            payload,
        };
        let text = serde_json::to_string(&envelope).expect("envelope serializes");
        if sender.send(text).await.is_err() {
            self.inner.pending.lock().await.remove(&id);
            *self.inner.page.lock().await = None;
            return Err(DispatchError::PageGone);
        }

        match tokio::time::timeout(self.inner.page_timeout, rx).await {
            Ok(Ok(payload)) => Ok(payload),
            Ok(Err(_)) => Err(DispatchError::PageGone),
            Err(_) => {
                self.inner.pending.lock().await.remove(&id);
                Err(DispatchError::Timeout)
            }
        }
    }
}

impl Default for Daemon {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared router state: the broker plus the harness asset root.
#[derive(Clone)]
struct AppState {
    daemon: Daemon,
    assets: Arc<PathBuf>,
}

/// Build the daemon's HTTP surface: the page WebSocket, the
/// capability endpoint, and harness asset serving.
pub fn router(daemon: Daemon, assets: PathBuf) -> Router {
    let state = AppState {
        daemon,
        assets: Arc::new(assets),
    };
    Router::new()
        .route("/ws/page", get(ws_handler))
        .route("/capability/{name}", post(capability_handler))
        .fallback(get(asset_handler))
        .with_state(state)
}

/// Bind on localhost and run until interrupted. Prints the URL the
/// human opens once; the harness page connects back over `/ws/page`.
pub async fn serve(port: u16, assets: PathBuf) -> anyhow::Result<()> {
    let daemon = Daemon::new();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let bound = listener.local_addr()?.port();
    println!("slide preview daemon listening on http://127.0.0.1:{bound}");
    println!("open that URL in a browser, then run `slide preview render ...`");
    axum::serve(listener, router(daemon, assets)).await?;
    Ok(())
}

async fn ws_handler(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| page_session(state.daemon, socket))
}

/// Pump the attached page: daemon-bound requests out, page replies
/// in. Ends when either side closes.
async fn page_session(daemon: Daemon, mut socket: WebSocket) {
    let mut outbound = daemon.attach_page().await;
    loop {
        tokio::select! {
            request = outbound.recv() => {
                let Some(text) = request else { break };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        let _ = daemon.handle_page_message(&text).await;
                    }
                    Some(Ok(_)) => {} // ignore pings/binary
                    _ => break,
                }
            }
        }
    }
}

async fn capability_handler(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    axum::Json(payload): axum::Json<serde_json::Value>,
) -> Response {
    match state.daemon.dispatch(&name, payload).await {
        Ok(reply) => axum::Json(reply).into_response(),
        Err(e) => {
            let status = match &e {
                DispatchError::UnknownCapability(_) => StatusCode::NOT_FOUND,
                DispatchError::NoPage => StatusCode::SERVICE_UNAVAILABLE,
                DispatchError::PageGone => StatusCode::BAD_GATEWAY,
                DispatchError::Timeout => StatusCode::GATEWAY_TIMEOUT,
            };
            (status, e.to_string()).into_response()
        }
    }
}

/// Serve harness assets from the assets directory. `/` maps to
/// `index.html`; path traversal is rejected.
async fn asset_handler(State(state): State<AppState>, uri: axum::http::Uri) -> Response {
    let raw = uri.path().trim_start_matches('/');
    let relative = if raw.is_empty() { "index.html" } else { raw };
    if relative.contains("..") {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }
    let path = state.assets.join(relative);
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type(relative))], bytes).into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            format!("{relative} not found — did you `trunk build` the harness and pass --assets?"),
        )
            .into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::protocol::{PageReply, PageRequest};
    use std::time::Duration;

    mod when_dispatching_capabilities {
        use super::*;

        #[dialog_common::test]
        async fn it_rejects_an_unknown_capability() {
            let daemon = Daemon::new();
            let result = daemon.dispatch("teleport", serde_json::json!({})).await;
            assert!(matches!(result, Err(DispatchError::UnknownCapability(name)) if name == "teleport"));
        }

        #[dialog_common::test]
        async fn it_errors_when_no_page_is_connected() {
            let daemon = Daemon::new();
            let result = daemon
                .dispatch("render-preview", serde_json::json!({"template": "<b>{x}</b>"}))
                .await;
            assert!(matches!(result, Err(DispatchError::NoPage)));
        }

        #[dialog_common::test]
        async fn it_round_trips_a_request_through_a_connected_page() {
            let daemon = Daemon::new();
            let mut page = daemon.attach_page().await;

            // Fake page: answer the first request with a fixed reply.
            let responder = daemon.clone();
            tokio::spawn(async move {
                let text = page.recv().await.expect("page receives a request");
                let request: PageRequest = serde_json::from_str(&text).expect("envelope parses");
                assert_eq!(request.capability, "render-preview");
                let reply = PageReply {
                    id: request.id,
                    payload: serde_json::json!({"html": "<b>Alice</b>", "row_count": 1}),
                };
                responder
                    .handle_page_message(&serde_json::to_string(&reply).unwrap())
                    .await
                    .expect("reply routes to the pending request");
            });

            let payload = daemon
                .dispatch("render-preview", serde_json::json!({"template": "<b>{name}</b>"}))
                .await
                .expect("dispatch resolves");
            assert_eq!(payload["html"], "<b>Alice</b>");
        }

        #[dialog_common::test]
        async fn it_times_out_when_the_page_never_replies() {
            let daemon = Daemon::with_timeout(Duration::from_millis(50));
            let _page = daemon.attach_page().await; // connected but silent
            let result = daemon.dispatch("render-preview", serde_json::json!({})).await;
            assert!(matches!(result, Err(DispatchError::Timeout)));
        }
    }

    mod when_serving_http {
        use super::*;

        /// Bind the router on an ephemeral port; return its base URL.
        async fn serve_for_test(daemon: Daemon, assets: std::path::PathBuf) -> String {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind ephemeral port");
            let addr = listener.local_addr().expect("local addr");
            let app = router(daemon, assets);
            tokio::spawn(async move {
                axum::serve(listener, app).await.expect("serve");
            });
            format!("http://{addr}")
        }

        #[dialog_common::test]
        async fn it_returns_503_when_no_page_is_connected() {
            let assets = tempfile::tempdir().expect("tempdir");
            let base = serve_for_test(Daemon::new(), assets.path().to_path_buf()).await;
            let response = reqwest::Client::new()
                .post(format!("{base}/capability/render-preview"))
                .json(&serde_json::json!({"template": "<b>{x}</b>", "conclusions": []}))
                .send()
                .await
                .expect("request reaches the daemon");
            assert_eq!(response.status(), 503);
            let body = response.text().await.unwrap();
            assert!(body.contains("open"), "actionable message, got: {body}");
        }

        #[dialog_common::test]
        async fn it_returns_404_for_an_unknown_capability() {
            let assets = tempfile::tempdir().expect("tempdir");
            let base = serve_for_test(Daemon::new(), assets.path().to_path_buf()).await;
            let response = reqwest::Client::new()
                .post(format!("{base}/capability/teleport"))
                .json(&serde_json::json!({}))
                .send()
                .await
                .expect("request reaches the daemon");
            assert_eq!(response.status(), 404);
        }

        #[dialog_common::test]
        async fn it_serves_the_harness_index() {
            let assets = tempfile::tempdir().expect("tempdir");
            std::fs::write(assets.path().join("index.html"), "<html>harness</html>").unwrap();
            let base = serve_for_test(Daemon::new(), assets.path().to_path_buf()).await;
            let response = reqwest::get(format!("{base}/")).await.expect("get index");
            assert_eq!(response.status(), 200);
            assert!(response.text().await.unwrap().contains("harness"));
        }
    }
}
