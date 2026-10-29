//! A branch's content-addressed blobs, read and written as streams.
//!
//! `GET /api/repository/{repo}/branch/{branch}/blob/{entity}` answers a
//! blob's bytes, or the byte range a `Range` header asks for, as they are
//! read from the branch's blob store. `POST …/blob` writes the request's
//! body into the store as it arrives. Neither holds a whole blob in memory.

use ::axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use dialog_artifacts::{ArtifactSelector, Entity, Relation, Value};
use dialog_effects::blob::{BlobError, BlobReader, ByteRange};
use dialog_repository::{Blob, CommitError, RepositoryExt as _};
use futures_util::{StreamExt as _, stream};
use serde::{Deserialize, Serialize};
use tonk_common::{ExclusiveStream, log};
use tonk_macros::handler;

use super::AppState;
use super::claim::RawClaim;
use super::evaluate::EvaluatePath;
use crate::{TonkWorkerError, worker::TonkState};

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

/// The bytes a `Range` header asks for, of a blob of `size` bytes, as the
/// first and last offset (inclusive).
///
/// `Ok(None)` answers the whole blob: no header, or one this does not
/// serve (another unit, or several ranges, which a server may answer with
/// the whole representation). `Err` is a range that starts past the end.
fn byte_range(header: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header.and_then(|header| header.trim().strip_prefix("bytes=")) else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return Ok(None);
    };
    let (first, last) = (first.trim(), last.trim());
    let range = match (first.parse::<u64>(), last.parse::<u64>()) {
        // `bytes=a-b`: from `a` to `b`, clamped to the end.
        (Ok(first), Ok(last)) if first <= last => (first, last.min(size.saturating_sub(1))),
        // `bytes=a-`: from `a` to the end.
        (Ok(first), Err(_)) if last.is_empty() => (first, size.saturating_sub(1)),
        // `bytes=-n`: the last `n` bytes.
        (Err(_), Ok(suffix)) if first.is_empty() && suffix > 0 => {
            (size.saturating_sub(suffix), size.saturating_sub(1))
        }
        _ => return Ok(None),
    };
    if size == 0 || range.0 >= size {
        return Err(());
    }
    Ok(Some(range))
}

