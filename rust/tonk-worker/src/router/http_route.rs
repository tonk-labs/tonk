//! `GET /api/repository/{repo}/branch/{branch}/http/{*path}`: what the
//! branch's routes answer a request for `path` with.
//!
//! A `route/http!` declares a path pattern, headers and a body. A site's
//! worker hands this route every request its own handlers do not claim,
//! and answers with what comes back: the body, under the declared headers,
//! when the most specific route for the path is one of those, and 404 when
//! the path matches a route a page shows or no route at all.
//!
//! The headers are the route's `header` dictionary, read by the same
//! query a page would run. A route that declares no `content-type` is
//! served as plain text, never sniffed.

use ::axum::extract::{Path, State};
use ::axum::http::{HeaderName, HeaderValue, StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use axum_wasm_macros::wasm_compat;
use ipld_core::ipld::Ipld;
use serde::Deserialize;
use serde_json::json;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::session::{Matched, match_route};
use crate::reactor::Query;
use crate::{TonkWorkerError, router::AppState};

/// Path parameters for the route.
#[derive(Debug, Deserialize)]
pub struct HttpRoutePath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
    /// The requested path, without its leading slash.
    pub path: String,
}

/// The query reading one route's headers: a row per header, each a map
/// of that header's name to its value under `header`.
fn header_query(route: &str) -> Result<Query, TonkWorkerError> {
    serde_json::from_value(json!({
        "predicate": {
            "with": {
                "header": {
                    "the": { "domain": "xyz.tonk.route.http.header", "keyed": "dictionary" },
                    "as": "Text",
                    "cardinality": "one"
                }
            }
        },
        "terms": {
            "this": route,
            "header": { "?": { "name": "header" } },
            "header/key": { "?": { "name": "header/key" } }
        }
    }))
    .map_err(|e| TonkWorkerError::Internal(format!("header query: {e}")))
}

/// Handler. Matches the path against the branch's routes and answers with
/// the matched route's content.
#[wasm_compat]
pub async fn respond(
    State(state): State<AppState>,
    Path(path): Path<HttpRoutePath>,
) -> Result<Response, TonkWorkerError> {
    let profile = super::names_profile(&state, &path.repo).await;
    let tonk = state.read().await;
    let branch = if profile {
        tonk.reactor.profile_repository().branch(&path.branch)
    } else {
        tonk.reactor.repository(&path.repo).branch(&path.branch)
    };
    let Ok(session) = branch.acquire(&tonk.operator).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let rest = format!("/{}", path.path);
    let Some(Matched::Http(matched)) = match_route(&tonk, &session, &rest).await else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let query = header_query(&matched.route.to_string())?
        .into_concept_query()
        .map_err(|_| TonkWorkerError::Internal("header query is not a concept query".into()))?;
    let rows = branch
        .query(query)
        .perform(&tonk.operator)
        .await
        .unwrap_or_default();

    let mut response = matched.body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    // A row per header, its `header` field a map of that one name to its
    // value.
    for row in rows {
        let Some(Ipld::Map(entries)) = row.fields.get("header") else {
            continue;
        };
        for (name, value) in entries {
            let Ipld::String(value) = value else {
                continue;
            };
            match (
                HeaderName::try_from(name.as_str()),
                HeaderValue::try_from(value.as_str()),
            ) {
                (Ok(name), Ok(value)) => {
                    headers.insert(name, value);
                }
                _ => tonk_common::log!("route {}: unusable header {name}", matched.route),
            }
        }
    }
    Ok(response)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use ::axum::Router;
    use ::axum::body::Body;
    use ::axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    const LIBRARY: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

    const ROUTES: &str = r#"
route/http!:
  path: /lib/hello.js
  header:
    content-type: "text/javascript"
    cache-control: "no-cache"
  body: |
    export const hello = "world";

route/http!:
  path: /plain
  body: just text

route!:
  path: /shown
  concept: probe:shown
"#;

    /// A space holding the library and the routes above, and its key.
    async fn space(label: &str) -> (Router, String) {
        let (app, state, _lsp) =
            crate::router::api_router_with_state(crate::router::tests::test_state().await);
        let key = crate::router::tests::put_repo(&app, label).await;
        let tonk = state.read().await;
        crate::router::repository::install_fresh_seed(
            &tonk,
            &key,
            "main",
            &format!("{LIBRARY}\n{ROUTES}"),
            &[],
        )
        .await
        .expect("the library installs");
        drop(tonk);
        (app, key)
    }

    async fn get(app: &Router, key: &str, path: &str) -> ::axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{key}/branch/main/http{path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn text(response: ::axum::response::Response) -> String {
        let bytes = ::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[dialog_common::test]
    async fn it_answers_a_path_with_the_routes_body_and_headers() {
        let (app, key) = space("http-route-body").await;

        let response = get(&app, &key, "/lib/hello.js").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/javascript");
        assert_eq!(response.headers()["cache-control"], "no-cache");
        assert_eq!(text(response).await, "export const hello = \"world\";\n");
    }

    #[dialog_common::test]
    async fn it_serves_a_route_without_a_content_type_as_plain_text() {
        let (app, key) = space("http-route-plain").await;

        let response = get(&app, &key, "/plain").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(text(response).await, "just text");
    }

    #[dialog_common::test]
    async fn it_answers_not_found_for_a_path_a_page_shows() {
        let (app, key) = space("http-route-view").await;

        assert_eq!(
            get(&app, &key, "/shown").await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[dialog_common::test]
    async fn it_answers_not_found_for_a_path_no_route_matches() {
        let (app, key) = space("http-route-none").await;

        assert_eq!(
            get(&app, &key, "/lib/missing.js").await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
