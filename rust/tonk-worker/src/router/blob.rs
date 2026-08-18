//! `GET /api/repository/{repo}/branch/{branch}/blob/{entity}` —
//! serve content-addressed blob bytes from a branch's blob store.

use ::axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::{ArtifactSelector, Attribute, Entity, Value};
use dialog_effects::blob::BlobError;
use dialog_repository::{Blob, CommitError, RepositoryExt as _};
use futures_util::{StreamExt as _, stream};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use super::claim::RawClaim;
use super::evaluate::EvaluatePath;
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
        Some(Ok(artifact)) => artifact
            .value()
            .ok()
            .and_then(|value| String::try_from(value).ok())
            .unwrap_or_else(|| "application/octet-stream".to_string()),
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

/// JSON body of a successful `POST …/blob`.
#[derive(Debug, Serialize)]
pub struct BlobUploadResponse {
    /// The content-addressed `blob:<hash>` entity the bytes were stored under.
    pub entity: String,
    /// MIME type recorded for the blob (from the request `Content-Type`).
    #[serde(rename = "contentType")]
    pub content_type: String,
    /// File name recorded for the blob (header value, existing fact, or the entity string).
    pub name: String,
    /// Size of the stored bytes.
    pub size: usize,
}

/// Handler for `POST /api/repository/{repo}/branch/{branch}/blob`.
///
/// Buffers the request body, writes it into the branch's content-addressed
/// blob store, and asserts the blob's `xyz.tonk.blob/content-type` and
/// `xyz.tonk.blob/name` facts — the name fact is always asserted, defaulting
/// to an existing fact (on a headerless re-upload) or the entity string when
/// the `X-Tonk-Blob-Name` header is absent — so the read route (`serve`, above) and
/// `<tonk-display model=tonk:blob>` work immediately. Idempotent by content
/// address: re-uploading the same bytes returns the same entity and
/// re-asserts the same (cardinality-one) facts.
///
/// The blob bytes are written directly to the branch's blob store (as
/// `serve`, above, reads them back), which needs no reactor involvement —
/// it's raw content-addressed storage, not a claim. The metadata facts,
/// though, go through the *reactor's* transaction (`tonk.reactor.repository
/// (..).branch(..).transaction()`), matching `claim::assert_claim`'s commit
/// path, not a bare `Repository::branch(..).open()` handle's — that's the
/// path with subscription re-polling, so following up the commit with
/// `run_scheduled_polls` makes the new facts visible to subscribers
/// immediately, the same as every other write route in this file's crate.
#[wasm_compat]
pub async fn upload(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, TonkWorkerError> {
    if body.is_empty() {
        return Err(TonkWorkerError::Router("empty upload body".to_string()));
    }
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream")
        .to_string();
    let name = headers
        .get("x-tonk-blob-name")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let size = body.len();

    let tonk = state.write().await;

    // Ingest bytes into the content-addressed store. Direct branch access
    // (as `serve` reads from) — the blob store isn't part of the claim/fact
    // graph, so it doesn't go through the reactor.
    let repository = tonk
        .profile
        .repository(&path.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {e}", path.repo))
        })?;
    let branch_handle = repository
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {e}", path.branch))
        })?;

    let chunks = vec![Ok::<_, BlobError>(body.to_vec())];
    let entity = Blob::import(stream::iter(chunks))
        .write(branch_handle.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("write blob: {e}")))?;

    // Assert extrinsic metadata as ordinary facts on the blob entity,
    // mirroring `tonk blob add`. Committed through the reactor (like
    // `claim::assert_claim`), then drain the scheduled poll so subscribers
    // on this branch see the new facts.
    let ct_attr: Attribute = "xyz.tonk.blob/content-type"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;

    // Effective name: an explicit header wins; otherwise an already-
    // asserted name fact is preserved (a raw re-upload must not clobber
    // a good name with the hash default); otherwise the entity string.
    // The name fact must always land — the `tonk:blob` concept query
    // matches only rows with every field present, so a nameless blob
    // would never reach the seeded media view.
    let name_attr: Attribute = "xyz.tonk.blob/name"
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
    let name = match name {
        Some(n) => n,
        None => {
            let existing = branch_handle
                .claims()
                .select(
                    ArtifactSelector::new()
                        .the(name_attr.clone())
                        .of(entity.clone()),
                )
                .perform(&tonk.operator)
                .await
                .map_err(|e| TonkWorkerError::Internal(format!("name query: {e}")))?;
            tokio::pin!(existing);
            match existing.next().await {
                Some(Ok(artifact)) => artifact
                    .value()
                    .ok()
                    .and_then(|value| String::try_from(value).ok())
                    .unwrap_or_else(|| entity.to_string()),
                Some(Err(e)) => {
                    log!("blob: name query error: {:?}", e);
                    entity.to_string()
                }
                None => entity.to_string(),
            }
        }
    };

    let tx = tonk
        .reactor
        .repository(&path.repo)
        .branch(&path.branch)
        .transaction()
        .assert(RawClaim {
            the: ct_attr,
            of: entity.clone(),
            is: Value::String(content_type.clone()),
            unique: true,
        })
        .assert(RawClaim {
            the: name_attr,
            of: entity.clone(),
            is: Value::String(name.clone()),
            unique: true,
        });
    tx.commit()
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("assert metadata: {e}")))?;

    tonk.reactor.run_scheduled_polls(&tonk.operator).await;

    let payload = BlobUploadResponse {
        entity: entity.to_string(),
        content_type,
        name,
        size,
    };
    let json = serde_json::to_string(&payload)
        .map_err(|e| TonkWorkerError::Internal(format!("serialize: {e}")))?;
    let mut response = (StatusCode::OK, json).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
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
    async fn it_uploads_bytes_and_serves_them_back() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-upload").await;

        let payload = b"\x89PNG\r\n\x1a\nupload".to_vec();

        // Upload via the new POST route.
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "image/png")
                    .header("x-tonk-blob-name", "shot.png")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        let body = axum::body::to_bytes(up.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entity = json["entity"].as_str().unwrap().to_string();
        assert!(
            entity.starts_with("blob:"),
            "entity is a blob ref: {entity}"
        );
        assert_eq!(json["contentType"], "image/png");
        assert_eq!(json["name"], "shot.png");
        assert_eq!(json["size"], payload.len());

        // The GET route serves the same bytes + Content-Type from the asserted fact.
        let got = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob/{entity}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(got.status(), StatusCode::OK);
        assert_eq!(got.headers().get("content-type").unwrap(), "image/png");
        let got_body = axum::body::to_bytes(got.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(got_body.as_ref(), payload.as_slice());

        // Idempotent: same bytes → same entity.
        let up2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "image/png")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body2 = axum::body::to_bytes(up2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(
            json2["entity"], entity,
            "content-addressed: re-upload yields same entity"
        );
        assert_eq!(
            json2["name"], "shot.png",
            "re-upload preserves the name fact"
        );

        // Empty body → 400.
        let empty = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "image/png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    }

    /// The blob route accepts bodies past axum's 2 MiB extractor default —
    /// real image files routinely exceed it — up to the route's own
    /// [`crate::router::BLOB_UPLOAD_LIMIT`] ceiling.
    #[dialog_common::test]
    async fn it_accepts_an_upload_larger_than_the_axum_default_limit() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-large").await;

        let payload = vec![0xabu8; 3 * 1024 * 1024];
        let up = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/octet-stream")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            up.status(),
            StatusCode::OK,
            "a 3 MiB upload lands (the 2 MiB axum default would 413 it)",
        );
        let body = axum::body::to_bytes(up.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["size"], payload.len());
    }

    #[dialog_common::test]
    async fn it_defaults_the_name_fact_to_the_entity_when_no_header_is_sent() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-name-default").await;

        // Upload with no X-Tonk-Blob-Name header.
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .body(Body::from(b"%PDF-1.4 nameless".to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        let body = axum::body::to_bytes(up.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entity = json["entity"].as_str().unwrap().to_string();
        assert_eq!(
            json["name"], entity,
            "no header: the name fact defaults to the content-addressed entity string",
        );
    }

    #[dialog_common::test]
    async fn it_preserves_an_existing_name_on_a_headerless_reupload() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-name-keep").await;
        let payload = b"%PDF-1.4 named".to_vec();

        // First upload names the blob.
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .header("x-tonk-blob-name", "report.pdf")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);

        // A headerless re-upload of the same bytes keeps the asserted name
        // rather than clobbering it with the hash default.
        let up2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/pdf")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body2 = axum::body::to_bytes(up2.into_body(), usize::MAX)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        assert_eq!(
            json2["name"], "report.pdf",
            "headerless re-upload preserves the existing name fact",
        );
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
