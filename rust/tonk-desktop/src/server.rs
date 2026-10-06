//! The loopback HTTP server the window's page talks to.
//!
//! In the browser, the page's `/api/...` fetches never reach a network:
//! the service worker answers them. Here a real server answers them, which
//! means anything on this machine can reach the port, and any web page in
//! any browser can send it requests. So it answers only the window it was
//! started for:
//!
//! - **Host check.** A request must name the loopback address and port it
//!   was sent to. That refuses DNS-rebinding, where a hostile page points
//!   a name it controls at `127.0.0.1` to make its requests same-origin.
//! - **Launch token.** The window opens `/__tonk/launch?token=…` with a
//!   random token minted at startup. That sets an `HttpOnly`,
//!   `SameSite=Strict` cookie, and every request that reaches the worker
//!   must carry it. Another page cannot read the cookie, and its
//!   cross-site requests arrive without it. The built UI itself is
//!   public code and needs no token: sealed guest frames load images
//!   from it directly, and their opaque origin sends no cookie.
//!
//! The server sends no CORS headers, so a browser lets no other origin
//! read a response even when a request does arrive. WebSockets are not
//! covered by CORS, so the stream route also checks the `Origin` header.
//!
//! Sealed guest frames (opaque origins) never fetch from here directly:
//! the portal bootstrap relays their fetches through the top document,
//! so every request comes from the window's own origin, cookie included.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use futures_util::StreamExt as _;
use tonk_worker::axum::RequestOrigin;
use tonk_worker::{AppState, ClientId};
use tower::ServiceExt as _;

/// Name of the cookie that carries the launch token.
const COOKIE: &str = "tonk-desktop";

/// The page's client identity.
///
/// The service worker tags each request with the id of the document that
/// sent it. Routes that keep per-tab state (`/api/site`, the profile
/// generation fence) need one. Every request here comes from the one
/// window's top document (guest frames relay through it), so a single
/// fixed id stands in for it.
const CLIENT_ID: &str = "desktop-window";

/// What every request is checked and dispatched against.
#[derive(Clone)]
pub struct Server {
    /// The loopback address the server listens on, as `127.0.0.1:port`.
    authority: String,
    /// The token the launch URL carries.
    token: Arc<str>,
    /// The built UI (`trunk build` output of `tonk-ui`).
    dist: Arc<Path>,
    /// The worker's router.
    worker: Router,
    /// The worker's state, kept so the server's lifetime holds it.
    _state: AppState,
}

impl Server {
    /// A server for the page served from `dist`, listening at `authority`.
    pub fn new(authority: String, dist: PathBuf, worker: Router, state: AppState) -> Self {
        let token = mint_token();
        Self {
            authority,
            token: token.into(),
            dist: dist.into(),
            worker,
            _state: state,
        }
    }

    /// The URL the window opens first: it exchanges the token for the
    /// cookie and redirects to the page.
    pub fn launch_url(&self) -> String {
        format!(
            "http://{}/__tonk/launch?token={}",
            self.authority, self.token
        )
    }

    /// The launch token, for the window's init script to present on the
    /// stream route.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The origin the page is served from.
    pub fn origin(&self) -> String {
        format!("http://{}", self.authority)
    }

    /// The axum app: the launch exchange, then everything else behind the
    /// guard.
    pub fn app(self) -> Router {
        Router::new()
            .route("/__tonk/launch", get(launch))
            .route("/api/health", get(health))
            .route("/__tonk/stream", get(stream))
            .fallback(dispatch)
            .layer(middleware::from_fn_with_state(self.clone(), guard))
            .with_state(self)
    }
}

/// A fresh 256-bit token, hex-encoded.
fn mint_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Refuse a request unless it names this server's address and, past the
/// launch exchange, carries the launch cookie.
///
/// A refused or failed request is logged to stderr with its status, so a
/// page that misbehaves can be diagnosed from the terminal that ran the app.
async fn guard(State(server): State<Server>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let response = if host != Some(server.authority.as_str()) {
        (StatusCode::MISDIRECTED_REQUEST, "unexpected host").into_response()
    } else if needs_token(&path) && !carries_token(request.headers(), &server.token) {
        (StatusCode::FORBIDDEN, "not this window").into_response()
    } else {
        next.run(request).await
    };
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        eprintln!("request failed: {method} {path} -> {status}");
    }
    response
}

