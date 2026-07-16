//! `tonk blob` — content-addressed blob ingest, readback, listing.
//!
//! `add` streams a local file into the branch's blob store
//! (`Blob::import(..).write(branch.blobs())`), which returns the
//! content-addressed `blob:<hash>` [`Entity`] directly, and asserts
//! extrinsic metadata (content type, file name) as ordinary facts on
//! that entity using the `tonk:blob` concept's attributes
//! (`xyz.tonk.blob/content-type`, `xyz.tonk.blob/name`) from the
//! standard library. `cat` reads a blob's bytes back out by
//! reference. `ls` is deferred: the current dialog-db pin exposes no
//! blob-enumeration API (see [`ls`]).

use std::path::Path;

use dialog_artifacts::{ArtifactSelector, Entity};
use dialog_effects::blob::BlobError as UpstreamBlobError;
use dialog_query::Attribute;
use dialog_reactor::BranchSession;
use dialog_repository::{Blob, CommitError};
use thiserror::Error;

use crate::site::TonkSite;

/// Failure modes for [`add`] (and, later, `cat`/`ls`).
#[derive(Debug, Error)]
pub enum BlobError {
    /// Reading the source file failed.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// Acquiring the branch, writing the blob, or asserting its
    /// metadata failed.
    #[error("{0}")]
    Site(String),
    /// The referenced blob isn't available locally or from the
    /// remote.
    #[error("blob not available locally or from the remote: {0}")]
    NotFound(String),
    /// The subcommand isn't available on the current dialog-db pin.
    #[error("{0}")]
    Unsupported(String),
}

impl BlobError {
    /// Map to a process exit code.
    pub fn exit_code(&self) -> crate::ExitCode {
        match self {
            BlobError::Io(_) => crate::ExitCode::IoError,
            BlobError::Site(_) => crate::ExitCode::CommitError,
            BlobError::NotFound(_) => crate::ExitCode::IoError,
            BlobError::Unsupported(_) => crate::ExitCode::CommitError,
        }
    }
}

/// Result of [`add`]: the blob's content-addressed entity plus the
/// metadata that was asserted alongside it.
pub struct AddOutcome {
    /// The `blob:<hash>` entity the bytes were stored under.
    pub entity: Entity,
    /// Size of the ingested file, in bytes.
    pub size: u64,
    /// MIME type asserted for the blob (inferred or overridden).
    pub content_type: String,
}

/// Map a file extension to a MIME type. Keep in sync with
/// tonk-worker's `mime_for_extension` until the two are
/// consolidated (planned for the display milestone).
fn mime_for_extension(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html".to_string(),
        "css" => "text/css".to_string(),
        "js" | "mjs" => "application/javascript".to_string(),
        "json" => "application/json".to_string(),
        "txt" => "text/plain".to_string(),
        "md" => "text/markdown".to_string(),
        "svg" => "image/svg+xml".to_string(),
        "png" => "image/png".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "wasm" => "application/wasm".to_string(),
        other => format!("application/{other}"),
    }
}

