//! Spike: the dialog peer, built per request inside a Workers isolate.
//!
//! Every request carries a token (session seed plus powerline). The handler
//! builds the ordinary tonk worker over in-memory storage acting as the
//! session, mounts the space as a replica of the access service, pulls,
//! serves the request with the worker's own routes, pushes any write, and
//! drops everything. Nothing survives the request.
//!
//! Routes (all but `/` and `/_spike/setup` take `Authorization: Bearer`):
//!
//! - `GET /` — liveness and the isolate's wasm memory.
//! - `POST /_spike/setup` — mint a token from the hardcoded agent space and
//!   session seeds, and provision the agent space on the local access service.
//! - `GET /spaces` — the spaces the token reaches.
//! - `POST /{space}/api/query` — notation, evaluated without committing.
//! - `POST /{space}/api/evaluate` — notation, committed and pushed.
//! - `POST /{space}/api/subscribe?ms=N` — a wire query over SSE; the frames
//!   that arrive within N ms are returned.
//! - `PUT /{space}/` — store an asset.
//! - `GET /{space}/asset/{entity}` — read an asset.
//!
//! Each response carries `x-spike-timing`: milliseconds per phase and the
//! wasm linear memory before and after.

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal as _};
use futures_util::StreamExt as _;
use http::{Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use tonk_worker::HostedPeer;
use worker::{Context, Env, HttpRequest, event};

/// The agent space's key. Only `/_spike/setup` uses it, to mint the
/// powerline; the request path never sees it, as in the design.
fn agent_space_seed() -> [u8; 32] {
    *blake3::hash(b"tonk agent peer spike: agent space").as_bytes()
}

/// The key of session `n`, which the token carries. Session 0 is the
/// default; others stand for further OAuth authorizations of one agent.
fn session_seed(n: u32) -> [u8; 32] {
    if n == 0 {
        return *blake3::hash(b"tonk agent peer spike: session").as_bytes();
    }
    *blake3::hash(format!("tonk agent peer spike: session {n}").as_bytes()).as_bytes()
}

fn wasm_memory() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) * 65_536
    }
    #[cfg(not(target_arch = "wasm32"))]
    0
}

fn now() -> f64 {
    js_sys::Date::now()
}