/// Whether `path` reaches the worker, and so must carry the launch token.
///
/// The stream route checks the token itself, in the request's first
/// message, because WKWebView does not send the `SameSite=Strict` cookie
/// on a WebSocket handshake.
fn needs_token(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

/// Whether the request's cookies include the launch token.
fn carries_token(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, value)| name == COOKIE && constant_time_eq(value.as_bytes(), token.as_bytes()))
}

/// Compare without exiting at the first differing byte.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[derive(serde::Deserialize)]
struct Launch {
    token: String,
}

/// Exchange the launch token for the cookie, then load the page.
async fn launch(State(server): State<Server>, Query(launch): Query<Launch>) -> Response {
    if !constant_time_eq(launch.token.as_bytes(), server.token.as_bytes()) {
        return (StatusCode::FORBIDDEN, "not this window").into_response();
    }
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict",
        server.token
    );
    let mut response = Redirect::to("/").into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Answered by the server itself, as the service worker's shim does.
async fn health() -> &'static str {
    "ok"
}

/// `/api/...` goes to the worker; everything else is the built UI.
async fn dispatch(State(server): State<Server>, request: Request) -> Response {
    if request.uri().path().starts_with("/api/") || request.uri().path() == "/api" {
        return to_worker(&server, request).await;
    }
    serve_static(&server.dist, request.method(), request.uri()).await
}

/// Run `request` through the worker's router, with the extensions the
/// service worker would have attached.
async fn to_worker(server: &Server, mut request: Request) -> Response {
    let extensions = request.extensions_mut();
    extensions.insert(ClientId(CLIENT_ID.to_owned()));
    match RequestOrigin::parse(&format!("{}/", server.origin())) {
        Ok(origin) => {
            extensions.insert(origin);
        }
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "bad origin").into_response(),
    }
    match server.worker.clone().oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// A streaming `/api` request, as the page's native-host script sends it