/// Ingest `path` into the site's blob store, returning its
/// content-addressed reference. Idempotent: adding the same bytes
/// twice yields the same `blob:<hash>` entity both times.
///
/// `content_type` overrides the extension-inferred MIME type.
/// Asserts `xyz.tonk.blob/content-type` (always) and
/// `xyz.tonk.blob/name` (always; defaults to the entity string when
/// `path` has no file name) on the blob entity in one transaction.
pub async fn add(
    site: &TonkSite,
    path: &Path,
    content_type: Option<String>,
) -> Result<AddOutcome, BlobError> {
    let content_type = content_type.unwrap_or_else(|| {
        path.extension()
            .and_then(|e| e.to_str())
            .map(mime_for_extension)
            .unwrap_or_else(|| "application/octet-stream".to_string())
    });
    let name = path.file_name().and_then(|n| n.to_str()).map(str::to_owned);

    let file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();

    // Stream the file lazily: the blob import takes a fallible stream
    // (`Stream<Item = Result<Vec<u8>, dialog_effects::blob::BlobError>>`),
    // so a mid-read I/O error propagates through the write (and
    // surfaces here as `BlobError::Site`) instead of either buffering
    // the whole file up front to check for it, or silently dropping it
    // by filtering a fallible reader-stream down to an infallible one
    // (which would ingest a truncated file "successfully" under a
    // valid hash). The whole file is never held in memory at once.
    use futures_util::StreamExt as _;
    let source = tokio_util::io::ReaderStream::new(file).map(|chunk| {
        chunk
            .map(|bytes| bytes.to_vec())
            .map_err(|e| dialog_effects::blob::BlobError::Io(e.to_string()))
    });

    let session = site
        .branch()
        .await
        .map_err(|e| BlobError::Site(format!("acquire branch: {e}")))?;
    // `Blob::import(..).write(..)` returns the content-addressed
    // `blob:<hash>` entity directly (entity-keyed blob API).
    let entity = Blob::import(Box::pin(source))
        .write(session.handle().blobs())
        .perform(&site.operator)
        .await
        .map_err(|e| BlobError::Site(format!("write blob: {e}")))?;

    // Extrinsic metadata as ordinary facts on the blob entity.
    // `name` always lands: the `tonk:blob` concept query behind the
    // seeded media view matches only rows with every field present, so
    // a nameless blob would never render. A path with no file name
    // (`.`/`..`) falls back to the content-addressed entity string.
    let tx = session
        .handle()
        .transaction()
        .assert(ContentType::of(entity.clone()).is(content_type.clone()))
        .assert(Name::of(entity.clone()).is(name.unwrap_or_else(|| entity.to_string())));
    tx.commit()
        .perform(&site.operator)
        .await
        .map_err(|e| BlobError::Site(format!("assert metadata: {e}")))?;

    Ok(AddOutcome {
        entity,
        size,
        content_type,
    })
}

