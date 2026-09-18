//! The shaping, logging front server.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use futures::stream;
use serde::Serialize;
use sha2_0_10::{Digest, Sha256};

/// The same prefixes dev:web proxies to the access service: the browser
/// reads its deployment config from /.well-known/tonk (index.html there
/// parses as "deployment configuration is invalid"), resolves the
/// service identity via did.json, the account panel reads /customer/,
/// and /@ is the invite shortcut.
const PROXIED_PREFIXES: [&str; 5] = [
    "/ucan",
    "/.well-known/tonk",
    "/.well-known/did.json",
    "/customer/",
    "/@",
];

/// Hop-by-hop headers that must not be forwarded either direction.
const HOP_HEADERS: [&str; 4] = ["connection", "keep-alive", "transfer-encoding", "host"];

struct Shaper {
    ucan: String,
    root: PathBuf,
    latency_ms: u64,
    bandwidth_kbps: u64,
    log: Mutex<File>,
    seq: AtomicU64,
    inflight: AtomicI64,
    client: reqwest::Client,
}

#[derive(Serialize)]
struct Record<'a> {
    seq: u64,
    t0: u64,
    t1: u64,
    dur_ms: u64,
    inflight: i64,
    method: &'a str,
    path: &'a str,
    status: u16,
    req_bytes: usize,
    resp_bytes: usize,
    resp_sha: String,
}

pub fn run(
    listen: u16,
    ucan: String,
    root: PathBuf,
    latency_ms: u64,
    bandwidth_kbps: u64,
    log: PathBuf,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(serve(listen, ucan, root, latency_ms, bandwidth_kbps, log))
}

async fn serve(
    listen: u16,
    ucan: String,
    root: PathBuf,
    latency_ms: u64,
    bandwidth_kbps: u64,
    log: PathBuf,
) -> anyhow::Result<()> {
    let shaper = Arc::new(Shaper {
        ucan: ucan.trim_end_matches('/').to_string(),
        root,
        latency_ms,
        bandwidth_kbps,
        log: Mutex::new(OpenOptions::new().create(true).append(true).open(&log)?),
        seq: AtomicU64::new(0),
        inflight: AtomicI64::new(0),
        // The invite shortcut answers a 301 whose relative Location the
        // BROWSER must resolve against this origin; following it here
        // would chase it against the access service instead.
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    });

    let app = axum::Router::new()
        .fallback(handle)
        .with_state(shaper.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", listen)).await?;
    println!(
        "PROXY_READY http://127.0.0.1:{listen} ucan->{} latency={}ms bw={}kbps",
        shaper.ucan,
        latency_ms,
        if bandwidth_kbps == 0 {
            "unlimited".to_string()
        } else {
            bandwidth_kbps.to_string()
        }
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn handle(
    State(shaper): State<Arc<Shaper>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let seq = shaper.seq.fetch_add(1, Ordering::Relaxed) + 1;
    let inflight = shaper.inflight.fetch_add(1, Ordering::Relaxed) + 1;
    let t0 = crate::now_ms();

    // Half the round trip on the way in, half before the response.
    if shaper.latency_ms > 0 {
        tokio::time::sleep(Duration::from_millis(shaper.latency_ms / 2)).await;
    }

    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let path = uri.path().to_string();

    let (status, payload, resp_headers) = if PROXIED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        forward(&shaper, &method, &path_and_query, &headers, &body).await
    } else if method == Method::GET || method == Method::HEAD {
        serve_static(&shaper.root, &path).await
    } else {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            Bytes::from_static(b"method not allowed"),
            Vec::new(),
        )
    };

    if shaper.latency_ms > 0 {
        tokio::time::sleep(Duration::from_millis(shaper.latency_ms.div_ceil(2))).await;
    }

    let resp_sha = {
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let digest = hasher.finalize();
        digest[..8].iter().map(|b| format!("{b:02x}")).collect()
    };
    let record = Record {
        seq,
        t0,
        t1: crate::now_ms(),
        dur_ms: crate::now_ms().saturating_sub(t0),
        inflight,
        method: method.as_str(),
        path: &path_and_query,
        status: status.as_u16(),
        req_bytes: body.len(),
        resp_bytes: payload.len(),
        resp_sha,
    };
    if let Ok(mut log) = shaper.log.lock()
        && let Ok(line) = serde_json::to_string(&record)
    {
        let _ = writeln!(log, "{line}");
    }
    shaper.inflight.fetch_sub(1, Ordering::Relaxed);

    let mut response = Response::builder().status(status);
    for (name, value) in resp_headers {
        response = response.header(name, value);
    }
    let body = if method == Method::HEAD {
        Body::empty()
    } else if shaper.bandwidth_kbps > 0 {
        throttled_body(payload, shaper.bandwidth_kbps)
    } else {
        Body::from(payload)
    };
    response.body(body).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("empty response builds")
    })
}

