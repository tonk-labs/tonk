//! Worker-side glue for the OAuth authorization server.
//!
//! The protocol lives in [`crate::oauth`]; this binds it to the service
//! seed, and writes its answers out.

use crate::oauth::client::ClientKey;
use crate::oauth::{Answer, AuthorizationServer, Endpoint, MAX_BODY, Request as OAuthRequest};
use worker::{Env, Headers, Request, Response, Result};

/// Answer a request at one of the authorization server's endpoints.
pub async fn serve(mut req: Request, env: Env, endpoint: Endpoint) -> Result<Response> {
    let url = req.url()?;
    let origin = url.origin().ascii_serialization();
    let method = req.method().to_string();
    let header = |name: &str| req.headers().get(name).ok().flatten();
    let (content_type, request_origin, cookie) =
        (header("Content-Type"), header("Origin"), header("Cookie"));
    let body = if method == "POST" {
        let declared = header("Content-Length").and_then(|length| length.parse::<usize>().ok());
        if declared.is_some_and(|length| length > MAX_BODY) {
            return write(too_large());
        }
        let body = req.bytes().await?;
        if body.len() > MAX_BODY {
            return write(too_large());
        }
        body
    } else {
        Vec::new()
    };

    let unavailable = |what: &str| {
        write(Answer {
            status: 503,
            headers: vec![("Content-Type", "application/json".into())],
            body: format!(r#"{{"error":"temporarily_unavailable","error_description":"{what}"}}"#)
                .into_bytes(),
        })
    };
    let Some(key) = crate::secret::secret(&env, "SERVICE_SECRET_KEY")
        .ok()
        .and_then(|seed| ClientKey::derive(&seed).ok())
    else {
        return unavailable("no service identity is configured");
    };
    let answer = AuthorizationServer {
        key: &key,
        origin: &origin,
        now: worker::Date::now().as_millis() / 1000,
    }
    .handle(
        &endpoint,
        &OAuthRequest {
            method: &method,
            query: url.query(),
            content_type: content_type.as_deref(),
            origin: request_origin.as_deref(),
            cookie: cookie.as_deref(),
            body: &body,
        },
    )
    .await;
    write(answer)
}

fn too_large() -> Answer {
    Answer {
        status: 413,
        headers: vec![("Content-Type", "application/json".into())],
        body: format!(
            r#"{{"error":"invalid_request","error_description":"the body is over {MAX_BODY} bytes"}}"#
        )
        .into_bytes(),
    }
}

fn write(answer: Answer) -> Result<Response> {
    let headers = Headers::new();
    for (name, value) in &answer.headers {
        headers.append(name, value)?;
    }
    Ok(Response::from_bytes(answer.body)?
        .with_status(answer.status)
        .with_headers(headers))
}
