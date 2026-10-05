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
//!   `SameSite=Strict` cookie, and every later request must carry it.
//!   Another page cannot read the cookie, and its cross-site requests
//!   arrive without it.
//!
//! The server sends no CORS headers, so a browser lets no other origin
//! read a response even when a request does arrive.
//!
//! Sealed guest frames (opaque origins) never fetch from here directly:
//! the portal bootstrap relays their fetches through the top document,
//! so every request comes from the window's own origin, cookie included.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
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
        format!("http://{}/__tonk/launch?token={}", self.authority, self.token)
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
async fn guard(State(server): State<Server>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if host != Some(server.authority.as_str()) {
        return (StatusCode::MISDIRECTED_REQUEST, "unexpected host").into_response();
    }
    if request.uri().path() != "/__tonk/launch"
        && !carries_token(request.headers(), &server.token)
    {
        return (StatusCode::FORBIDDEN, "not this window").into_response();
    }
    next.run(request).await
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
    let cookie = format!("{COOKIE}={}; Path=/; HttpOnly; SameSite=Strict", server.token);
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
async fn dispatch(State(server): State<Server>, mut request: Request) -> Response {
    if request.uri().path().starts_with("/api/") || request.uri().path() == "/api" {
        let extensions = request.extensions_mut();
        extensions.insert(ClientId(CLIENT_ID.to_owned()));
        match RequestOrigin::parse(&format!("{}/", server.origin())) {
            Ok(origin) => {
                extensions.insert(origin);
            }
            Err(_) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, "bad origin").into_response();
            }
        }
        return match server.worker.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
    }
    serve_static(&server.dist, request.method(), request.uri()).await
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
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type(&path)));
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
        assert_eq!(safe_relative("/guest/app.wasm"), Some(PathBuf::from("guest/app.wasm")));
    }

    #[test]
    fn it_finds_the_token_among_other_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("a=b; tonk-desktop=xyz"));
        assert!(carries_token(&headers, "xyz"));
        assert!(!carries_token(&headers, "xy"));
    }
}