/// Stream `payload` in 16 KiB chunks paced to `kbps`.
fn throttled_body(payload: Bytes, kbps: u64) -> Body {
    const CHUNK: usize = 16 * 1024;
    // kbps -> bytes/ms is kbps / 8; per-chunk delay in ms.
    let delay = Duration::from_millis((CHUNK as u64 * 8) / kbps.max(1));
    let chunks: Vec<Bytes> = payload.chunks(CHUNK).map(Bytes::copy_from_slice).collect();
    Body::from_stream(stream::unfold(
        chunks.into_iter(),
        move |mut chunks| async move {
            let chunk = chunks.next()?;
            tokio::time::sleep(delay).await;
            Some((Ok::<_, std::convert::Infallible>(chunk), chunks))
        },
    ))
}

async fn forward(
    shaper: &Shaper,
    method: &Method,
    path_and_query: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> (StatusCode, Bytes, Vec<(String, HeaderValue)>) {
    let url = format!("{}{path_and_query}", shaper.ucan);
    let mut request = shaper
        .client
        .request(method.clone(), &url)
        .body(body.clone());
    for (name, value) in headers {
        if !HOP_HEADERS.contains(&name.as_str()) {
            request = request.header(name, value);
        }
    }
    match request.send().await {
        Ok(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let resp_headers = response
                .headers()
                .iter()
                .filter(|(name, _)| {
                    !HOP_HEADERS.contains(&name.as_str()) && name.as_str() != "content-length"
                })
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect();
            let payload = response.bytes().await.unwrap_or_default();
            (status, payload, resp_headers)
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Bytes::from(error.to_string()),
            Vec::new(),
        ),
    }
}

async fn serve_static(root: &Path, path: &str) -> (StatusCode, Bytes, Vec<(String, HeaderValue)>) {
    let index = root.join("index.html");
    let relative = path.trim_start_matches('/');
    // Resolve inside the root; anything escaping it (or missing) falls
    // back to the SPA shell, mirroring `try_files {path} /index.html`.
    let candidate = if relative.is_empty() {
        index.clone()
    } else {
        let joined = root.join(relative);
        match (joined.canonicalize(), root.canonicalize()) {
            (Ok(resolved), Ok(root)) if resolved.starts_with(&root) && resolved.is_file() => {
                resolved
            }
            _ => index.clone(),
        }
    };
    let served = if candidate.is_file() {
        candidate
    } else {
        index
    };
    match tokio::fs::read(&served).await {
        Ok(bytes) => {
            let content_type = HeaderValue::from_static(mime_for(&served));
            (
                StatusCode::OK,
                Bytes::from(bytes),
                vec![("content-type".to_string(), content_type)],
            )
        }
        Err(_) => (
            StatusCode::NOT_FOUND,
            Bytes::from_static(b"not found"),
            Vec::new(),
        ),
    }
}

fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript",
        Some("css") => "text/css",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("yaml" | "yml") => "application/yaml",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
