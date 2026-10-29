//! The Host Worker: the hosted agent's `/mcp`, reached from the access
//! service over a service binding, so `/mcp` is served on the provider's
//! own origin (the OAuth resource the authorization server issues tokens
//! for).
//!
//! Every write to a space goes through the space writer of that space and
//! the caller's MCP session ([`SpaceWriterObject`]), a Durable Object that
//! keeps a warm [`tonk_worker::SpaceWriter`] and is the only place that
//! signs revisions for the pair.
//!
//! The front ([`tonk_worker::mcp`]) decodes the agent session token,
//! verifies its grants, mints and checks `Mcp-Session-Id`, and answers MCP
//! JSON-RPC. Tool calls on a space go to the writer named by the space
//! and the session's writer DID.
//!
//! Bindings:
//!
//! - `SPACE_WRITER`: the writers' Durable Object namespace;
//! - `ACCESS_SERVICE_URL`: the access service the writers sync with;
//! - `MCP_SESSION_SECRET`: the secret `Mcp-Session-Id`s are MAC'd under.
//!
//! To run it locally: start `tonk-access-local` (see the `run-app` skill),
//! build with `worker-build --release` in this directory, and serve with
//! `wrangler dev --var ACCESS_SERVICE_URL:<its url> --var
//! MCP_SESSION_SECRET:<anything>`.

use axum::body::Body;
use dialog_credentials::Ed25519Signer;
use dialog_varsig::{Did, Principal as _};
use http::{Response, StatusCode, header};
use tonk_worker::mcp::{McpEndpoint, SessionKey, Verified, Writers};
use tonk_worker::view::TicketKey;
use worker::{Context, Env, HttpRequest, event};

mod stateless;
mod writer;
pub use writer::SpaceWriterObject;

#[event(fetch)]
async fn fetch(
    request: HttpRequest,
    env: Env,
    _context: Context,
) -> worker::Result<Response<Body>> {
    console_error_panic_hook::set_once();
    let path = request.uri().path().to_owned();
    if path == "/" {
        return Ok(json(StatusCode::OK, serde_json::json!({ "ok": true })));
    }
    if path != "/mcp" && path != "/mcp/" {
        return Ok(json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "no such route" }),
        ));
    }
    let Ok(secret) = env
        .secret("MCP_SESSION_SECRET")
        .map(|secret| secret.to_string())
    else {
        return Ok(json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": "MCP_SESSION_SECRET unset" }),
        ));
    };
    let Some(remote) = remote(&env) else {
        return Ok(json(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": "ACCESS_SERVICE_URL unset" }),
        ));
    };
    // The origin the client reached: the access service forwards the
    // request as it came, so this is the provider's.
    let origin = match (request.uri().scheme_str(), request.uri().authority()) {
        (Some(scheme), Some(authority)) => format!("{scheme}://{authority}"),
        _ => remote.trim_end_matches("/ucan/").to_owned(),
    };
    let key = SessionKey::derive(&secret);
    let tickets = TicketKey::derive(&secret);
    let writers = DurableWriters { env: &env, remote };
    let endpoint = McpEndpoint {
        origin: &origin,
        key: &key,
        tickets: &tickets,
        writers: &writers,
    };
    let (parts, body) = request.into_parts();
    let request = http::Request::from_parts(parts, Body::new(body));
    Ok(endpoint.serve(request).await)
}

/// The access service's `/ucan/`, which the writers sync with.
fn remote(env: &Env) -> Option<String> {
    env.var("ACCESS_SERVICE_URL")
        .ok()
        .map(|origin| format!("{}/ucan/", origin.to_string().trim_end_matches('/')))
}

/// The space writers, one Durable Object per space and writer DID.
struct DurableWriters<'a> {
    env: &'a Env,
    remote: String,
}

impl Writers for DurableWriters<'_> {
    async fn call(
        &self,
        verified: &Verified,
        bearer: &str,
        mcp_session: &str,
        space: &Did,
        request: http::Request<Body>,
    ) -> Response<Body> {
        // A read that may scan runs here rather than raise the writer's
        // memory, which never comes back down.
        let request = if stateless::is_read(&request) {
            let (parts, body) = request.into_parts();
            let body = match axum::body::to_bytes(body, usize::MAX).await {
                Ok(body) => body,
                Err(error) => {
                    return json(
                        StatusCode::BAD_REQUEST,
                        serde_json::json!({ "error": error.to_string() }),
                    );
                }
            };
            if stateless::runs_here(&parts.uri, &body) {
                let request = http::Request::from_parts(parts, Body::from(body));
                return match stateless::read(verified, mcp_session, space, &self.remote, request)
                    .await
                {
                    Ok(response) => response,
                    Err(error) => json(
                        StatusCode::BAD_GATEWAY,
                        serde_json::json!({ "error": error, "code": "READ_UNAVAILABLE" }),
                    ),
                };
            }
            http::Request::from_parts(parts, Body::from(body))
        } else {
            request
        };
        // The object is named by the writer it holds, not by the MCP
        // session id alone: two OAuth sessions that happen to send one id
        // are two writers.
        let writer = match Ed25519Signer::import(&*tonk_worker::writer_seed(
            verified.token().seed(),
            mcp_session,
        ))
        .await
        {
            Ok(signer) => signer.did().to_string(),
            Err(error) => {
                return json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": format!("bad writer key: {error}") }),
                );
            }
        };
        let identity = writer::Identity {
            writer,
            token: bearer.to_owned(),
            mcp_session: mcp_session.to_owned(),
            space: space.to_string(),
            remote: self.remote.clone(),
        };
        let path = request.uri().path().trim_start_matches('/').to_owned();
        match writer::forward(self.env, &identity, request, &path).await {
            Ok(response) => response,
            Err(error) => json(
                StatusCode::BAD_GATEWAY,
                serde_json::json!({ "error": error.to_string(), "code": "WRITER_UNREACHABLE" }),
            ),
        }
    }
}

fn json(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .expect("static response")
}