/// Phase timings for one request.
struct Timing {
    start: f64,
    last: f64,
    memory_before: usize,
    phases: Vec<(&'static str, f64)>,
}

impl Timing {
    fn new() -> Self {
        let start = now();
        Self {
            start,
            last: start,
            memory_before: wasm_memory(),
            phases: Vec::new(),
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let at = now();
        self.phases.push((phase, at - self.last));
        self.last = at;
    }

    fn header(&self) -> String {
        let mut phases = serde_json::Map::new();
        for (name, ms) in &self.phases {
            phases.insert((*name).into(), serde_json::json!(ms));
        }
        serde_json::json!({
            "total_ms": now() - self.start,
            "phases": phases,
            "wasm_memory_before": self.memory_before,
            "wasm_memory_after": wasm_memory(),
        })
        .to_string()
    }
}

/// A decoded token: the session's seed and the powerline it acts under.
struct Token {
    seed: [u8; 32],
    powerline: DelegationChain,
    /// The agent space: the powerline's issuer.
    space: Did,
}

/// The spike's token: `base64url(seed) "." base64url(powerline container)`.
/// Stage 1 replaces it with the UCAN container the design describes.
async fn decode_token(request: &Request<Body>) -> Result<Token, String> {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or("missing bearer token")?;
    let (seed, powerline) = bearer.split_once('.').ok_or("malformed token")?;
    let seed: [u8; 32] = B64
        .decode(seed)
        .map_err(|_| "malformed seed")?
        .try_into()
        .map_err(|_| "malformed seed")?;
    let powerline = DelegationChain::try_from(
        B64.decode(powerline)
            .map_err(|_| "malformed powerline")?
            .as_slice(),
    )
    .map_err(|error| format!("malformed powerline: {error}"))?;
    let session = Ed25519Signer::import(&seed)
        .await
        .map_err(|error| format!("bad seed: {error}"))?
        .did();
    if powerline.audience() != &session {
        return Err("powerline is not addressed to the session".into());
    }
    if powerline.subject().is_some() || !powerline.ability().trim_matches('/').is_empty() {
        return Err("not a powerline".into());
    }
    let space = powerline.issuer().clone();
    Ok(Token {
        seed,
        powerline,
        space,
    })
}

fn json(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .expect("static response")
}

fn error(status: StatusCode, message: impl std::fmt::Display) -> Response<Body> {
    json(status, serde_json::json!({ "error": message.to_string() }))
}

/// The `/ucan/` endpoint the hosted peer syncs through.
fn remote(env: &Env) -> Result<url::Url, String> {
    let origin = env
        .var("ACCESS_SERVICE_URL")
        .map_err(|_| "ACCESS_SERVICE_URL unset")?
        .to_string();
    url::Url::parse(&format!("{}/ucan/", origin.trim_end_matches('/'))).map_err(|e| e.to_string())
}

/// Send one request to the worker router and buffer its answer.
async fn call(
    peer: &mut HostedPeer,
    method: &str,
    path: &str,
    content_type: &str,
    body: impl Into<Body>,
) -> (StatusCode, http::HeaderMap, bytes::Bytes) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .body(body.into())
        .expect("static request");
    let response = peer.call(request).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    (status, headers, body)
}

fn failed(step: &str, status: StatusCode, body: &[u8]) -> Response<Body> {
    error(
        StatusCode::BAD_GATEWAY,
        format!("{step}: {status}: {}", String::from_utf8_lossy(body)),
    )
}

#[event(fetch)]
async fn fetch(
    request: HttpRequest,
    env: Env,
    _context: Context,
) -> worker::Result<Response<Body>> {
    console_error_panic_hook::set_once();
    let mut timing = Timing::new();
    let (parts, body) = request.into_parts();
    let body = BodyExt::collect(body)
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    let request = Request::from_parts(parts, Body::from(body.clone()));
    timing.mark("read_body");
    let mut response = route(request, body, &env, &mut timing).await;
    if let Ok(value) = timing.header().parse() {
        response.headers_mut().insert("x-spike-timing", value);
    }
    Ok(response)
}

async fn route(
    request: Request<Body>,
    body: bytes::Bytes,
    env: &Env,
    timing: &mut Timing,
) -> Response<Body> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    let method = request.method().clone();
    if path == "/" {
        return json(
            StatusCode::OK,
            serde_json::json!({ "ok": true, "wasm_memory": wasm_memory() }),
        );
    }
    if path == "/_spike/setup" && method == http::Method::POST {
        let session = query
            .split('&')
            .find_map(|pair| pair.strip_prefix("session="))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        return setup(env, session).await;
    }
    let token = match decode_token(&request).await {
        Ok(token) => token,
        Err(message) => return error(StatusCode::UNAUTHORIZED, message),
    };
    timing.mark("decode_token");
    let remote = match remote(env) {
        Ok(remote) => remote,
        Err(message) => return error(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    if path == "/spaces" {
        return json(
            StatusCode::OK,
            serde_json::json!({ "spaces": [ { "space": token.space.to_string(), "role": "agent" } ] }),
        );
    }
    let segments: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
    let space = segments[0].to_owned();
    let rest = segments.get(1).copied().unwrap_or("").to_owned();
    if space != token.space.to_string() {
        // Other spaces come by invite to the agent space; none in the spike.
        return error(StatusCode::FORBIDDEN, "no invite to this space");
    }

    // A per-request name keeps concurrent requests in one isolate apart.
    let name = format!("{:016x}", (js_sys::Math::random() * 1e16) as u64);
    let mut peer = match HostedPeer::open(&name, &token.seed, &[token.powerline.clone()]).await {
        Ok(peer) => peer,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, format!("open: {e}")),
    };
    timing.mark("open_peer");
    if let Err(e) = peer.mount(&token.space, &remote).await {
        peer.close().await;
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("mount: {e}"));
    }
    timing.mark("mount");
    let base = format!("/api/repository/{space}/branch/main");
    let (status, _, pulled) = call(
        &mut peer,
        "POST",
        &format!("{base}/sync/pull"),
        "text/plain",
        "",
    )
    .await;
    timing.mark("pull");
    if !status.is_success() {
        peer.close().await;
        return failed("pull", status, &pulled);
    }

    let response = match (method.as_str(), rest.as_str()) {
        ("POST", "api/query") => {
            let (status, _, body) = call(
                &mut peer,
                "POST",
                &format!("{base}/evaluate?transact=false"),
                "text/plain",
                body,
            )
            .await;
            timing.mark("query");
            passthrough(status, "application/json", body)
        }
        ("POST", "api/evaluate") => {
            let (status, _, result) = call(
                &mut peer,
                "POST",
                &format!("{base}/evaluate?transact=true"),
                "text/plain",
                body,
            )
            .await;
            timing.mark("evaluate");
            if !status.is_success() {
                passthrough(status, "application/json", result)
            } else {
                push_then(&mut peer, &base, timing, status, "application/json", result).await
            }
        }
        ("POST", "api/subscribe") => subscribe(&mut peer, &base, &query, body, timing).await,
        ("PUT", "") => {
            let content_type = request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_owned();
            let (status, _, result) = call(
                &mut peer,
                "POST",
                &format!("{base}/blob"),
                &content_type,
                body,
            )
            .await;
            timing.mark("store_asset");
            if !status.is_success() {
                passthrough(status, "application/json", result)
            } else {
                push_then(&mut peer, &base, timing, status, "application/json", result).await
            }
        }
        ("GET", rest) if rest.starts_with("asset/") => {
            let entity = &rest["asset/".len()..];
            let (status, headers, result) =
                call(&mut peer, "GET", &format!("{base}/blob/{entity}"), "", "").await;
            timing.mark("read_asset");
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("application/octet-stream")
                .to_owned();
            passthrough(status, &content_type, result)
        }
        _ => error(StatusCode::NOT_FOUND, "no such route"),
    };
    peer.close().await;
    timing.mark("close");
    response
}

