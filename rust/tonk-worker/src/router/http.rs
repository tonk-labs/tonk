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
pub(crate) async fn get(endpoint: &Url) -> Result<HttpResponse, HttpError> {
    request_with_timeout("GET", endpoint, None, None, TIMEOUT).await
}

#[allow(dead_code)] // reserved for JSON-only service operations
pub(crate) async fn post_json(endpoint: &Url, body: &[u8]) -> Result<HttpResponse, HttpError> {
    post(endpoint, body, "application/json").await
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn post(endpoint: &Url, body: &[u8], media_type: &str) -> Result<HttpResponse, HttpError> {
    post_with_timeout(endpoint, body, media_type, TIMEOUT).await
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn post_with_timeout(
    endpoint: &Url,
    body: &[u8],
    media_type: &str,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    request_with_timeout("POST", endpoint, Some(body), Some(media_type), timeout).await
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn post(endpoint: &Url, body: &[u8], media_type: &str) -> Result<HttpResponse, HttpError> {
    post_with_timeout(endpoint, body, media_type, TIMEOUT).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn post_with_timeout(
    endpoint: &Url,
    body: &[u8],
    media_type: &str,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    request_with_timeout("POST", endpoint, Some(body), Some(media_type), timeout).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn request_with_timeout(
    method: &str,
    endpoint: &Url,
    body: Option<&[u8]>,
    media_type: Option<&str>,
    timeout: Duration,
) -> Result<HttpResponse, HttpError> {
    use std::cell::Cell;
    use std::rc::Rc;

    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{AbortController, Request, RequestInit, Response};

    let controller = AbortController::new()
        .map_err(|_| HttpError::Transport("could not create abort controller".to_string()))?;
    let init = RequestInit::new();
    init.set_method(method);
    if let Some(body) = body {
        init.set_body(&js_sys::Uint8Array::from(body).into());
    }
    init.set_signal(Some(&controller.signal()));
    let request = Request::new_with_str_and_init(endpoint.as_str(), &init)
        .map_err(|_| HttpError::Transport("could not construct upstream request".to_string()))?;
    if let Some(media_type) = media_type {
        request
            .headers()
            .set("content-type", media_type)
            .map_err(|_| HttpError::Transport("could not set content type".to_string()))?;
    }

    // Call the worker-global timer and fetch functions dynamically. This keeps
    // the request path testable in a browser harness while using the same
    // `globalThis` functions in production's service-worker scope.
    let global = js_sys::global();
    let set_timeout: js_sys::Function = js_sys::Reflect::get(&global, &"setTimeout".into())
        .ok()
        .and_then(|value| value.dyn_into().ok())
        .ok_or_else(|| HttpError::Transport("worker timer is unavailable".to_string()))?;
    let clear_timeout: js_sys::Function = js_sys::Reflect::get(&global, &"clearTimeout".into())
        .ok()
        .and_then(|value| value.dyn_into().ok())
        .ok_or_else(|| HttpError::Transport("worker timer is unavailable".to_string()))?;
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &"fetch".into())
        .ok()
        .and_then(|value| value.dyn_into().ok())
        .ok_or_else(|| HttpError::Transport("worker fetch is unavailable".to_string()))?;
    let timed_out = Rc::new(Cell::new(false));
    let timeout_flag = Rc::clone(&timed_out);
    let timeout_controller = controller.clone();
    let abort = Closure::<dyn FnMut()>::new(move || {
        timeout_flag.set(true);
        timeout_controller.abort();
    });
    let timer = set_timeout
        .call2(
            &global,
            abort.as_ref().unchecked_ref(),
            &JsValue::from_f64(timeout.as_millis() as f64),
        )
        .ok()
        .and_then(|value| value.as_f64())
        .ok_or_else(|| HttpError::Transport("could not schedule request timeout".to_string()))?;
    let fetched = fetch
        .call1(&global, &request)
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Promise>().ok())
        .ok_or_else(|| HttpError::Transport("worker fetch did not return a promise".to_string()))
        .map(JsFuture::from);
    let fetched = match fetched {
        Ok(future) => future.await,
        Err(error) => Err(JsValue::from_str(&error.to_string())),
    };
    let _ = clear_timeout.call1(&global, &JsValue::from_f64(timer));
    drop(abort);
    let response: Response = fetched.and_then(|value| value.dyn_into()).map_err(|_| {
        if timed_out.get() {
            HttpError::Timeout
        } else {
            HttpError::Transport("upstream fetch failed".to_string())
        }
    })?;
    let status = response.status();
    let buffer = JsFuture::from(
        response
            .array_buffer()
            .map_err(|_| HttpError::Transport("could not read upstream body".to_string()))?,
    )
    .await
    .map_err(|_| HttpError::Transport("could not read upstream body".to_string()))?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
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

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen::JsValue;
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

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
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

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    fn install_fake_fetch(body: &str) -> crate::router::tests::GlobalPropertyGuard {
        let fetch = js_sys::Function::new_with_args("request", body);
        crate::router::tests::GlobalPropertyGuard::replace("fetch", fetch.as_ref())
    }

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    #[dialog_common::test]
    async fn it_preserves_request_and_response_contracts_in_wasm() {
        let global = js_sys::global();
        let _capture = crate::router::tests::GlobalPropertyGuard::replace(
            "__tonkHttpCapture",
            &JsValue::UNDEFINED,
        );
        {
            let _fetch = install_fake_fetch(
                r#"
                return request.arrayBuffer().then(buffer => {
                    globalThis.__tonkHttpCapture = {
                        method: request.method,
                        contentType: request.headers.get("content-type"),
                        body: Array.from(new Uint8Array(buffer))
                    };
                    if (request.url.includes("/reject")) {
                        return new Response(
                            JSON.stringify({ error: {
                                code: "CREDENTIAL_REVOKED",
                                message: "credential revoked"
                            } }),
                            { status: 403, headers: { "content-type": "application/json" } }
                        );
                    }
                    return new Response(new Uint8Array([7, 8, 9]), {
                        status: 201,
                        headers: { "X-Test-Echo": "v1" }
                    });
                });
                "#,
            );

            let endpoint = Url::parse("https://service.example.test/ok").unwrap();
            let response = post_cbor(&endpoint, &[0xd9, 0xd9, 0xf7]).await.unwrap();
            assert_eq!(response.status, 201);
            assert_eq!(response.body, [7, 8, 9]);

            let capture = js_sys::Reflect::get(&global, &"__tonkHttpCapture".into()).unwrap();
            assert_eq!(
                js_sys::Reflect::get(&capture, &"method".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some("POST")
            );
            assert_eq!(
                js_sys::Reflect::get(&capture, &"contentType".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some("application/cbor")
            );
            let captured_body =
                js_sys::Array::from(&js_sys::Reflect::get(&capture, &"body".into()).unwrap());
            assert_eq!(
                captured_body
                    .iter()
                    .map(|value| value.as_f64().unwrap() as u8)
                    .collect::<Vec<_>>(),
                [0xd9, 0xd9, 0xf7]
            );

            let endpoint = Url::parse("https://service.example.test/reject").unwrap();
            let error = post_json(&endpoint, br#"{"request":true}"#)
                .await
                .unwrap_err();
            let HttpError::Upstream(error) = error else {
                panic!("expected structured upstream failure");
            };
            assert_eq!(error.status, 403);
            assert_eq!(error.code.as_deref(), Some("CREDENTIAL_REVOKED"));
            assert_eq!(error.message, "credential revoked");
        }

        let _fetch = install_fake_fetch(
            r#"
            return new Promise((_resolve, reject) => {
                request.signal.addEventListener("abort", () => {
                    reject(new DOMException("aborted", "AbortError"));
                });
            });
            "#,
        );
        let endpoint = Url::parse("https://service.example.test/slow").unwrap();
        let error = post_with_timeout(
            &endpoint,
            &[1],
            "application/cbor",
            Duration::from_millis(1),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, HttpError::Timeout));
    }
}
