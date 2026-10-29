//! `/space-api/`: what a view's guest sends, with the view ticket the show
//! tool sealed (see [`tonk_worker::view`]).
//!
//! The ticket is opened, its bearer verified again as `/mcp` would, and
//! the request handed to the writer of the ticket's space and MCP session,
//! the same object the agent's own tool calls reach. Only routes of the
//! ticket's space's branch are served, and a space the token reaches for
//! reading only takes no write.

use axum::body::Body;
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::{Did, Principal as _};
use http::{HeaderValue, Method, Request, Response, StatusCode, header};
use tonk_worker::view::TicketKey;
use tonk_worker::{SpaceWriter, writer_seed};
use worker::Env;

use crate::writer;

/// Where views send their guests' requests.
pub(crate) const PREFIX: &str = "/space-api/";

/// The branch views reach.
const BRANCH: &str = "main";

/// Answer one view request.
pub(crate) async fn serve(
    request: Request<Body>,
    env: &Env,
    tickets: &TicketKey,
    remote: &str,
) -> Response<Body> {
    if request.method() == Method::OPTIONS {
        return cors(Response::builder().status(StatusCode::NO_CONTENT))
            .header(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS",
            )
            .header(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Authorization, Content-Type, Accept",
            )
            .header(header::ACCESS_CONTROL_MAX_AGE, "600")
            .body(Body::empty())
            .expect("static response");
    }
    match route(request, env, tickets, remote).await {
        Ok(response) => with_cors(response),
        Err((status, message)) => with_cors(failure(status, message)),
    }
}

async fn route(
    request: Request<Body>,
    env: &Env,
    tickets: &TicketKey,
    remote: &str,
) -> Result<Response<Body>, (StatusCode, String)> {
    let sealed = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "missing view ticket".to_owned()))?;
    let ticket = tickets
        .open(sealed, Timestamp::now().to_unix())
        .map_err(|error| (StatusCode::UNAUTHORIZED, error))?;
    let (space, path) = branch_path(request.uri().path(), &ticket.space)?;
    let verified = tonk_worker::mcp::verify(&ticket.bearer)
        .await
        .map_err(|error| (StatusCode::UNAUTHORIZED, error))?;
    let access = verified.spaces().get(&space).ok_or((
        StatusCode::FORBIDDEN,
        "the token no longer reaches this space".to_owned(),
    ))?;
    if !access.agent && !access.read {
        return Err((
            StatusCode::FORBIDDEN,
            "access to this space lapsed".to_owned(),
        ));
    }
    if !access.agent && !access.write && !SpaceWriter::is_read(&request) {
        return Err((
            StatusCode::FORBIDDEN,
            "this connection can only read this space".to_owned(),
        ));
    }
    let writer = Ed25519Signer::import(&*writer_seed(verified.token().seed(), &ticket.mcp_session))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("bad writer key: {error}"),
            )
        })?
        .did()
        .to_string();
    let identity = writer::Identity {
        writer,
        token: ticket.bearer.clone(),
        mcp_session: ticket.mcp_session.clone(),
        space: space.to_string(),
        remote: remote.to_owned(),
    };
    writer::forward(env, &identity, request, &format!("view/{path}"))
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

/// The ticket's space and the route under its branch that `path` names:
/// `/space-api/api/repository/{space}/branch/main/{route}`, each segment
/// decoded on its own, so an encoded slash cannot reach another space or
/// leave the branch.
fn branch_path(path: &str, space: &str) -> Result<(Did, String), (StatusCode, String)> {
    let outside = || {
        (
            StatusCode::FORBIDDEN,
            "outside this view's space".to_owned(),
        )
    };
    let rest = path.strip_prefix(PREFIX).ok_or_else(outside)?;
    let parts = rest
        .split('/')
        .map(|part| {
            percent_decode(part).ok_or((StatusCode::BAD_REQUEST, "malformed path".to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match parts.as_slice() {
        [api, repository, subject, branch, name, route @ ..]
            if api == "api"
                && repository == "repository"
                && subject == space
                && branch == "branch"
                && name == BRANCH
                && !route.is_empty()
                && route.iter().all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && !part.contains(['/', '\\'])
                        && !part.chars().any(char::is_control)
                }) =>
        {
            let space = space.parse().map_err(|_| outside())?;
            let route = route
                .iter()
                .map(|part| percent_encode(part))
                .collect::<Vec<_>>()
                .join("/");
            Ok((space, route))
        }
        _ => Err(outside()),
    }
}

fn percent_decode(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = part.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(part: &str) -> String {
    part.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'@' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn cors(builder: http::response::Builder) -> http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "no-store")
}

fn with_cors(mut response: Response<Body>) -> Response<Body> {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn failure(status: StatusCode, message: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({ "error": message }).to_string(),
        ))
        .expect("static response")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

    #[test]
    fn it_takes_a_route_of_the_tickets_space() {
        let path = format!("/space-api/api/repository/{SPACE}/branch/main/query");
        assert_eq!(branch_path(&path, SPACE).unwrap().1, "query");
        let encoded = path.replace(':', "%3A");
        assert_eq!(branch_path(&encoded, SPACE).unwrap().1, "query");
    }

    #[test]
    fn it_refuses_another_space_or_branch() {
        let other = "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuBV8xRoAnwWsdvktH";
        let path = format!("/space-api/api/repository/{other}/branch/main/query");
        assert_eq!(
            branch_path(&path, SPACE).unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        let path = format!("/space-api/api/repository/{SPACE}/branch/other/query");
        assert_eq!(
            branch_path(&path, SPACE).unwrap_err().0,
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn it_refuses_a_route_that_leaves_the_branch() {
        for route in ["..%2F..%2Fx", "%2E%2E", "a%2Fb", "", "a//b"] {
            let path = format!("/space-api/api/repository/{SPACE}/branch/main/{route}");
            assert!(branch_path(&path, SPACE).is_err(), "{route}");
        }
    }
}
