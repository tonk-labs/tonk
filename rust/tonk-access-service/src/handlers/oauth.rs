//! Worker-side glue for the OAuth authorization server.
//!
//! The protocol lives in [`crate::oauth`]; this binds it to D1, the
//! service seed and the runtime's `fetch`, and writes its answers out.

use crate::oauth::client::{ClientKey, Documents, MAX_DOCUMENT};
use crate::oauth::{Answer, AuthorizationServer, Endpoint, MAX_BODY, Request as OAuthRequest};
use crate::store::d1::D1Store;
use worker::{
    Env, Fetch, Headers, Method, Request, RequestInit, RequestRedirect, Response, Result,
};

/// Fetches client metadata documents with the runtime's `fetch`.
struct Fetched;

#[async_trait::async_trait(?Send)]
impl Documents for Fetched {
    async fn fetch(&self, url: &url::Url) -> std::result::Result<Vec<u8>, String> {
        let headers = Headers::new();
        headers
            .set("Accept", "application/json")
            .map_err(|e| e.to_string())?;
        let mut init = RequestInit::new();
        init.with_method(Method::Get)
            .with_headers(headers)
            .with_redirect(RequestRedirect::Manual);
        let request = Request::new_with_init(url.as_str(), &init).map_err(|e| e.to_string())?;
        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status_code() != 200 {
            return Err(format!("answered {}", response.status_code()));
        }
        let declared = response
            .headers()
            .get("Content-Length")
            .ok()
            .flatten()
            .and_then(|length| length.parse::<usize>().ok());
        if declared.is_some_and(|length| length > MAX_DOCUMENT) {
            return Err(format!("the document is over {MAX_DOCUMENT} bytes"));
        }
        response.bytes().await.map_err(|e| e.to_string())
    }
}

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
    let Ok(database) = env.d1("CONTROL") else {
        return unavailable("no CONTROL database is bound");
    };
    let Some(key) = crate::secret::secret(&env, "SERVICE_SECRET_KEY")
        .ok()
        .and_then(|seed| ClientKey::derive(&seed).ok())
    else {
        return unavailable("no service identity is configured");
    };
    let store = D1Store::new(database);
    let answer = AuthorizationServer {
        store: &store,
        documents: &Fetched,
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
