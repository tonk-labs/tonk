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
    answer(
        &state,
        &params.repo,
        &params.branch,
        entity,
        method,
        headers,
    )
    .await
}

/// Answer `entity`, a blob of `branch` of the space `repo`, as [`serve`]
/// does: whole, or the range `headers` ask for, under its media type.
pub(super) async fn answer(
    state: &AppState,
    repo: &str,
    branch: &str,
    entity: Entity,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, TonkWorkerError> {
    let params = BlobPath {
        repo: repo.to_owned(),
        branch: branch.to_owned(),
        entity: entity.to_string(),
    };
    if entity.blob_hash().is_none() {
        return Err(TonkWorkerError::Router(format!(
            "Not a blob reference: {}",
            params.entity
        )));
    }

    hydrate_media(state, &params.repo, &params.branch, &entity).await?;

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
pub(super) const MEDIA_TYPE: &str = "tonk.dialog.asset/media-type";
/// The asset name attribute every writer records (`tonk:asset`).
pub(super) const NAME: &str = "tonk.dialog.asset/name";
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
    // The hash is what is compared: an older space names its media as a
    // `blob:` and the store now names the same bytes an `asset:`.
    let hash = expected.blob_hash();
    if hash.as_ref().map(|hash| hash.as_slice()) != Some(blake3::hash(&bytes).as_bytes().as_slice())
    {
        return Err(internal("bundled blob hash mismatch"));
    }
    let chunks = futures_util::stream::iter(vec![Ok::<_, dialog_effects::blob::BlobError>(bytes)]);
    let entity = Blob::import(chunks)
        .write(branch.blobs())
        .perform(&tonk.operator)
        .await
        .map_err(internal)?;
    if entity.blob_hash() != hash {
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

/// A kernel reads the media where the deployment serves its library, as
/// anyone would: it carries no copy of its own.
#[cfg(target_arch = "wasm32")]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    let read = tonk_kernel_world::application::read_bytes(url)
        .await
        .map_err(|error| internal(format!("{error:?}")))?;
    if !(200..300).contains(&read.status) {
        return Err(internal(format!("{url} answered {}", read.status)));
    }
    Ok(read.body)
}

/// A worker with no deployment to read from carries the media.
#[cfg(not(target_arch = "wasm32"))]
async fn fetch_media(url: &str) -> Result<Vec<u8>, TonkWorkerError> {
    match url {
        "/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-CmZq5URy2ucFKNyZCFUtHiNvEaUNK4Z8UstvzwbjREFG.webp").to_vec()),
        "/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp" => Ok(include_bytes!("../../../tonk-core/assets/library/welcome-9hKHdfALCDKRL5z3Xkn2JUM72DWSzsSBwAvdbyaPF2sU.webp").to_vec()),
        _ => Err(internal("unknown bundled media")),
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod media_tests {
    use super::{bundled_media, fetch_media};

    #[dialog_common::test]
    async fn it_bundles_each_medium_as_the_blob_it_is_listed_as() {
        for media in bundled_media().unwrap() {
            let bytes = fetch_media(&media.url).await.unwrap();
            assert_eq!(
                media.entity.blob_hash().map(|hash| hash.to_vec()),
                Some(blake3::hash(&bytes).as_bytes().to_vec()),
                "{}",
                media.url
            );
        }
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