/// over a WebSocket (see `native_host.js`).
#[derive(serde::Deserialize)]
struct StreamRequest {
    /// The launch token, which the init script holds in a closure no page
    /// or frame can read. Stands in for the cookie WKWebView leaves off
    /// the handshake.
    #[serde(default)]
    token: String,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

/// The response head, sent as the first message on the socket.
#[derive(serde::Serialize)]
struct StreamHead {
    status: u16,
    headers: Vec<(String, String)>,
}

/// Carry one streaming `/api` request over a WebSocket.
///
/// Browsers allow six HTTP/1.1 connections to one host, and every live
/// query holds one open for as long as it is watched, so a page watching
/// more than six stalls. WebSockets do not count against that limit.
async fn stream(
    State(server): State<Server>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    if origin != Some(server.origin().as_str()) {
        return (StatusCode::FORBIDDEN, "not this window").into_response();
    }
    let cookie = carries_token(&headers, &server.token);
    upgrade.on_upgrade(move |socket| relay(server, socket, cookie))
}

/// Whether a stream request may reach the worker: its handshake carried
/// the launch cookie, or its first message carries the launch token.
fn stream_admitted(cookie: bool, sent: &str, token: &str) -> bool {
    cookie || (!sent.is_empty() && constant_time_eq(sent.as_bytes(), token.as_bytes()))
}

/// Read the request off `socket`, answer it from the worker, and send the
/// response back until its body ends or the page closes the socket.
async fn relay(server: Server, mut socket: WebSocket, cookie: bool) {
    let Some(Ok(Message::Text(text))) = socket.recv().await else {
        return;
    };
    let Ok(sent) = serde_json::from_str::<StreamRequest>(&text) else {
        return;
    };
    if !stream_admitted(cookie, &sent.token, &server.token) {
        eprintln!(
            "request failed: stream {} -> refused, no launch token",
            sent.path
        );
        return;
    }
    // Only `/api` goes through here; the socket must not become a way
    // around the static-file rules.
    if !sent.path.starts_with("/api/") {
        return;
    }
    let mut builder = Request::builder()
        .method(sent.method.as_str())
        .uri(sent.path);
    for (name, value) in &sent.headers {
        builder = builder.header(name, value);
    }
    let Ok(request) = builder.body(Body::from(sent.body)) else {
        return;
    };

    let response = to_worker(&server, request).await;
    let head = StreamHead {
        status: response.status().as_u16(),
        headers: response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_owned())))
            .collect(),
    };
    let Ok(head) = serde_json::to_string(&head) else {
        return;
    };
    if socket.send(Message::Text(head.into())).await.is_err() {
        return;
    }

    let mut body = response.into_body().into_data_stream();
    loop {
        tokio::select! {
            chunk = body.next() => match chunk {
                Some(Ok(bytes)) => {
                    if socket.send(Message::Binary(bytes)).await.is_err() {
                        return;
                    }
                }
                // The body ended, or failed: either way the stream is over.
                _ => break,
            },
            // Anything from the page after the request is a close, or a
            // dropped connection. Ending here drops the body, which ends
            // the worker's subscription.
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}

/// Serve a file from the built UI. A path with no file behind it is an
/// app route (`/space/...`, `/join`), so it gets the shell document, as
/// the service worker's navigation handler does.
async fn serve_static(dist: &Path, method: &Method, uri: &Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    // The page registers no service worker here, and must not: one would
    // answer `/api` from IndexedDB, beside the native worker.
    if uri.path() == "/service_worker.js" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(relative) = safe_relative(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut path = dist.join(&relative);
    if path.is_dir() {
        path = path.join("index.html");
    }
    let (path, bytes) = match tokio::fs::read(&path).await {
        Ok(bytes) => (path, bytes),
        Err(_) if relative.extension().is_none() => {
            let shell = dist.join("index.html");
            match tokio::fs::read(&shell).await {
                Ok(bytes) => (shell, bytes),
                Err(_) => return StatusCode::NOT_FOUND.into_response(),
            }
        }
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&path)),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// The request path as a path under `dist`, refusing anything that could
/// step outside it.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(path.trim_start_matches('/'))?;
    let relative = PathBuf::from(decoded);
    relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(relative)
}

/// Decode `%XX` escapes; `None` for a malformed escape or non-UTF-8.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = input.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The media type for a file in the built UI. Wasm must be
/// `application/wasm` for streaming compilation.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("yaml" | "yml") => "text/yaml; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_refuses_paths_that_leave_the_dist_directory() {
        assert_eq!(safe_relative("/../etc/passwd"), None);
        assert_eq!(safe_relative("/%2e%2e/etc/passwd"), None);
        assert_eq!(safe_relative("/a/../../b"), None);
        assert_eq!(
            safe_relative("/guest/app.wasm"),
            Some(PathBuf::from("guest/app.wasm"))
        );
    }

    #[test]
    fn it_requires_the_token_only_where_the_worker_answers() {
        assert!(needs_token("/api/identify"));
        assert!(needs_token("/api"));
        assert!(!needs_token("/__tonk/stream"), "it checks the token itself");
        assert!(!needs_token("/images/tonk-wordmark.svg"));
        assert!(!needs_token("/apish"));
    }

    #[test]
    fn it_admits_a_stream_with_the_cookie_or_the_token() {
        assert!(stream_admitted(true, "", "secret"));
        assert!(stream_admitted(false, "secret", "secret"));
        assert!(!stream_admitted(false, "", "secret"));
        assert!(!stream_admitted(false, "guess", "secret"));
    }

    #[test]
    fn it_finds_the_token_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=b; tonk-desktop=xyz"),
        );
        assert!(carries_token(&headers, "xyz"));
        assert!(!carries_token(&headers, "xy"));
    }
}
