//! Conversion utilities between browser and Axum HTTP types.
//!
//! This module provides bidirectional conversion between browser `Request`/`Response`
//! types and Axum's HTTP types, enabling Axum to work in a browser service worker context.

use std::str::FromStr;

use axum::{
    body::Body,
    http::{
        HeaderName, HeaderValue, Method, Request as AxumRequest, Response as AxumResponse, Uri,
    },
};
use futures_util::TryStreamExt;
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsError, JsValue, UnwrapThrowExt};
use wasm_bindgen_futures::JsFuture;
use wasm_streams::ReadableStream;
use web_sys::{Blob, Request as BrowserRequest, Response as BrowserResponse, ResponseInit};

use tonk_common::ExclusiveStream;
use url::Url;

/// Same-origin scheme and authority captured from the browser request URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestOrigin(Url);

impl RequestOrigin {
    /// Capture only the URL origin, discarding path, query, and fragment.
    pub fn parse(request_url: &str) -> Result<Self, JsError> {
        let parsed = Url::parse(request_url)
            .map_err(|error| JsError::new(&format!("invalid browser request URL: {error}")))?;
        let origin = parsed.origin().ascii_serialization();
        let origin = Url::parse(&format!("{origin}/"))
            .map_err(|error| JsError::new(&format!("invalid browser request origin: {error}")))?;
        Ok(Self(origin))
    }

    /// Origin URL with a root path and no query or fragment.
    pub fn url(&self) -> &Url {
        &self.0
    }
}

/// Wrapper for converting browser requests to Axum requests.
pub struct RequestConversion(BrowserRequest);

impl From<BrowserRequest> for RequestConversion {
    fn from(value: BrowserRequest) -> Self {
        RequestConversion(value)
    }
}

impl From<RequestConversion> for BrowserRequest {
    fn from(value: RequestConversion) -> Self {
        value.0
    }
}

impl RequestConversion {
    /// Convert the browser request into an Axum request.
    ///
    /// Firefox does not expose the body of a service-worker-intercepted
    /// request as a `ReadableStream`: [`BrowserRequest::body`] is `undefined`
    /// (mapped to `None`) even when bytes are present. Reading the body via
    /// `blob()` works in every browser, so when `body()` yields nothing we
    /// fall back to streaming the blob. A genuinely bodyless request (e.g.
    /// `GET`) yields an empty blob, hence an empty stream, matching the
    /// previous `Body::empty()` behaviour.
    pub async fn into_axum_request(self) -> Result<AxumRequest<Body>, JsError> {
        let request: BrowserRequest = self.into();
        let method = Method::from_str(&request.method())?;
        let request_url = request.url();
        let origin = RequestOrigin::parse(&request_url)?;
        let uri = Uri::try_from(&request_url)?;

        let mut request_builder = AxumRequest::builder()
            .method(method)
            .uri(uri)
            .extension(origin);

        for header_entry in request
            .headers()
            .entries()
            .into_iter()
            .map(|entry| entry.expect_throw("Could not read request header"))
        {
            let header_entry = header_entry.dyn_into::<Array>().unwrap_throw();
            let key = header_entry.get(0).as_string().unwrap_or_default();
            let value = header_entry.get(1).as_string().unwrap_or_default();

            let header_name = HeaderName::from_bytes(key.as_bytes())?;
            let header_value = HeaderValue::from_str(&value)?;

            request_builder = request_builder.header(header_name, header_value);
        }

        let stream = match request.body() {
            Some(stream) => stream,
            None => {
                let promise = request
                    .blob()
                    .map_err(|value| JsError::new(&format!("request.blob() failed: {value:?}")))?;
                let blob: Blob = JsFuture::from(promise)
                    .await
                    .map_err(|value| JsError::new(&format!("reading request blob: {value:?}")))?
                    .dyn_into()
                    .map_err(|value| {
                        JsError::new(&format!("request.blob() was not a Blob: {value:?}"))
                    })?;
                blob.stream()
            }
        };

        request_builder
            .body(Body::from_stream(Box::pin(ExclusiveStream::new(
                ReadableStream::from_raw(stream)
                    .into_stream()
                    .map_ok(|bytes| {
                        bytes
                            .dyn_into::<Uint8Array>()
                            .expect_throw("Could not convert readable stream bytes")
                            .to_vec()
                    })
                    .map_err(|error| format!("{:?}", error)),
            ))))
            .map_err(|error| JsError::new(&format!("Could not set request body: {error}")))
    }
}

