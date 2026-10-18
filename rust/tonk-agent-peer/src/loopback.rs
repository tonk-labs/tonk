//! The owner's worker API on a loopback port, for Tonk views rendered in the
//! host's conversation (MCP Apps). Spike: no authentication yet, so any local
//! process or page that finds the port acts as this peer.
//!
//! The view page relays the sealed guest's requests here, the way the browser
//! relays them to its service worker. Besides the worker routes, it serves the
//! guest runtime pack (`space-runtime.json`) and the guest document
//! (`worker-guest.html`) from the runtime directory shipped beside the binary.
use std::path::PathBuf;

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use tokio::{net::TcpListener, task::JoinHandle};

/// Bind a loopback port and serve `router` on it. Returns the origin and the
/// serving task.
pub async fn start(router: Router) -> std::io::Result<(String, JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let origin = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let app = Router::new()
        .route("/space-runtime.json", get(|| asset("space-runtime.json")))
        .route("/worker-guest.html", get(|| asset("worker-guest.html")))
        .route("/agent-peer/probe", get(|| async { "ok" }))
        .merge(router)
        .layer(middleware::from_fn(client_id))
        .layer(middleware::from_fn(cors));
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((origin, task))
}

/// Where the guest runtime lives: `TONK_AGENT_PEER_RUNTIME`, else `runtime/`
/// beside the binary (how the extension ships it).
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("TONK_AGENT_PEER_RUNTIME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()
                .map(|dir| dir.join("runtime"))
        })
}

async fn asset(name: &'static str) -> Response {
    let content_type = if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else {
        "application/json"
    };
    match runtime_dir().map(|dir| dir.join(name)) {
        Some(path) => match tokio::fs::read(&path).await {
            Ok(bytes) => Response::builder()
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(bytes))
                .expect("asset response"),
            Err(_) => missing(name),
        },
        None => missing(name),
    }
}

fn missing(name: &str) -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from(format!(
            "The Tonk guest runtime ({name}) is not installed beside this peer."
        )))
        .expect("missing response")
}

/// The worker tags subscriptions and site entities by client; the view page
/// names itself with `x-tonk-client-id`, as the hosted worker expects.
async fn client_id(mut request: Request, next: Next) -> Response {
    if let Some(client) = request
        .headers()
        .get("x-tonk-client-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    {
        request
            .extensions_mut()
            .insert(tonk_worker::ClientId(client));
    }
    next.run(request).await
}

/// The view page runs at an opaque origin inside the host, so every response
/// allows any origin, and preflights include Private Network Access.
async fn cors(request: Request, next: Next) -> Response {
    let asked = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .cloned();
    let mut response = if request.method() == Method::OPTIONS {
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("preflight response")
    } else {
        next.run(request).await
    };
    allow(response.headers_mut(), asked);
    response
}

fn allow(headers: &mut HeaderMap, asked: Option<HeaderValue>) {
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        asked.unwrap_or(HeaderValue::from_static("*")),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
}