/// Serve a blob's bytes, or the range a `Range` header asks for, streamed
/// from the branch's blob store. The `Content-Type` is the asset's recorded
/// media type, or the legacy one older blobs carry, defaulting to
/// `application/octet-stream`.
#[handler]
pub async fn serve(
    State(state): State<AppState>,
    Path(params): Path<BlobPath>,
    method: Method,
    headers: HeaderMap,
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

    hydrate_media(&state, &params.repo, &params.branch, &entity).await?;

    let tonk = state.read().await;
    let repo = tonk
        .profile
        .space(&params.repo)
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

    // Media type from the asset's metadata, if asserted: the current
    // attribute, or the legacy one blobs were described with before.
    let content_type = match first_text(&branch, &tonk.operator, MEDIA_TYPE, &entity).await? {
        Some(media_type) => media_type,
        None => first_text(&branch, &tonk.operator, LEGACY_CONTENT_TYPE, &entity)
            .await?
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    };

    // The blob's size, which the branch's tree records: what a range is
    // taken from and what an answer says it holds.
    let size = Blob::from(entity.clone())
        .size(branch.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("blob size: {e}")))?
        .ok_or_else(|| {
            TonkWorkerError::NotFound(format!("blob not available: {}", params.entity))
        })?;
    let range = match byte_range(
        headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok()),
        size,
    ) {
        Ok(range) => range,
        Err(()) => {
            let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(response);
        }
    };
    let (first, last) = range.unwrap_or((0, size.saturating_sub(1)));
    let length = if size == 0 { 0 } else { last - first + 1 };

    // A `HEAD` is answered from what is recorded, without reading a byte.
    let body = if method == Method::HEAD || length == 0 {
        Body::empty()
    } else {
        let mut read = Blob::from(entity);
        if range.is_some() {
            read = read.slice(ByteRange {
                offset: first,
                length: Some(length),
            });
        }
        let reader = match read.read(branch.blobs()).perform(&tonk.operator).await {
            Ok(reader) => reader,
            Err(CommitError::Blob(BlobError::NotFound(_))) => {
                return Err(TonkWorkerError::NotFound(format!(
                    "blob not available: {}",
                    params.entity
                )));
            }
            Err(e) => return Err(TonkWorkerError::Internal(format!("read blob: {}", e))),
        };
        Body::from_stream(Box::pin(ExclusiveStream::new(Box::pin(chunks(reader)))))
    };

    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut response = (status, body).into_response();
    let answer = response.headers_mut();
    answer.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    answer.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if range.is_some()
        && let Ok(value) = HeaderValue::from_str(&format!("bytes {first}-{last}/{size}"))
    {
        answer.insert(header::CONTENT_RANGE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        response.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    Ok(response)
}

/// A blob reader as the chunks of a response body, ending at the reader's
/// end or at its first failure.
fn chunks(
    reader: BlobReader,
) -> impl futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> {
    stream::unfold(Some(reader), |reader| async move {
        let mut reader = reader?;
        match reader.next().await {
            Ok(Some(chunk)) => Some((Ok(axum::body::Bytes::from(chunk)), Some(reader))),
            Ok(None) => None,
            Err(error) => Some((Err(std::io::Error::other(error.to_string())), None)),
        }
    })
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

/// The asset media type attribute every writer records (`tonk:asset`).
const MEDIA_TYPE: &str = "tonk.dialog.asset/media-type";
/// The asset name attribute every writer records (`tonk:asset`).
const NAME: &str = "tonk.dialog.asset/name";
/// The media type blobs were described with before `tonk:asset`, which
/// older blobs and templates writing `tonk:blob` themselves still carry.
const LEGACY_CONTENT_TYPE: &str = "xyz.tonk.blob/content-type";
/// The legacy name attribute, read beside [`NAME`] for the same reason.
const LEGACY_NAME: &str = "xyz.tonk.blob/name";

/// The current text value of `attribute` on `entity`, if any. A failed
/// read is logged and treated as absent: these facts describe content
/// the route can still serve without them.
async fn first_text(
    branch: &dialog_repository::Branch,
    operator: &crate::worker::DefaultOperator,
    attribute: &str,
    entity: &Entity,
) -> Result<Option<String>, TonkWorkerError> {
    let attribute: Relation = attribute
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
    let stream = branch
        .claims()
        .select(ArtifactSelector::new().the(attribute).of(entity.clone()))
        .perform(operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("metadata query: {e}")))?;
    tokio::pin!(stream);
    Ok(match stream.next().await {
        Some(Ok(artifact)) => artifact
            .value()
            .ok()
            .and_then(|value| String::try_from(value).ok()),
        Some(Err(e)) => {
            log!("blob: metadata query error: {:?}", e);
            None
        }
        None => None,
    })
}

/// Move `branch` of the space at `repo` to this release's storage format,
/// once per process, before a blob import: the import commits the asset
/// fact into the branch directly, not through the reactor, and a commit
/// into a branch an earlier release wrote is refused.
async fn migrate_once(tonk: &TonkState, repo: &str, branch: &str) -> Result<(), TonkWorkerError> {
    tonk.reactor
        .repository(repo)
        .branch(branch)
        .migrate_once(&tonk.operator)
        .await
        .map(|_| ())
        .map_err(|error| {
            TonkWorkerError::Internal(format!("migrating '{repo}/{branch}' failed: {error}"))
        })
}

/// Handler for `POST /api/repository/{repo}/branch/{branch}/blob`.
///
/// Writes the request's body into the branch's content-addressed blob
/// store as it arrives, and asserts the asset's
/// `tonk.dialog.asset/media-type` and `tonk.dialog.asset/name` facts — the
/// name fact is always asserted, defaulting to an existing fact (on a
/// headerless re-upload) or the entity string when the `X-Tonk-Blob-Name`
/// header is absent — so the read route (`serve`, above) and
/// `<tonk-display model=tonk:blob>` work immediately. Idempotent by content
/// address: re-uploading the same bytes returns the same entity and
/// re-asserts the same facts.
///
/// The bytes go straight to the branch's blob store, which needs no reactor
/// involvement: it's raw content-addressed storage, not a claim. The
/// metadata facts go through the *reactor's* transaction, matching
/// `claim::assert_claim`'s commit path, which re-polls subscriptions, so
/// following the commit with `run_scheduled_polls` makes the new facts
/// visible to subscribers immediately.
#[handler]
pub async fn upload(
    State(state): State<AppState>,
    Path(path): Path<EvaluatePath>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, TonkWorkerError> {
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
    let tonk = state.read().await;

    // Ingest bytes into the content-addressed store. Direct branch access
    // (as `serve` reads from) — the blob store isn't part of the claim/fact
    // graph, so it doesn't go through the reactor.
    let repository = tonk
        .profile
        .space(&path.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {e}", path.repo))
        })?;
    migrate_once(&tonk, &path.repo, path.branch.as_str()).await?;
    let branch_handle = repository
        .branch(path.branch.as_str())
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {e}", path.branch))
        })?;

    // The body as it arrives, counted on its way into the store.
    let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = received.clone();
    let arriving = body.into_data_stream().map(move |chunk| {
        chunk
            .map(|bytes| {
                counted.fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
                bytes.to_vec()
            })
            .map_err(|error| BlobError::Storage(format!("reading the upload: {error}")))
    });
    let entity = Blob::import(Box::pin(arriving))
        .write(branch_handle.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("write blob: {e}")))?;
    let size = received.load(std::sync::atomic::Ordering::Relaxed);
    if size == 0 {
        return Err(TonkWorkerError::Router("empty upload body".to_string()));
    }

    // Assert extrinsic metadata as ordinary facts on the blob entity,
    // mirroring `tonk blob add`. Committed through the reactor (like
    // `claim::assert_claim`), then drain the scheduled poll so subscribers
    // on this branch see the new facts.
    let media_type_attr: Relation = MEDIA_TYPE
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;
    let name_attr: Relation = NAME
        .parse()
        .map_err(|e| TonkWorkerError::Internal(format!("bad attribute: {e}")))?;

    // Effective name: an explicit header wins; otherwise an already-
    // asserted name fact is preserved, current or legacy (a raw re-upload
    // must not clobber a good name with the hash default); otherwise the
    // entity string. The name fact must always land — the `tonk:asset`
    // concept query matches only rows with every field present, so a
    // nameless asset would never reach the seeded media view.
    let name = match name {
        Some(n) => n,
        None => match first_text(&branch_handle, &tonk.operator, NAME, &entity).await? {
            Some(existing) => existing,
            None => first_text(&branch_handle, &tonk.operator, LEGACY_NAME, &entity)
                .await?
                .unwrap_or_else(|| entity.to_string()),
        },
    };

    let tx = tonk
        .reactor
        .repository(&path.repo)
        .branch(&path.branch)
        .transaction()
        .assert(RawClaim {
            the: media_type_attr,
            of: entity.clone(),
            is: Value::String(content_type.clone()),
            policy: dialog_artifacts::Pick::Last,
        })
        .assert(RawClaim {
            the: name_attr,
            of: entity.clone(),
            is: Value::String(name.clone()),
            policy: dialog_artifacts::Pick::Last,
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
    use futures_util::{StreamExt as _, stream};
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    use crate::api_router_from_state;
    use crate::router::tests::{put_repo, test_state};

    /// Read back what the branch holds under `attribute` for `entity`.
    async fn fact(
        app_state: &Arc<RwLock<crate::TonkState>>,
        repo: &str,
        attribute: &str,
        entity: &str,
    ) -> Option<String> {
        let guard = app_state.read().await;
        let repository = guard
            .profile
            .space(repo)
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
        super::first_text(
            &branch,
            &guard.operator,
            attribute,
            &entity.parse().unwrap(),
        )
        .await
        .unwrap()
    }

    async fn post(
        app: &axum::Router,
        uri: String,
        body: Body,
        name: Option<&str>,
    ) -> serde_json::Value {
        let mut request = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "image/png");
        if let Some(name) = name {
            request = request.header("x-tonk-blob-name", name);
        }
        let response = app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// An upload describes the asset with the `tonk:asset` attributes
    /// alone; the legacy ones are left to blobs that already carry them.
    #[dialog_common::test]
    async fn it_records_an_upload_with_the_asset_attributes() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-asset-attributes").await;

        let json = post(
            &app,
            format!("/api/repository/{repo}/branch/main/blob"),
            Body::from(b"\x89PNG\r\n\x1a\nnew".to_vec()),
            Some("new.png"),
        )
        .await;
        let entity = json["entity"].as_str().unwrap();

        assert_eq!(
            fact(&app_state, &repo, super::MEDIA_TYPE, entity)
                .await
                .as_deref(),
            Some("image/png")
        );
        assert_eq!(
            fact(&app_state, &repo, super::NAME, entity)
                .await
                .as_deref(),
            Some("new.png")
        );
        assert_eq!(
            fact(&app_state, &repo, super::LEGACY_CONTENT_TYPE, entity).await,
            None
        );
        assert_eq!(
            fact(&app_state, &repo, super::LEGACY_NAME, entity).await,
            None
        );
    }

    /// A blob named the legacy way keeps its name when the same bytes are
    /// uploaded again without one, rather than falling back to its hash.
    #[dialog_common::test]
    async fn it_keeps_a_legacy_name_on_a_nameless_re_upload() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-legacy-name").await;
        let payload = b"\x89PNG\r\n\x1a\nlegacy".to_vec();

        let entity = post(
            &app,
            format!("/api/repository/{repo}/branch/main/blob"),
            Body::from(payload.clone()),
            Some("ignored.png"),
        )
        .await["entity"]
            .as_str()
            .unwrap()
            .to_string();
        // Make it look like a blob from before `tonk:asset`: only the
        // legacy name, through the ordinary claim routes.
        for (method, attribute, value) in [
            ("retract", super::NAME, "ignored.png"),
            ("assert", super::LEGACY_NAME, "legacy.png"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/api/repository/{repo}/branch/main/claim/{method}/{entity}/{attribute}"
                        ))
                        .method("POST")
                        .header("content-type", "text/plain")
                        .body(Body::from(value))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{method} {attribute}");
        }
        assert_eq!(fact(&app_state, &repo, super::NAME, &entity).await, None);

        let json = post(
            &app,
            format!("/api/repository/{repo}/branch/main/blob"),
            Body::from(payload),
            None,
        )
        .await;
        assert_eq!(json["name"], "legacy.png");
    }

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
                .space(&repo)
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
            "a blob described the legacy way is served with its content type",
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
            entity.starts_with("asset:"),
            "entity is an asset ref: {entity}"
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

    /// Upload `chunks` as a body that arrives in those pieces, and answer
    /// the stored asset's entity and recorded size.
    async fn upload_in_pieces(
        app: &axum::Router,
        repo: &str,
        chunks: impl futures_util::Stream<Item = Vec<u8>> + Send + 'static,
    ) -> (String, u64) {
        let body = Body::from_stream(chunks.map(Ok::<_, std::io::Error>));
        let up = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/repository/{repo}/branch/main/blob"))
                    .method("POST")
                    .header("content-type", "application/octet-stream")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(up.status(), StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(up.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        (
            json["entity"].as_str().unwrap().to_owned(),
            json["size"].as_u64().unwrap(),
        )
    }

    /// Ask the blob route for `entity`, with `method` and an optional
    /// `Range` header.
    async fn read(
        app: &axum::Router,
        repo: &str,
        entity: &str,
        method: &str,
        range: Option<&str>,
    ) -> axum::response::Response {
        let mut request = Request::builder()
            .uri(format!("/api/repository/{repo}/branch/main/blob/{entity}"))
            .method(method);
        if let Some(range) = range {
            request = request.header("range", range);
        }
        app.clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// The bytes `index` of a payload the tests make up as it is read.
    fn byte(index: usize) -> u8 {
        (index % 251) as u8
    }

    /// An upload is written as it arrives, so its size is not bounded by
    /// what the worker can hold: this one is larger than the 64 MiB the
    /// route used to buffer, and arrives in pieces made as it is read.
    #[dialog_common::test]
    async fn it_takes_an_upload_larger_than_it_could_hold_at_once() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-streamed").await;

        const PIECE: usize = 1024 * 1024;
        const PIECES: usize = 65;
        let pieces = stream::iter(0..PIECES).map(|piece| {
            (piece * PIECE..(piece + 1) * PIECE)
                .map(byte)
                .collect::<Vec<u8>>()
        });
        let (entity, size) = upload_in_pieces(&app, &repo, pieces).await;
        assert_eq!(size, (PIECE * PIECES) as u64);

        // Its last bytes read back as they were written.
        let tail = read(&app, &repo, &entity, "GET", Some("bytes=-16")).await;
        assert_eq!(tail.status(), StatusCode::PARTIAL_CONTENT);
        let total = PIECE * PIECES;
        assert_eq!(
            tail.headers()["content-range"],
            format!("bytes {}-{}/{total}", total - 16, total - 1).as_str()
        );
        let bytes = axum::body::to_bytes(tail.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            bytes.as_ref(),
            (total - 16..total)
                .map(byte)
                .collect::<Vec<u8>>()
                .as_slice()
        );
    }

    /// A read answers the range a `Range` header asks for, says what it
    /// answered, and refuses one past the end.
    #[dialog_common::test]
    async fn it_answers_the_range_a_read_asks_for() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-ranges").await;

        let payload: Vec<u8> = (0..10_000).map(byte).collect();
        let (entity, _) = upload_in_pieces(
            &app,
            &repo,
            stream::iter(
                payload
                    .chunks(3_000)
                    .map(<[u8]>::to_vec)
                    .collect::<Vec<_>>(),
            ),
        )
        .await;

        let whole = read(&app, &repo, &entity, "GET", None).await;
        assert_eq!(whole.status(), StatusCode::OK);
        assert_eq!(whole.headers()["accept-ranges"], "bytes");
        assert_eq!(whole.headers()["content-length"], "10000");
        let bytes = axum::body::to_bytes(whole.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), payload.as_slice());

        for (asked, first, last) in [
            ("bytes=100-199", 100, 199),
            ("bytes=9990-", 9_990, 9_999),
            ("bytes=-10", 9_990, 9_999),
            ("bytes=9995-20000", 9_995, 9_999),
        ] {
            let part = read(&app, &repo, &entity, "GET", Some(asked)).await;
            assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT, "{asked}");
            assert_eq!(
                part.headers()["content-range"],
                format!("bytes {first}-{last}/10000").as_str(),
                "{asked}"
            );
            assert_eq!(
                part.headers()["content-length"],
                (last - first + 1).to_string().as_str(),
                "{asked}"
            );
            let bytes = axum::body::to_bytes(part.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(bytes.as_ref(), &payload[first..=last], "{asked}");
        }

        let past = read(&app, &repo, &entity, "GET", Some("bytes=10000-")).await;
        assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(past.headers()["content-range"], "bytes */10000");

        // Several ranges are answered with the whole.
        let several = read(&app, &repo, &entity, "GET", Some("bytes=0-1,5-6")).await;
        assert_eq!(several.status(), StatusCode::OK);
    }

    /// A `HEAD` says how large the asset is and what it is, and sends none
    /// of it.
    #[dialog_common::test]
    async fn it_answers_a_head_without_the_bytes() {
        let tonk = test_state().await;
        let app_state = Arc::new(RwLock::new(tonk));
        let (app, _lsp) = api_router_from_state(app_state.clone());
        let repo = put_repo(&app, "blob-head").await;
        let payload: Vec<u8> = (0..4_096).map(byte).collect();
        let (entity, _) = upload_in_pieces(&app, &repo, stream::iter([payload])).await;

        let head = read(&app, &repo, &entity, "HEAD", None).await;
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()["content-length"], "4096");
        assert_eq!(head.headers()["accept-ranges"], "bytes");
        let bytes = axum::body::to_bytes(head.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(bytes.is_empty());
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

// Compatibility for images deferred by older Welcome-space builds.
fn internal(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Internal(format!("welcome media: {error}"))
}
#[derive(Deserialize)]
struct Media {
    entity: dialog_artifacts::Entity,
    url: String,
}

fn bundled_media() -> Result<Vec<Media>, TonkWorkerError> {
    serde_json::from_str(include_str!(
        "../../../tonk-core/assets/library/onboarding-media.json"
    ))
    .map_err(internal)
}

async fn import_blob(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    expected: &dialog_artifacts::Entity,
    bytes: Vec<u8>,
) -> Result<(), TonkWorkerError> {
    // Verify before writing anything, including the branch's blob index.
    if &dialog_artifacts::Entity::from_blob(blake3::hash(&bytes).as_bytes()).map_err(internal)?
        != expected
    {
        return Err(internal("bundled blob hash mismatch"));
    }
    let chunks = futures_util::stream::iter(vec![Ok::<_, dialog_effects::blob::BlobError>(bytes)]);
    let entity = Blob::import(chunks)
        .write(branch.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    if &entity != expected {
        return Err(internal("bundled blob hash mismatch"));
    }
    Ok(())
}

async fn hydrate_media(
    state: &AppState,
    key: &str,
    branch_name: &str,
    entity: &dialog_artifacts::Entity,
) -> Result<(), TonkWorkerError> {
    if !bundled_media()?.iter().any(|media| &media.entity == entity) {
        return Ok(());
    }
    // Blob imports advance the branch index; serialize with other writer routes.
    let tonk = state.write().await;
    let repository = tonk
        .profile
        .space(key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    migrate_once(&tonk, key, branch_name).await?;
    let branch = repository
        .branch(branch_name)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    ensure_media(&tonk, &branch, entity).await?;
    Ok(())
}

/// Only this immutable, compiled-in allowlist can fill a missing starter image.
/// The caller has already resolved its authorized repository and branch. Bytes
/// become ordinary branch blobs, so export/sync and later offline reads work.
async fn ensure_media(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    entity: &dialog_artifacts::Entity,
) -> Result<bool, TonkWorkerError> {
    let Some(media) = bundled_media()?
        .into_iter()
        .find(|media| &media.entity == entity)
    else {
        return Ok(false);
    };
    match Blob::from(entity.clone())
        .read(branch.blobs())
        .perform(&tonk.operator)
        .await
    {
        Ok(_) => return Ok(true),
        Err(dialog_repository::CommitError::Blob(dialog_effects::blob::BlobError::NotFound(_))) => {
        }
        Err(error) => return Err(internal(error)),
    }
    let bytes = fetch_media(&media.url).await?;
    import_blob(tonk, branch, entity, bytes).await?;
    Ok(true)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_futures::JsFuture;
    #[wasm_bindgen::prelude::wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(catch, js_name = tonkBundledAsset)]
        async fn bundled_asset(path: &str) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
    }
    let response: web_sys::Response = bundled_asset(url)
        .await
        .and_then(|v| v.dyn_into())
        .map_err(|e| internal(format!("fetch {url}: {e:?}")))?;
    if !response.ok() {
        return Err(internal(format!("fetch {url}: HTTP {}", response.status())));
    }
    let bytes = JsFuture::from(
        response
            .array_buffer()
            .map_err(|e| internal(format!("media body: {e:?}")))?,
    )
    .await
    .map_err(|e| internal(format!("media body: {e:?}")))?;
    Ok(js_sys::Uint8Array::new(&bytes).to_vec())
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    match url {
        "/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp").to_vec()),
        "/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp").to_vec()),
        _ => Err(internal("unknown bundled media")),
    }
}

#[cfg(test)]
mod range_tests {
    use super::byte_range;

    #[dialog_common::test]
    fn it_reads_the_forms_of_a_single_byte_range() {
        assert_eq!(byte_range(Some("bytes=0-9"), 100), Ok(Some((0, 9))));
        assert_eq!(byte_range(Some("bytes=90-"), 100), Ok(Some((90, 99))));
        assert_eq!(byte_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(byte_range(Some("bytes=95-500"), 100), Ok(Some((95, 99))));
        assert_eq!(byte_range(Some("bytes=-500"), 100), Ok(Some((0, 99))));
    }

    #[dialog_common::test]
    fn it_answers_the_whole_where_it_serves_no_range() {
        for header in [
            None,
            Some("items=0-9"),
            Some("bytes=0-1,5-6"),
            Some("bytes=9-0"),
            Some("bytes=-0"),
            Some("bytes=nonsense"),
        ] {
            assert_eq!(byte_range(header, 100), Ok(None), "{header:?}");
        }
    }

    #[dialog_common::test]
    fn it_refuses_a_range_past_the_end() {
        assert_eq!(byte_range(Some("bytes=100-"), 100), Err(()));
        assert_eq!(byte_range(Some("bytes=0-"), 0), Err(()));
    }
}