/// Wrapper for converting Axum responses to browser responses.
pub struct ResponseConversion {
    method: Method,
    response: AxumResponse<Body>,
}

impl ResponseConversion {
    /// Pair a response with the method of the request that produced it.
    pub fn new(method: Method, response: AxumResponse<Body>) -> Self {
        Self { method, response }
    }
}

impl TryFrom<ResponseConversion> for BrowserResponse {
    type Error = JsError;

    fn try_from(value: ResponseConversion) -> Result<Self, Self::Error> {
        let ResponseConversion { method, response } = value;
        let status_code = response.status();
        let headers = JsValue::from(Object::new());

        for (key, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                Reflect::set(
                    &headers,
                    &JsValue::from(key.as_str()),
                    &JsValue::from(value),
                )
                .map_err(|_| JsError::new("Could not set fetch response header"))?;
            }
        }

        let response_options = ResponseInit::new();
        response_options.set_status(status_code.as_u16());
        response_options.set_headers(&headers);

        let has_null_body =
            method == Method::HEAD || matches!(status_code.as_u16(), 204 | 205 | 304);
        let body_stream = (!has_null_body).then(|| {
            ReadableStream::from_stream(
                response
                    .into_body()
                    .into_data_stream()
                    .map_ok(|value| JsValue::from(Uint8Array::from(value.as_ref())))
                    .map_err(|error| JsValue::from(format!("{error}"))),
            )
            .into_raw()
        });

