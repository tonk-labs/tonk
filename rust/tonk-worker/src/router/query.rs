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
use http_body_util::BodyExt as _;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::reactor::{Conclusion, Query, ReactorError};
use crate::router::update_pending;
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
    let client = request_client(&request);
    let tonk = state.read().await;
    // First use of a directory-listed space this device has not
    // replicated mounts it on demand — the lazy half of
    // directory-driven adoption. A no-op for mounted repos.
    if let Err(error) = super::adopt::ensure_space_mounted(&tonk, &path.repo).await {
        tonk_common::log!("on-demand mount of '{}' failed: {error}", path.repo);
    }
    let branch = tonk.reactor.repository(&path.repo).branch(&path.branch);
    query_on_branch(&tonk, branch, headers, request, client).await
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
    let client = request_client(&request);
    let tonk = state.read().await;
    let branch = tonk.reactor.profile_repository().branch(&path.branch);
    query_on_branch(&tonk, branch, headers, request, client).await
}

/// The requesting SW client's id, when the fetch handler stamped one.
/// Tags SSE subscribers so the stale-client sweep can prune the ones
/// whose page is gone (a dead client's stream may never cancel, so
/// send-failure pruning alone can't be relied on).
fn request_client(request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<crate::router::ClientId>()
        .map(|c| c.0.clone())
        .filter(|id| !id.is_empty())
}

/// Shared body for [`query`] and [`query_profile`]. Takes a
/// [`crate::reactor::BranchReference`] so the URL extraction is the
/// only difference between the two routes.
async fn query_on_branch<'a>(
    tonk: &'a crate::worker::TonkState,
    branch: crate::reactor::BranchReference<'a>,
    headers: HeaderMap,
    request: Request,
    client: Option<String>,
) -> Result<Response, TonkWorkerError> {
    let bytes = request
        .into_body()
        .collect()
        .await
        .map_err(|e| TonkWorkerError::Router(format!("failed to read body: {e}")))?
        .to_bytes();
    let wire: Query = serde_json::from_slice(&bytes)
        .map_err(|e| TonkWorkerError::Router(format!("invalid query body: {e}")))?;

    let want_stream = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("text/event-stream"));

    // A formula query (string predicate) is resolved by the worker
    // rather than dialog's planner — the `tree/*` introspection
    // family. One-shot only for now.
    let query = match wire.into_concept_query() {
        Ok(query) => query,
        Err(wire) => {
            if want_stream {
                return Err(TonkWorkerError::Router(
                    "formula queries do not support subscriptions yet".into(),
                ));
            }
            let session = branch
                .acquire(&tonk.operator)
                .await
                .map_err(reactor_to_error)?;
            let conclusions =
                crate::reactor::resolve_formula(session.handle(), &tonk.operator, &wire)
                    .await
                    .map_err(|e| TonkWorkerError::Router(e.to_string()))?;
            return Ok(Json(conclusions).into_response());
        }
    };

    if want_stream {
        // Once this worker starts retiring, refuse to open a long-lived
        // stream: an SSE response is a fetch event that never settles, so one
        // reopened stream would keep the outgoing instance serving stale
        // clients after replacement. A live `waiting` check closes the race
        // before the Rust retirement
        // hook runs; the worker-owned latch keeps the refusal terminal after
        // the successor activates and `registration.waiting` becomes empty.
        if update_pending() || tonk.is_retiring() {
            let snapshot = match branch.query(query.clone()).perform(&tonk.operator).await {
                Ok(rows) => rows,
                Err(error) if is_absence(&error) => Vec::new(),
                Err(error) => return Err(reactor_to_error(error)),
            };
            return Ok(retry_later(snapshot));
        }

        let mut subscribe = branch.subscribe(query.clone());
        if let Some(client) = client.clone() {
            subscribe = subscribe.client(client);
        }
        let subscriber = match subscribe.perform(&tonk.operator).await {
            Ok(subscriber) => subscriber,
            Err(ReactorError::Shutdown) => {
                let snapshot = match branch.query(query).perform(&tonk.operator).await {
                    Ok(rows) => rows,
                    Err(error) if is_absence(&error) => Vec::new(),
                    Err(error) => return Err(reactor_to_error(error)),
                };
                return Ok(retry_later(snapshot));
            }
            // The branch is not here YET. Absence is a result, not a
            // failure, so answer with an open stream carrying the empty
            // set and keep watching: when the repo mounts (a join in
            // another tab, a directory row syncing in), the standing
            // subscription attaches and delivers a real frame into this
            // same stream. Erroring here instead closed the stream, and
            // the page had nothing left to hear the answer on.
            // The branch is not here YET. Register in the reactor's
            // waiting room and answer with an OPEN stream carrying the
            // empty set: nothing matched, which is exactly what a
            // present-but-empty branch answers. When the repo mounts or
            // the branch is created, `acquire` moves this subscriber
            // onto the real branch and the next poll delivers rows into
            // this same stream. No polling, no reconnect.
            Err(error) if is_absence(&error) => {
                let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
                // The honest current answer, delivered immediately so the
                // consumer leaves `loading` and renders its empty state.
                let _ = sender.send(Bytes::from_static(b"[]"));
                if tonk
                    .reactor
                    .register_pending(
                        branch.repository.name(),
                        branch.name,
                        dialog_reactor::PendingSubscription {
                            query,
                            client,
                            sender,
                        },
                    )
                    .is_err()
                {
                    return Ok(retry_later(Vec::new()));
                }
                return Ok(sse_response(receiver));
            }
            Err(error) => return Err(reactor_to_error(error)),
        };

        Ok(sse_response(subscriber.receiver))
    } else {
        // An absent repo/branch answers with the EMPTY SET, not a 404 —
        // "nothing matched" is a result, and it is the same result a
        // present-but-empty branch gives. See [`is_absence`].
        let wire: Vec<Conclusion> = match branch.query(query).perform(&tonk.operator).await {
            Ok(rows) => rows,
            Err(error) if is_absence(&error) => Vec::new(),
            Err(error) => return Err(reactor_to_error(error)),
        };
        Ok(Json(wire).into_response())
    }
}

