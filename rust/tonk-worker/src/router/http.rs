//! Typed outbound HTTP operations used by worker service proxies.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ERROR_BODY: usize = 8 * 1024;

/// Successful upstream response.
#[derive(Debug)]
pub(crate) struct HttpResponse {
    #[allow(dead_code)]
    pub status: u16,
    pub body: Vec<u8>,
}

/// Structured non-success response from an upstream service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpstreamFailure {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Error)]
pub(crate) enum HttpError {
    #[error("upstream request timed out")]
    Timeout,
    #[error("upstream transport failed: {0}")]
    Transport(String),
    #[error("upstream request failed: {0:?}")]
    Upstream(UpstreamFailure),
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: Option<String>,
    message: Option<String>,
}

fn failure(status: u16, body: &[u8]) -> UpstreamFailure {
    let bounded = &body[..body.len().min(MAX_ERROR_BODY)];
    match serde_json::from_slice::<ErrorEnvelope>(bounded) {
        Ok(envelope) => UpstreamFailure {
            status,
            code: envelope.error.code.filter(|code| !code.is_empty()),
            message: envelope
                .error
                .message
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| "upstream service rejected the request".to_string()),
        },
        // Not the structured envelope: surface what the service actually
        // said (a plain-text rejection, a proxy page) rather than a
        // generic phrase that hides the cause.
        Err(_) => {
            let text = String::from_utf8_lossy(bounded);
            let text = text.trim();
            UpstreamFailure {
                status,
                code: None,
                message: if text.is_empty() {
                    "upstream service rejected the request".to_string()
                } else {
                    let mut snippet: String = text.chars().take(512).collect();
                    if snippet.len() < text.len() {
                        snippet.push('…');
                    }
                    snippet
                },
            }
        }
    }
}

pub(crate) async fn post_cbor(endpoint: &Url, body: &[u8]) -> Result<HttpResponse, HttpError> {
    post(endpoint, body, "application/cbor").await
}

/// GET a JSON (or other) resource from an upstream service, with the
/// same timeout and error-envelope handling as the POST path.
#[cfg(feature = "persona")]
pub(crate) async fn get(endpoint: &Url) -> Result<HttpResponse, HttpError> {
    request_with_timeout("GET", endpoint, None, None, TIMEOUT).await
}

#[allow(dead_code)] // reserved for JSON-only service operations
pub(crate) async fn post_json(endpoint: &Url, body: &[u8]) -> Result<HttpResponse, HttpError> {
    post(endpoint, body, "application/json").await
}

async fn post(endpoint: &Url, body: &[u8], media_type: &str) -> Result<HttpResponse, HttpError> {
    post_with_timeout(endpoint, body, media_type, TIMEOUT).await
}

async fn post_with_timeout(
    endpoint: &Url,
    body: &[u8],
    media_type: &str,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    request_with_timeout("POST", endpoint, Some(body), Some(media_type), timeout).await
}

/// A kernel on WASI has no HTTP client: its embedder delivers the request,
/// such as an invocation the kernel signed, and hands back the answer, or
/// none within `timeout`.
#[cfg(target_arch = "wasm32")]
async fn request_with_timeout(
    method: &str,
    endpoint: &Url,
    body: Option<&[u8]>,
    media_type: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    use futures_util::future::{Either, select};

    let delivered =
        tonk_kernel_world::application::fetch(method, endpoint.as_str(), media_type, body);
    let waited = crate::r#async::sleep(timeout);
    futures_util::pin_mut!(delivered, waited);
    let answer = match select(delivered, waited).await {
        Either::Left((answer, _)) => {
            answer.map_err(|error| HttpError::Transport(format!("{error:?}")))?
        }
        Either::Right(_) => return Err(HttpError::Timeout),
    };
    if !(200..300).contains(&answer.status) {
        return Err(HttpError::Upstream(failure(answer.status, &answer.body)));
    }
    Ok(HttpResponse {
        status: answer.status,
        body: answer.body,
    })
}