/// Write a blob's bytes to `out`, returning the number of bytes
/// written.
///
/// `reference` must be a `blob:<hash>` [`Entity`] URI, as returned
/// by [`add`]. A reference that doesn't parse as a `blob:` entity
/// is rejected before touching the branch. A well-formed reference
/// that isn't available locally or from the remote surfaces as
/// [`BlobError::NotFound`].
pub async fn cat(
    site: &TonkSite,
    reference: &str,
    out: &mut (impl tokio::io::AsyncWrite + Unpin),
) -> Result<u64, BlobError> {
    let entity: Entity = reference
        .parse()
        .map_err(|e| BlobError::Site(format!("invalid reference: {e}")))?;
    if entity.blob_hash().is_none() {
        return Err(BlobError::Site(format!(
            "not a blob reference: {reference}"
        )));
    }

    let session = site
        .branch()
        .await
        .map_err(|e| BlobError::Site(format!("acquire branch: {e}")))?;
    let mut reader = match Blob::from(entity)
        .read(session.handle().blobs())
        .perform(&site.operator)
        .await
    {
        Ok(r) => r,
        Err(CommitError::Blob(UpstreamBlobError::NotFound(_))) => {
            return Err(BlobError::NotFound(format!(
                "blob not available locally or from the remote: {reference}"
            )));
        }
        Err(e) => {
            return Err(BlobError::Site(format!("read blob: {e}")));
        }
    };

    use tokio::io::AsyncWriteExt as _;
    let mut written = 0u64;
    while let Some(chunk) = reader
        .next()
        .await
        .map_err(|e| BlobError::Site(format!("read: {e}")))?
    {
        written += chunk.len() as u64;
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    Ok(written)
}

/// One row of [`ls`]: a blob's entity, size, and content type (if
/// the `xyz.tonk.blob/content-type` fact was ever asserted on it).
pub struct LsRow {
    /// The blob's `blob:<hash>` entity.
    pub entity: Entity,
    /// Size in bytes, from the branch's blob index.
    pub size: u64,
    /// Asserted MIME type, if any.
    pub content_type: Option<String>,
}

/// Enumerate every blob referenced by the branch's current tree,
/// paired with its size and (best-effort) content type.
///
/// Deferred: the current dialog-db pin's blob API is entity-keyed
/// (read/write/size by `blob:<hash>`) and exposes no way to
/// enumerate the blobs a branch holds. `ls` returns an
/// [`BlobError::Unsupported`] until the pin advances to a dialog-db
/// that offers a blob-enumeration API (at which point the
/// [`query_content_type`] helper below wires the per-blob content
/// type back in).
pub async fn ls(_site: &TonkSite) -> Result<Vec<LsRow>, BlobError> {
    Err(BlobError::Unsupported(
        "tonk blob ls is not supported on the current dialog-db pin \
         (no blob-enumeration API)"
            .to_string(),
    ))
}

/// Look up the `xyz.tonk.blob/content-type` fact for `entity`,
/// returning `None` if it was never asserted.
///
/// Goes straight at the branch's raw claims index
/// (`Branch::claims().select(ArtifactSelector)`) rather than the
/// `Query::<Concept>`-and-`try_vec` surface `remote.rs` uses for
/// `RemoteConcept`: that surface exists to bind several attributes
/// of a whole concept at once via a `#[derive(Concept)]` struct,
/// which would mean inventing a one-field concept type just to ask
/// "what's the value of this single attribute on this single
/// entity". `ArtifactSelector::new().the(..).of(..)` expresses
/// exactly that constraint directly, with no extra type needed.
///
/// Retained for the deferred [`ls`]: it's the per-blob content-type
/// lookup that a future blob-enumeration API will pair with each
/// listed entity.
#[allow(dead_code)]
async fn query_content_type(
    session: &BranchSession,
    operator: &dialog_operator::Operator<dialog_storage::provider::storage::NativeSpace>,
    entity: &Entity,
) -> Result<Option<String>, BlobError> {
    use futures_util::StreamExt as _;

    let selector = ArtifactSelector::new()
        .the(ContentType::the().into())
        .of(entity.clone());

    let artifacts = session
        .handle()
        .claims()
        .select(selector)
        .perform(operator)
        .await
        .map_err(|e| BlobError::Site(format!("content-type query: {e}")))?;
    let mut artifacts = Box::pin(artifacts);

    match artifacts.next().await {
        Some(Ok(artifact)) => {
            let value = String::try_from(artifact.is)
                .map_err(|e| BlobError::Site(format!("content-type decode: {e}")))?;
            Ok(Some(value))
        }
        Some(Err(e)) => Err(BlobError::Site(format!("content-type query: {e}"))),
        None => Ok(None),
    }
}

/// MIME type of a blob's content. Matches the standard library's
/// `tonk:blob` concept (`xyz.tonk.blob/content-type`).
#[derive(Attribute, Clone)]
#[domain("xyz.tonk.blob")]
struct ContentType(String);

/// Human-readable file name of a blob (optional). Matches the
/// standard library's `tonk:blob` concept (`xyz.tonk.blob/name`).
#[derive(Attribute, Clone)]
#[domain("xyz.tonk.blob")]
struct Name(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// The `#[derive(Attribute)]` naming convention (struct name to
    /// kebab-case) must land on exactly the URIs Task 8 put in
    /// `core.yaml`'s `tonk:blob` concept. If this ever drifts, the
    /// yaml and the derive have gone out of sync and one of them
    /// needs to change — not this test.
    #[test]
    fn derived_attribute_uris_match_the_standard_library() {
        assert_eq!(ContentType::the().to_string(), "xyz.tonk.blob/content-type");
        assert_eq!(Name::the().to_string(), "xyz.tonk.blob/name");
    }
}