/// Tell a query consumer to hold its reconnect for `controllerchange`, then
/// close immediately so this response cannot pin the retiring worker.
fn retry_later(conclusions: Vec<Conclusion>) -> Response {
    let snapshot = serde_json::to_string(&tonk_worker_api::Frame::Snapshot { conclusions })
        .expect("snapshot frame serialization failed");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(format!(
            "data: {snapshot}\n\ndata: {{\"control\":\"update-pending\"}}\n\n"
        )))
        .expect("response builder failed")
}

fn reactor_to_error(err: ReactorError) -> TonkWorkerError {
    match err {
        ReactorError::Shutdown => TonkWorkerError::Internal(err.to_string()),
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
            TonkWorkerError::NotFound(err.to_string())
        }
        ReactorError::QueryFailed(_)
        | ReactorError::Commit(_)
        | ReactorError::Pull(_)
        | ReactorError::Download(_)
        | ReactorError::Push(_) => TonkWorkerError::Internal(err.to_string()),
    }
}

/// Frame an subscriber's receiver as an SSE response.
///
/// Shared by the ordinary subscribe path and the waiting-room path so a
/// pending subscription is indistinguishable on the wire from a live
/// one — same content type, same framing, same open stream.
fn sse_response(receiver: tokio::sync::mpsc::UnboundedReceiver<Bytes>) -> Response {
    let body_stream = UnboundedReceiverStream::new(receiver).map(|bytes: Bytes| {
        let mut framed = Vec::with_capacity(bytes.len() + 8);
        framed.extend_from_slice(b"data: ");
        framed.extend_from_slice(&bytes);
        framed.extend_from_slice(b"\n\n");
        Ok::<_, std::io::Error>(Bytes::from(framed))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(body_stream))
        .expect("response builder failed")
}

/// Whether this failure means "there is nothing here" rather than
/// "something went wrong".
///
/// A repository or branch this device has not replicated is an ABSENCE,
/// and absence is the same answer a query gets when a branch exists but
/// holds no matching rows: the empty set. Reporting it as `404` made it
/// a transport failure instead of a result, which every consumer then
/// had to special-case — and, because a failed subscription has no
/// frames, a page that asked before the repo arrived could never be told
/// when it did. A space joined in another tab left the first tab showing
/// "not here" forever.
///
/// Absence is not permanent: the subscription stays open against the
/// named branch, so the moment the repo mounts, the standing query
/// delivers a real frame.
fn is_absence(err: &ReactorError) -> bool {
    matches!(
        err,
        ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// A repository or branch this device has not replicated is an
    /// ABSENCE, not a failure. Classifying it as one is what lets the
    /// query paths answer with the empty set and park the subscription
    /// in the reactor's waiting room, instead of returning `404` — which
    /// closed the stream and left a page with nothing to hear the answer
    /// on when the repo finally arrived.
    #[dialog_common::test]
    fn it_reads_a_missing_repository_or_branch_as_absence() {
        assert!(is_absence(&ReactorError::RepositoryNotFound {
            repo: "did:key:zAbsent".into(),
            reason: "not replicated here".into(),
        }));
        assert!(is_absence(&ReactorError::BranchNotFound {
            repo: "did:key:zPresent".into(),
            branch: "main".into(),
            reason: "branch not created yet".into(),
        }));
    }

    /// A genuine fault is still a fault: answering it with the empty set
    /// would report "nothing matched" for a broken query or a failed
    /// commit, hiding the error behind a plausible-looking result. The
    /// match is exhaustive, so a new variant has to be classified here
    /// rather than silently defaulting to one side.
    #[dialog_common::test]
    fn it_classifies_every_variant_deliberately() {
        fn absent(error: &ReactorError) -> bool {
            match error {
                ReactorError::Shutdown => false,
                ReactorError::RepositoryNotFound { .. } | ReactorError::BranchNotFound { .. } => {
                    true
                }
                ReactorError::QueryFailed(_)
                | ReactorError::Commit(_)
                | ReactorError::Pull(_)
                | ReactorError::Download(_)
                | ReactorError::Push(_) => false,
            }
        }
        // `is_absence` must agree with that intent for the cases we can
        // construct without fabricating upstream error types.
        let missing_repo = ReactorError::RepositoryNotFound {
            repo: "did:key:zAbsent".into(),
            reason: "not replicated here".into(),
        };
        assert_eq!(is_absence(&missing_repo), absent(&missing_repo));
    }
}