#[cfg(not(target_arch = "wasm32"))]
async fn request_with_timeout(
    method: &str,
    endpoint: &Url,
    body: Option<&[u8]>,
    media_type: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    let client = reqwest::Client::new();
    let mut request = match method {
        "POST" => client.post(endpoint.clone()),
        _ => client.get(endpoint.clone()),
    };
    if let Some(media_type) = media_type {
        request = request.header(reqwest::header::CONTENT_TYPE, media_type);
    }
    if let Some(body) = body {
        request = request.body(body.to_vec());
    }
    let response = request.timeout(timeout).send().await.map_err(|error| {
        if error.is_timeout() {
            HttpError::Timeout
        } else {
            HttpError::Transport(error.to_string())
        }
    })?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| HttpError::Transport(error.to_string()))?
        .to_vec();
    if !(200..300).contains(&status) {
        return Err(HttpError::Upstream(failure(status, &bytes)));
    }
    Ok(HttpResponse {
        status,
        body: bytes,
    })
}

impl From<HttpError> for crate::TonkWorkerError {
    fn from(error: HttpError) -> Self {
        match error {
            HttpError::Upstream(error) => crate::TonkWorkerError::Upstream {
                status: error.status,
                code: error.code,
                message: error.message,
            },
            HttpError::Timeout => crate::TonkWorkerError::Upstream {
                status: 504,
                code: Some("UPSTREAM_TIMEOUT".to_string()),
                message: "upstream service timed out".to_string(),
            },
            HttpError::Transport(detail) => crate::TonkWorkerError::Upstream {
                status: 503,
                code: Some("UPSTREAM_UNAVAILABLE".to_string()),
                message: format!("upstream service is unavailable: {detail}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_preserves_structured_errors_and_bounds_untrusted_bodies() {
        let parsed = failure(
            403,
            br#"{"error":{"code":"CREDENTIAL_REVOKED","message":"revoked"}}"#,
        );
        assert_eq!(parsed.status, 403);
        assert_eq!(parsed.code.as_deref(), Some("CREDENTIAL_REVOKED"));
        assert_eq!(parsed.message, "revoked");

        let plain = failure(
            401,
            b"Authorization failed: hosted space is deleting or deleted",
        );
        assert_eq!(plain.code, None);
        assert_eq!(
            plain.message,
            "Authorization failed: hosted space is deleting or deleted"
        );

        let malformed = failure(502, &vec![b'x'; MAX_ERROR_BODY + 1]);
        assert_eq!(malformed.status, 502);
        assert_eq!(malformed.code, None);
        assert_eq!(malformed.message.chars().count(), 513);
        assert!(malformed.message.ends_with('…'));

        let empty = failure(502, b"");
        assert_eq!(empty.message, "upstream service rejected the request");
    }

    fn server(response: &'static [u8]) -> (Url, std::sync::mpsc::Receiver<Vec<u8>>) {
        use std::io::{Read as _, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + length {
                    break;
                }
            }
            sent.send(request).unwrap();
            stream.write_all(response).unwrap();
        });
        (
            Url::parse(&format!("http://{address}/operation")).unwrap(),
            received,
        )
    }

    #[dialog_common::test]
    async fn it_posts_json_with_an_explicit_media_type() {
        let (endpoint, request) =
            server(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nx-tonk-account-spaces: v1\r\nconnection: close\r\n\r\n{}");
        let response = post_json(&endpoint, br#"{"ok":true}"#).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{}");
        let request = request.recv().unwrap();
        let request = String::from_utf8_lossy(&request);
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
    }

    #[dialog_common::test]
    async fn it_posts_cbor_and_preserves_the_upstream_status_and_code() {
        let response = b"HTTP/1.1 403 Forbidden\r\ncontent-length: 70\r\nconnection: close\r\n\r\n{\"error\":{\"code\":\"CREDENTIAL_REVOKED\",\"message\":\"credential revoked\"}}";
        let (endpoint, request) = server(response);
        let error = post_cbor(&endpoint, &[0xd9, 0xd9, 0xf7]).await.unwrap_err();
        let HttpError::Upstream(error) = error else {
            panic!("expected structured upstream failure")
        };
        assert_eq!(error.status, 403);
        assert_eq!(error.code.as_deref(), Some("CREDENTIAL_REVOKED"));
        assert_eq!(error.message, "credential revoked");

        let request = request.recv().unwrap();
        let headers = String::from_utf8_lossy(&request);
        assert!(headers.starts_with("POST /operation HTTP/1.1"));
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: application/cbor")
        );
        assert!(request.ends_with(&[0xd9, 0xd9, 0xf7]));
    }
}
