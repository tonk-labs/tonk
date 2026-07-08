//! `GET /api/repository/{repo}/branch/{branch}/blob/{entity}` —
//! serve content-addressed blob bytes from a branch's blob store.

use ::axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Entity};
use dialog_effects::blob::BlobError;
use dialog_repository::{Blob, CommitError, RepositoryExt as _};
use futures_util::StreamExt as _;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for the blob route.
#[derive(Debug, Deserialize)]
pub struct BlobPath {
    /// The repository the blob's branch lives in.
    pub repo: String,
    /// The branch whose blob store holds the bytes.
    pub branch: String,
    /// The `blob:<hash>` entity URI to serve.
    pub entity: String,
}

/// Serve a blob's bytes. The `Content-Type` is read from the blob's
/// `xyz.tonk.blob/content-type` fact (as asserted by `tonk blob add`),
/// defaulting to `application/octet-stream` when none is recorded.
///
/// The whole blob is buffered before responding — fine for images;
/// streaming is a later refinement.
#[wasm_compat]
pub async fn serve(
    State(state): State<AppState>,
    Path(params): Path<BlobPath>,
) -> Result<Response, TonkWorkerError> {
    let entity: Entity = params.entity.parse().map_err(|e| {
        TonkWorkerError::Router(format!("Invalid blob entity '{}': {}", params.entity, e))
    })?;
    if entity.blob_hash().is_none() {
        return Err(TonkWorkerError::Router(format!(
            "Not a blob reference: {}",
            params.entity
        )));
    }

    let tonk = state.read().await;
    let repo = tonk
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", params.repo, e))
        })?;
    let branch = repo
        .branch(params.branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", params.branch, e))
        })?;

    // Content type from the blob's metadata fact, if asserted.
    let ct_attr: Attribute = "xyz.tonk.blob/content-type"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {}", e)))?;
    let ct_stream = branch
        .claims()
        .select(ArtifactSelector::new().the(ct_attr).of(entity.clone()))
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("content-type query: {}", e)))?;
    tokio::pin!(ct_stream);
    let content_type = match ct_stream.next().await {
        Some(Ok(artifact)) => {
            String::try_from(artifact.is).unwrap_or_else(|_| "application/octet-stream".to_string())
        }
        Some(Err(e)) => {
            log!("blob: content-type query error: {:?}", e);
            "application/octet-stream".to_string()
        }
        None => "application/octet-stream".to_string(),
    };

    // Blob bytes from the branch's content-addressed store.
    let mut reader = match Blob::from(entity)
        .read(branch.blobs())
        .perform(&tonk.operator)
        .await
    {
        Ok(reader) => reader,
        Err(CommitError::Blob(BlobError::NotFound(_))) => {
            return Err(TonkWorkerError::NotFound(format!(
                "blob not available: {}",
                params.entity
            )));
        }
        Err(e) => return Err(TonkWorkerError::Internal(format!("read blob: {}", e))),
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = reader
        .next()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("read blob chunk: {}", e)))?
    {
        bytes.extend_from_slice(&chunk);
    }

    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    Ok(response)
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dialog_repository::{Blob, RepositoryExt as _};
    use futures_util::stream;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::api_router_from_state;
    use crate::router::tests::{put_repo, test_state};

    #[dialog_common::test]
    async fn it_serves_blob_bytes_with_the_asserted_content_type() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-serve").await;

        // Write a blob straight into the branch store. `Blob::import(...).write(...)`
        // returns the content-addressed `blob:<hash>` entity directly — the same
        // value `tonk blob add` returns.
        let payload = b"\x89PNG\r\n\x1a\nhello".to_vec();
        let entity = {
            let guard = app_state.read().await;
            let repository = guard
                .profile
                .repository(&repo)
                .load()
                .perform(&guard.operator)
                .await
                .unwrap();
            let branch = repository
                .branch("main")
                .open()
                .perform(&guard.operator)
                .await
                .unwrap();
            let chunks = vec![Ok::<_, dialog_effects::blob::BlobError>(payload.clone())];
            Blob::import(stream::iter(chunks))
                .write(branch.blobs())
                .perform(&guard.operator)
                .await
                .unwrap()
        };

        // Assert its content-type fact through the HTTP claim route.
        let assert = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/repository/{repo}/branch/main/claim/assert/{entity}/xyz.tonk.blob/content-type"
                    ))
                    .method("POST")
                    .header("content-type", "text/plain")
                    .body(Body::from("image/png"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(assert.status(), StatusCode::OK);

        // GET the bytes back.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob/{entity}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "image/png",
            "Content-Type comes from the xyz.tonk.blob/content-type fact",
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), payload.as_slice());
    }

    #[dialog_common::test]
    async fn it_rejects_a_non_blob_entity() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-reject").await;

        // A well-formed entity that is not a blob reference must be rejected
        // before it can be used to read arbitrary fact bytes.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob/id:alice"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