fn passthrough(status: StatusCode, content_type: &str, body: bytes::Bytes) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("static response")
}

async fn push_then(
    peer: &mut HostedPeer,
    base: &str,
    timing: &mut Timing,
    status: StatusCode,
    content_type: &str,
    result: bytes::Bytes,
) -> Response<Body> {
    let (pushed, _, body) =
        call(peer, "POST", &format!("{base}/sync/push"), "text/plain", "").await;
    timing.mark("push");
    if !pushed.is_success() {
        return failed("push", pushed, &body);
    }
    passthrough(status, content_type, result)
}

/// Open the query as an SSE subscription and gather what arrives within
/// `ms` (default 500). A hosted peer holds no live feed of remote commits,
/// so only frames this peer itself causes can follow the first.
async fn subscribe(
    peer: &mut HostedPeer,
    base: &str,
    query: &str,
    body: bytes::Bytes,
    timing: &mut Timing,
) -> Response<Body> {
    let ms: u64 = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("ms="))
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(500);
    let request = Request::builder()
        .method("POST")
        .uri(format!("{base}/query"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "text/event-stream")
        .body(Body::from(body))
        .expect("static request");
    let response = peer.call(request).await;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .into_body()
            .collect()
            .await
            .map(|b| b.to_bytes())
            .unwrap_or_default();
        return passthrough(status, "application/json", body);
    }
    let mut stream = response.into_body().into_data_stream();
    let mut frames = Vec::new();
    let deadline = now() + ms as f64;
    loop {
        let remaining = deadline - now();
        if remaining <= 0.0 {
            break;
        }
        let next = stream.next();
        let delay = worker::Delay::from(std::time::Duration::from_millis(remaining as u64));
        futures_util::pin_mut!(next, delay);
        match futures_util::future::select(next, delay).await {
            futures_util::future::Either::Left((Some(Ok(chunk)), _)) => {
                frames.push(String::from_utf8_lossy(&chunk).into_owned());
                if frames.len() == 1 {
                    timing.mark("first_frame");
                }
            }
            futures_util::future::Either::Left(_) => break,
            futures_util::future::Either::Right(_) => break,
        }
    }
    timing.mark("subscribe");
    json(StatusCode::OK, serde_json::json!({ "frames": frames }))
}

/// Mint the spike's token and provision the agent space on the local
/// access service (`/_test/provision`, the native test server's shortcut).
async fn setup(env: &Env, session_index: u32) -> Response<Body> {
    let remote = match remote(env) {
        Ok(remote) => remote,
        Err(message) => return error(StatusCode::INTERNAL_SERVER_ERROR, message),
    };
    let space = match Ed25519Signer::import(&agent_space_seed()).await {
        Ok(signer) => signer,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let session = match Ed25519Signer::import(&session_seed(session_index)).await {
        Ok(signer) => signer,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let powerline = match tonk_identity::delegation::mint_addressed_device_delegation(
        space.clone(),
        &session.did(),
        &remote,
    )
    .await
    {
        Ok(chain) => chain,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let bytes = match powerline.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let origin = remote
        .as_str()
        .trim_end_matches("ucan/")
        .trim_end_matches('/');
    let mut init = worker::RequestInit::new();
    init.with_method(worker::Method::Post);
    let headers = worker::Headers::new();
    let _ = headers.set("content-type", "application/json");
    init.with_headers(headers);
    init.with_body(Some(
        serde_json::json!({ "subject": space.did().to_string() })
            .to_string()
            .into(),
    ));
    let provisioned =
        match worker::Request::new_with_init(&format!("{origin}/_test/provision"), &init) {
            Ok(request) => match worker::Fetch::Request(request).send().await {
                Ok(response) => response.status_code(),
                Err(e) => return error(StatusCode::BAD_GATEWAY, format!("provision: {e}")),
            },
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
    json(
        StatusCode::OK,
        serde_json::json!({
            "space": space.did().to_string(),
            "session": session.did().to_string(),
            "provisioned": provisioned,
            "token": format!("{}.{}", B64.encode(session_seed(session_index)), B64.encode(bytes)),
        }),
    )
}
