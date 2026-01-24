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
use wasm_streams::ReadableStream;
use web_sys::{Request as BrowserRequest, Response as BrowserResponse, ResponseInit};

use tonk_common::ExclusiveStream;

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

impl TryFrom<RequestConversion> for AxumRequest<Body> {
    type Error = JsError;

    fn try_from(request: RequestConversion) -> Result<Self, Self::Error> {
        let request: BrowserRequest = request.into();
        let method = Method::from_str(&request.method())?;
        let uri = Uri::try_from(&request.url())?;

        let mut request_builder = AxumRequest::builder().method(method).uri(uri);

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

        let request = match request.body() {
            Some(stream) => {
                request_builder.body(Body::from_stream(Box::pin(ExclusiveStream::new(
                    wasm_streams::ReadableStream::from_raw(stream)
                        .into_stream()
                        .map_ok(|bytes| {
                            bytes
                                .dyn_into::<Uint8Array>()
                                .expect_throw("Could not convert readable stream bytes")
                                .to_vec()
                        })
                        .map_err(|error| format!("{:?}", error)),
                ))))
            }
            None => request_builder.body(Body::empty()),
        }
        .expect_throw("Could not set request body");

        Ok(request)
    }
}

/// Wrapper for converting Axum responses to browser responses.
pub struct ResponseConversion(AxumResponse<Body>);

impl From<AxumResponse<Body>> for ResponseConversion {
    fn from(value: AxumResponse<Body>) -> Self {
        ResponseConversion(value)
    }
}

impl From<ResponseConversion> for AxumResponse<Body> {
    fn from(value: ResponseConversion) -> Self {
        value.0
    }
}

impl TryFrom<ResponseConversion> for BrowserResponse {
    type Error = JsError;

    fn try_from(value: ResponseConversion) -> Result<Self, Self::Error> {
        let response: AxumResponse<Body> = value.into();
        let status_code = response.status();
        let headers = JsValue::from(Object::new());

        for (key, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                Reflect::set(
                    &headers,
                    &JsValue::from(key.as_str()),
                    &JsValue::from(value),
                )
                .unwrap_throw();
            }
        }

        let body_stream = ReadableStream::from_stream(
            response
                .into_body()
                .into_data_stream()
                .map_ok(|value| JsValue::from(Uint8Array::from(value.as_ref())))
                .map_err(|error| JsValue::from(format!("{error}"))),
        )
        .into_raw();

        let response_options = ResponseInit::new();
        response_options.set_status(status_code.as_u16());
        response_options.set_headers(&headers);

        let response = BrowserResponse::new_with_opt_readable_stream_and_init(
            Some(&body_stream),
            &response_options,
        )
        .expect_throw("Could not construct fetch response");

        Ok(response)
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{Headers, RequestInit};

    wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[dialog_common::test]
    async fn it_converts_get_request_without_body() {
        let init = RequestInit::new();
        init.set_method("GET");

        let request = BrowserRequest::new_with_str_and_init("https://example.com/api/test", &init)
            .expect("Failed to create request");

        let axum_request: AxumRequest<Body> = RequestConversion::from(request)
            .try_into()
            .expect("Failed to convert request");

        assert_eq!(axum_request.method(), Method::GET);
        assert_eq!(axum_request.uri().path(), "/api/test");
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

        let axum_request: AxumRequest<Body> = RequestConversion::from(request)
            .try_into()
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

            let axum_request: AxumRequest<Body> = RequestConversion::from(request)
                .try_into()
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

        let browser_response: BrowserResponse = ResponseConversion::from(response)
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

        let browser_response: BrowserResponse = ResponseConversion::from(response)
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

            let browser_response: BrowserResponse = ResponseConversion::from(response)
                .try_into()
                .expect("Failed to convert response");

            assert_eq!(browser_response.status(), *status);
        }
    }
}
