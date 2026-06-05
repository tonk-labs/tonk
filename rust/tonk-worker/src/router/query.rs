//! `POST /api/repository/{repo}/branch/{branch}/query` —
//! one-shot query or live subscription, depending on the
//! `Accept` header.
//!
//! Body: a serialized [`crate::reactor::Query`] (the on-the-wire
//! shape of a `dialog_query::ConceptQuery`).
//!
//! - **Default** — returns `Vec<Conclusion>` as JSON.
//! - **`Accept: text/event-stream`** — opens an SSE subscription;
//!   the body emits `data: <Vec<Conclusion>>\n\n` events.
//!   First event is the current snapshot; subsequent events fire
//!   whenever a commit/pull/sync changes the result.

use ::axum::Json;
use ::axum::body::{Body, Bytes};
use ::axum::extract::{Path, Request, State};
use ::axum::http::{HeaderMap, StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use axum_wasm_macros::wasm_compat;
use dialog_query::Output as _;
use http_body_util::BodyExt as _;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::reactor::{Conclusion, Query, ReactorError};
use crate::{TonkWorkerError, router::AppState};

/// Path parameters for `/query`.
#[derive(Debug, Deserialize)]
pub struct QueryPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Path parameters for the profile `/query` (no `repo` segment —
/// the profile lives outside the named-repo namespace).
#[derive(Debug, Deserialize)]
pub struct ProfileQueryPath {
    /// The branch name.
    pub branch: String,
}

/// Handler. Reads body, decides one-shot vs subscription based on
/// `Accept`, dispatches to the reactor.
#[wasm_compat]
pub async fn query(
    State(state): State<AppState>,
    Path(path): Path<QueryPath>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    let tonk = state.read().await;
    let branch = tonk.reactor.repository(&path.repo).branch(&path.branch);
    query_on_branch(&tonk, branch, headers, request).await
}

/// `POST /api/profile/branch/{branch}/query`
///
/// Profile-side counterpart to [`query`]. The profile is its own
/// repository but lives outside the named-repo namespace, so the
/// route surface is parallel rather than nested. Same body / `Accept`
/// / response contract — only the branch reference differs. Lets a
/// `<tonk-display>` read the profile's meta branch (e.g. the Hub's
/// list of spaces) the same way it reads any repository branch.
#[wasm_compat]
pub async fn query_profile(
    State(state): State<AppState>,
    Path(path): Path<ProfileQueryPath>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    let tonk = state.read().await;
    let branch = tonk.reactor.profile_repository().branch(&path.branch);
    query_on_branch(&tonk, branch, headers, request).await
}

/// Shared body for [`query`] and [`query_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is the
/// only difference between the two routes.
async fn query_on_branch<'a>(
    tonk: &'a crate::worker::TonkState,
    branch: crate::reactor::BranchReference<'a>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, TonkWorkerError> {
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|e| TonkWorkerError::Router(format!("failed to read body: {e}")))?
        .to_bytes();
    let wire: Query = serde_json::from_slice(&bytes)
        .map_err(|e| TonkWorkerError::Router(format!("invalid ConceptQuery body: {e}")))?;
    let query = wire.into();

    let want_stream = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"));

    if want_stream {
        let subscriber = branch
            .subscribe(query)
            .perform(&tonk.operator)
            .await
            .map_err(reactor_to_error)?;

        let body_stream = UnboundedReceiverStream::new(subscriber.receiver).map(|bytes: Bytes| {
            let mut framed = Vec::with_capacity(bytes.len() + 8);
            framed.extend_from_slice(b"data: ");
            framed.extend_from_slice(&bytes);
            framed.extend_from_slice(b"\n\n");
            Ok::<_, std::io::Error>(Bytes::from(framed))
        });

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header("Connection", "keep-alive")
            .body(Body::from_stream(body_stream))
            .expect("response builder failed"))
    } else {
        let session = branch
            .acquire(&tonk.operator)
            .await
            .map_err(reactor_to_error)?;
        let terms = query.terms.clone();
        let conclusions = session
            .handle()
            .select(tonk_schema::concept::QueryPlan::from(query))
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(|e| reactor_to_error(ReactorError::QueryFailed(e)))?;
        let wire: Vec<Conclusion> = conclusions
            .iter()
            .map(|c| Conclusion::project(c, &terms))
            .collect();
        Ok(Json(wire).into_response())
    }
}

fn reactor_to_error(err: ReactorError) -> TonkWorkerError {
    match err {
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
            TonkWorkerError::NotFound(err.to_string())
        }
        ReactorError::QueryFailed(_)
        | ReactorError::Commit(_)
        | ReactorError::Induce(_)
        | ReactorError::Pull(_)
        | ReactorError::Push(_) => TonkWorkerError::Internal(err.to_string()),
    }
}
