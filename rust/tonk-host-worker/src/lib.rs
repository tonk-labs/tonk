//! The Host Worker: a hosted peer for agents, reached by the MCP Worker
//! over a service binding.
//!
//! Every write to a space goes through the space writer of that space and
//! the caller's MCP session ([`SpaceWriterObject`]), a Durable Object that
//! keeps a warm [`tonk_worker::SpaceWriter`] and is the only place that
//! signs revisions for the pair.
//!
//! The front here is a development stand-in. It decodes the stage 0
//! spike's token (`base64url(seed) "." base64url(powerline)`, minted by
//! `prototypes/agent-peer-worker`'s `/_spike/setup`), takes the MCP
//! session from `Mcp-Session-Id`, and hands the request to the writer. It
//! verifies nothing beyond the powerline being addressed to the seed's
//! key: the token-verifying front (the stage 1 token, revocation on every
//! call) replaces it.
//!
//! Routes, each with `Authorization: Bearer` and optionally
//! `Mcp-Session-Id` (`default` when absent):
//!
//! - `POST /{space}/api/query`: notation, evaluated without committing.
//! - `POST /{space}/api/evaluate`: notation, committed, answered once
//!   pushed.
//! - `PUT /{space}/`: store an asset, answered once pushed.
//! - `GET /{space}/asset/{entity}`: read an asset.
//!
//! To run it locally: start `tonk-access-local` (see the `run-app` skill),
//! build with `worker-build --release` in this directory, serve with
//! `wrangler dev --var ACCESS_SERVICE_URL:<its url>`, and mint a token
//! from the spike Worker's `POST /_spike/setup` pointed at the same
//! access service.

use axum::body::Body;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Principal as _;
use http::{Response, StatusCode, header};
use worker::{Context, Env, HttpRequest, event};

mod writer;
pub use writer::SpaceWriterObject;

/// The MCP session a request without `Mcp-Session-Id` is served as.
const DEFAULT_SESSION: &str = "default";

#[event(fetch)]
async fn fetch(
    request: HttpRequest,
    env: Env,
    _context: Context,
) -> worker::Result<Response<Body>> {
    console_error_panic_hook::set_once();
    Ok(match front(request, &env).await {
        Ok(response) => response,
        Err((status, message)) => error(status, &message),
    })
}

/// Decode the caller's session and hand the request to its writer.
async fn front(request: HttpRequest, env: &Env) -> Result<Response<Body>, (StatusCode, String)> {
    if request.uri().path() == "/" {
        return Ok(json(StatusCode::OK, serde_json::json!({ "ok": true })));
    }
    let unauthorized = |message: &str| (StatusCode::UNAUTHORIZED, message.to_owned());
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("missing bearer token"))?;
    let (seed, powerline) = bearer
        .split_once('.')
        .ok_or_else(|| unauthorized("malformed token"))?;
    let seed: [u8; 32] = B64
        .decode(seed)
        .ok()
        .and_then(|seed| seed.try_into().ok())
        .ok_or_else(|| unauthorized("malformed seed"))?;
    let powerline_bytes = B64
        .decode(powerline)
        .map_err(|_| unauthorized("malformed powerline"))?;
    let chain = DelegationChain::try_from(powerline_bytes.as_slice())
        .map_err(|error| unauthorized(&format!("malformed powerline: {error}")))?;
    let session = Ed25519Signer::import(&seed)
        .await
        .map_err(|error| unauthorized(&format!("bad seed: {error}")))?
        .did();
    if chain.audience() != &session {
        return Err(unauthorized(
            "the powerline is not addressed to the session",
        ));
    }
    let space = chain.issuer().to_string();
    let mcp_session = request
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or(DEFAULT_SESSION)
        .to_owned();

    let path = request.uri().path().trim_start_matches('/').to_owned();
    let (requested, rest) = path.split_once('/').unwrap_or((path.as_str(), ""));
    if requested != space {
        // Other spaces come by invite to the agent space, through the
        // token-verifying front.
        return Err((StatusCode::FORBIDDEN, "no invite to this space".into()));
    }
    let remote = env
        .var("ACCESS_SERVICE_URL")
        .map(|origin| format!("{}/ucan/", origin.to_string().trim_end_matches('/')))
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "ACCESS_SERVICE_URL unset".to_owned(),
            )
        })?;

    // The object is named by the writer it holds, not by the MCP session
    // id alone: two OAuth sessions that happen to send one id are two
    // writers.
    let writer = Ed25519Signer::import(&*tonk_worker::writer_seed(&seed, &mcp_session))
        .await
        .map_err(|error| unauthorized(&format!("bad writer key: {error}")))?
        .did()
        .to_string();
    let identity = writer::Identity {
        writer,
        seed: B64.encode(seed),
        powerline: powerline.to_owned(),
        mcp_session: mcp_session.clone(),
        space: space.clone(),
        remote,
    };
    writer::forward(env, &identity, request, rest)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

fn json(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .expect("static response")
}

fn error(status: StatusCode, message: &str) -> Response<Body> {
    json(status, serde_json::json!({ "error": message }))
}