        BrowserResponse::new_with_opt_readable_stream_and_init(
            body_stream.as_ref(),
            &response_options,
        )
        .map_err(|_| JsError::new("Could not construct fetch response"))
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Headers, RequestInit};

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_converts_get_request_without_body() {
        let init = RequestInit::new();
        init.set_method("GET");

        let request = BrowserRequest::new_with_str_and_init("https://example.com/api/test", &init)
            .expect("Failed to create request");

        let axum_request = RequestConversion::from(request)
            .into_axum_request()
            .await
            .expect("Failed to convert request");

        assert_eq!(axum_request.method(), Method::GET);
        assert_eq!(axum_request.uri().path(), "/api/test");
        let origin = axum_request.extensions().get::<RequestOrigin>().unwrap();
        assert_eq!(origin.url().as_str(), "https://example.com/");
        assert!(origin.url().query().is_none());
        assert!(origin.url().fragment().is_none());
    }

    #[dialog_common::test]
    async fn it_converts_post_request_with_headers() {
        let init = RequestInit::new();
        init.set_method("POST");

        let headers = Headers::new().expect("Failed to create headers");
        headers
            .append("content-type", "application/json")
            .expect("Failed to append header");
        headers
            .append("x-custom-header", "test-value")
            .expect("Failed to append header");
        init.set_headers(&headers);

        let request =
            BrowserRequest::new_with_str_and_init("https://example.com/api/authorize", &init)
                .expect("Failed to create request");

        let axum_request = RequestConversion::from(request)
            .into_axum_request()
            .await
            .expect("Failed to convert request");

        assert_eq!(axum_request.method(), Method::POST);
        assert_eq!(axum_request.uri().path(), "/api/authorize");

        let headers = axum_request.headers();
        assert_eq!(
            headers.get("content-type").map(|v| v.to_str().unwrap()),
            Some("application/json")
        );
        assert_eq!(
            headers.get("x-custom-header").map(|v| v.to_str().unwrap()),
            Some("test-value")
        );
    }

    #[dialog_common::test]
    async fn it_converts_requests_with_different_http_methods() {
        for method in &["GET", "POST", "PUT", "DELETE", "PATCH"] {
            let init = RequestInit::new();
            init.set_method(method);

            let request = BrowserRequest::new_with_str_and_init("https://example.com/api", &init)
                .expect("Failed to create request");

            let axum_request = RequestConversion::from(request)
                .into_axum_request()
                .await
                .expect("Failed to convert request");

            assert_eq!(axum_request.method().as_str(), *method);
        }
    }

    #[dialog_common::test]
    async fn it_converts_response_with_status_code() {
        let response = AxumResponse::builder()
            .status(200)
            .body(Body::empty())
            .expect("Failed to build response");

        let browser_response: BrowserResponse = ResponseConversion::new(Method::GET, response)
            .try_into()
            .expect("Failed to convert response");

        assert_eq!(browser_response.status(), 200);
    }

    #[dialog_common::test]
    async fn it_converts_response_with_headers() {
        let response = AxumResponse::builder()
            .status(201)
            .header("content-type", "application/json")
            .header("x-custom-header", "response-value")
            .body(Body::empty())
            .expect("Failed to build response");

        let browser_response: BrowserResponse = ResponseConversion::new(Method::GET, response)
            .try_into()
            .expect("Failed to convert response");

        assert_eq!(browser_response.status(), 201);

        let headers = browser_response.headers();
        assert_eq!(
            headers.get("content-type").ok().flatten().as_deref(),
            Some("application/json")
        );
        assert_eq!(
            headers.get("x-custom-header").ok().flatten().as_deref(),
            Some("response-value")
        );
    }

    #[dialog_common::test]
    async fn it_converts_responses_with_different_status_codes() {
        for status in &[200, 201, 400, 404, 500] {
            let response = AxumResponse::builder()
                .status(*status)
                .body(Body::empty())
                .expect("Failed to build response");

            let browser_response: BrowserResponse = ResponseConversion::new(Method::GET, response)
                .try_into()
                .expect("Failed to convert response");

            assert_eq!(browser_response.status(), *status);
        }
    }

    #[dialog_common::test]
    async fn it_omits_bodies_for_null_body_statuses_and_head() {
        for (method, status) in [
            (Method::GET, 204),
            (Method::GET, 205),
            (Method::GET, 304),
            (Method::HEAD, 200),
        ] {
            let response = AxumResponse::builder()
                .status(status)
                .header("x-test", "retained")
                .body(Body::from("accidental body"))
                .unwrap();
            let browser: BrowserResponse = ResponseConversion::new(method, response)
                .try_into()
                .expect("null-body response converts");

            assert!(
                browser.body().is_none(),
                "status {status} must have no body"
            );
            assert_eq!(
                browser.headers().get("x-test").unwrap().as_deref(),
                Some("retained")
            );
        }
    }

    #[dialog_common::test]
    async fn it_keeps_empty_and_streamed_success_bodies() {
        for body in [Body::empty(), Body::from(r#"{"ok":true}"#)] {
            let response = AxumResponse::builder()
                .status(200)
                .header("content-type", "application/json")
                .body(body)
                .unwrap();
            let browser: BrowserResponse = ResponseConversion::new(Method::GET, response)
                .try_into()
                .expect("success response converts");

            assert!(browser.body().is_some());
            assert_eq!(
                browser.headers().get("content-type").unwrap().as_deref(),
                Some("application/json")
            );
        }
    }

    #[dialog_common::test]
    async fn it_reports_invalid_browser_response_construction() {
        let response = AxumResponse::builder()
            .status(101)
            .body(Body::empty())
            .unwrap();

        let result = BrowserResponse::try_from(ResponseConversion::new(Method::GET, response));
        assert!(result.is_err());
    }
}
