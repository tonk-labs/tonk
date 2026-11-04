//! `GET /api/repository/{repo}/branch/{branch}/http/{*path}`: what the
//! branch's routes answer a request for `path` with.
//!
//! A `route/http!` declares a path pattern, headers and a body, and a
//! `route/asset!` a path pattern and an asset of the space. A site's worker
//! hands this route every request its own handlers do not claim, and answers
//! with what comes back: the body, under the declared headers, or the
//! asset's bytes, under its media type, when the most specific route for the
//! path is one of those, and 404 when the path matches a route a page shows
//! or no route at all.
//!
//! The headers are the route's `header` dictionary, read by the same
//! query a page would run. A route that declares no `content-type` is
//! served as plain text, never sniffed.

use ::axum::extract::{Path, State};
use ::axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use ipld_core::ipld::Ipld;
use serde::Deserialize;
use serde_json::json;
use tonk_macros::handler;

use super::blob;
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
    /// The requested path, without its leading slash: empty for `/`.
    #[serde(default)]
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
#[handler]
pub async fn respond(
    State(state): State<AppState>,
    Path(path): Path<HttpRoutePath>,
    method: Method,
    headers: HeaderMap,
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
    let matched = match match_route(&tonk, &session, &rest).await {
        Some(Matched::Http(matched)) => matched,
        Some(Matched::Asset(matched)) => {
            drop(tonk);
            return blob::answer(
                &state,
                &path.repo,
                &path.branch,
                matched.asset,
                method,
                headers,
            )
            .await;
        }
        Some(Matched::View(_)) | None => return Ok(StatusCode::NOT_FOUND.into_response()),
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
