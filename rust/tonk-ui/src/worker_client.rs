//! Same-origin request construction for the browser worker.
//!
//! Callers name only an HTTP method and a worker-relative `/api` path. Worker
//! readiness, the page origin, and browser request context stay behind this
//! module's small interface.

use reqwest::{Method, RequestBuilder, Response, StatusCode, header::HeaderMap};

use crate::error::TonkUiError;

/// Construct a request to this page's local worker.
pub(crate) async fn request(
    method: Method,
    path: impl AsRef<str>,
) -> Result<WorkerRequest, TonkUiError> {
    tonk_host::ready::wait().await;
    request_at(
        &crate::api::origin(),
        method,
        path.as_ref(),
        browser_context_headers(),
    )
}

fn request_at(
    origin: &str,
    method: Method,
    path: &str,
    headers: Vec<(&'static str, String)>,
) -> Result<WorkerRequest, TonkUiError> {
    let is_worker_path =
        |path: &str| path == "/api" || path.starts_with("/api/") || path.starts_with("/api?");
    if !is_worker_path(path) {
        return Err(TonkUiError::ApiError(format!(
            "worker request path must stay under /api: {path}"
        )));
    }
    let url = url::Url::parse(&format!("{}{}", origin.trim_end_matches('/'), path))
        .map_err(|error| TonkUiError::ApiError(format!("invalid worker request URL: {error}")))?;
    if !is_worker_path(url.path()) || url.fragment().is_some() {
        return Err(TonkUiError::ApiError(format!(
            "worker request path escapes /api after URL normalization: {path}"
        )));
    }
    let mut request = reqwest::Client::new().request(method, url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    Ok(WorkerRequest(request))
}

/// A worker request whose response retains the ordinary reqwest contract while
/// the adapter observes transport-level worker signals.
pub(crate) struct WorkerRequest(RequestBuilder);

impl WorkerRequest {
    pub(crate) fn header(self, name: &'static str, value: &str) -> Self {
        Self(self.0.header(name, value))
    }

    pub(crate) fn json<T: serde::Serialize + ?Sized>(self, value: &T) -> Self {
        Self(self.0.json(value))
    }

    pub(crate) fn body(self, body: String) -> Self {
        Self(self.0.body(body))
    }

    pub(crate) async fn send(self) -> Result<Response, reqwest::Error> {
        let response = self.0.send().await?;
        notify_on_stale_build(response.status(), response.headers(), announce_update);
        Ok(response)
    }

    #[cfg(test)]
    fn build(self) -> Result<reqwest::Request, reqwest::Error> {
        self.0.build()
    }
}

fn notify_on_stale_build(status: StatusCode, headers: &HeaderMap, notify: impl FnOnce()) {
    let marked_stale = status == StatusCode::CONFLICT
        && headers
            .get(tonk_worker_api::ERROR_KIND_HEADER)
            .and_then(|value| value.to_str().ok())
            == Some(tonk_worker_api::STALE_BUILD_ERROR_KIND);
    if marked_stale {
        notify();
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn announce_update() {
    tonk_host::announce_update();
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn announce_update() {}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn browser_context_headers() -> Vec<(&'static str, String)> {
    tonk_host::bridge::context_headers()
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn browser_context_headers() -> Vec<(&'static str, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use reqwest::StatusCode;

    use super::*;

    #[dialog_common::test]
    fn it_keeps_worker_requests_on_the_page_api_surface() {
        let request = request_at(
            "https://tonk.example/",
            Method::DELETE,
            "/api/account?reason=sign-out",
            Vec::new(),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(request.method(), Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://tonk.example/api/account?reason=sign-out"
        );
        assert!(
            request_at(
                "https://tonk.example",
                Method::POST,
                "/accounts",
                Vec::new(),
            )
            .is_err(),
            "provider/service routes must not be mistaken for worker routes"
        );
        for path in ["/api/../accounts", "/api/%2e%2e/accounts", "/api#worker"] {
            assert!(
                request_at("https://tonk.example", Method::POST, path, Vec::new()).is_err(),
                "URL normalization must not let {path} escape or ambiguously name the worker surface"
            );
        }
        assert!(
            request_at(
                "https://tonk.example",
                Method::GET,
                "/api?health=1",
                Vec::new(),
            )
            .is_ok(),
            "the worker root may still carry a query string"
        );
    }

    #[dialog_common::test]
    fn it_stamps_every_direct_ui_worker_mutation() {
        let context = || {
            vec![
                ("x-tonk-site", "site:tab".to_owned()),
                ("x-tonk-build", "0123456789abcdef".to_owned()),
            ]
        };
        let mutations = [
            (Method::POST, "/api/profile/branch/main/transact"),
            (Method::POST, "/api/profile/branch/main/evaluate"),
            (Method::POST, "/api/repository/space/branch/main/evaluate"),
            (Method::POST, "/api/repository/space/branch/main/sync"),
            (Method::POST, "/api/repository/space/branch/main/sync/pull"),
            (Method::POST, "/api/repository/space/branch/main/sync/push"),
            (Method::POST, "/api/profile/join"),
            (Method::POST, "/api/identity/root"),
            (Method::POST, "/api/account/attach"),
            (Method::POST, "/api/account/display-name"),
            (Method::POST, "/api/custody/provision"),
            (Method::POST, "/api/custody/queue"),
            (Method::POST, "/api/account/devices/register"),
            (Method::POST, "/api/account/devices/revoke"),
            (Method::POST, "/api/customer/activated"),
            (Method::POST, "/api/profiles/activate"),
            (Method::POST, "/api/profiles/add"),
            (Method::DELETE, "/api/account"),
            (Method::POST, "/api/account/delete"),
            (Method::POST, "/api/account/spaces/delete"),
        ];

        for (method, path) in mutations {
            let request = request_at("https://tonk.example", method, path, context())
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(
                request
                    .headers()
                    .get_all("x-tonk-build")
                    .iter()
                    .collect::<Vec<_>>(),
                vec!["0123456789abcdef"],
                "{path} must carry this document's one exact build stamp"
            );
        }

        let query = request_at(
            "https://tonk.example",
            Method::POST,
            "/api/repository/space/branch/main/query",
            context(),
        )
        .unwrap()
        .build()
        .unwrap();
        assert_eq!(
            query.headers().get("x-tonk-build").unwrap(),
            "0123456789abcdef",
            "reads are stamped too, though the worker lets them survive skew"
        );

        let unstamped = request_at(
            "https://tonk.example",
            Method::POST,
            "/api/account/attach",
            vec![("x-tonk-site", "site:sealed".to_owned())],
        )
        .unwrap()
        .build()
        .unwrap();
        assert!(
            unstamped.headers().get("x-tonk-build").is_none(),
            "a context with no build retains missing-header compatibility"
        );
    }

    #[dialog_common::test]
    fn it_announces_only_a_marked_stale_build_conflict() {
        let cases = [
            (StatusCode::CONFLICT, Some("stale-build"), true),
            (StatusCode::CONFLICT, None, false),
            (StatusCode::BAD_REQUEST, Some("stale-build"), false),
            (StatusCode::BAD_REQUEST, Some("invalid-build-header"), false),
        ];

        for (status, marker, expected) in cases {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Some(marker) = marker {
                headers.insert(tonk_worker_api::ERROR_KIND_HEADER, marker.parse().unwrap());
            }
            let announced = Cell::new(false);
            notify_on_stale_build(status, &headers, || announced.set(true));
            assert_eq!(
                announced.get(),
                expected,
                "only a marked stale-build 409 should raise the reload prompt"
            );
        }
    }
}
