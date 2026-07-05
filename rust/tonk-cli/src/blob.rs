//! `tonk blob` — content-addressed blob ingest, readback, listing.
//!
//! `add` streams a local file into the branch's blob store
//! (`Branch::write_blob`), derives the content-addressed `blob:<hash>`
//! [`Entity`] from the discovered hash, and asserts extrinsic
//! metadata (content type, file name) as ordinary facts on that
//! entity using the `tonk:blob` concept's attributes
//! (`xyz.tonk.blob/content-type`, `xyz.tonk.blob/name`) from the
//! standard library. Cat/Ls land in a later task.

use std::path::Path;

use dialog_artifacts::Entity;
use dialog_query::Attribute;
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
    /// remote. Reserved for `cat`/`ls` (Task 10).
    #[error("blob not available locally or from the remote: {0}")]
    NotFound(String),
}

impl BlobError {
    /// Map to a process exit code.
    pub fn exit_code(&self) -> crate::ExitCode {
        match self {
            BlobError::Io(_) => crate::ExitCode::IoError,
            BlobError::Site(_) => crate::ExitCode::CommitError,
            BlobError::NotFound(_) => crate::ExitCode::IoError,
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
/// `xyz.tonk.blob/name` (when `path` has a file name) on the blob
/// entity in one transaction.
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
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned);

    let mut file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();

    // Hand-rolled read loop rather than `tokio_util::io::ReaderStream`:
    // `write_blob` wants `Stream<Item = Vec<u8>>` (no `Result`), and
    // reading the whole file into fixed-size chunks up front is no
    // more work than adapting a fallible reader-stream and filtering
    // out its `Result` wrapper. `write_blob` reads the stream exactly
    // once either way.
    use tokio::io::AsyncReadExt as _;
    const CHUNK_SIZE: usize = 64 * 1024;
    let mut chunks = Vec::new();
    loop {
        let mut buf = vec![0u8; CHUNK_SIZE];
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        buf.truncate(n);
        chunks.push(buf);
    }
    let source = futures_util::stream::iter(chunks);

    let session = site
        .branch()
        .await
        .map_err(|e| BlobError::Site(format!("acquire branch: {e}")))?;
    let hash = session
        .handle()
        .write_blob(Box::pin(source))
        .perform(&site.operator)
        .await
        .map_err(|e| BlobError::Site(format!("write blob: {e}")))?;

    let entity = Entity::from_blob(hash.as_bytes())
        .map_err(|e| BlobError::Site(format!("blob entity: {e}")))?;

    // Extrinsic metadata as ordinary facts on the blob entity.
    let mut tx = session
        .handle()
        .transaction()
        .assert(ContentType::of(entity.clone()).is(content_type.clone()));
    if let Some(n) = name {
        tx = tx.assert(Name::of(entity.clone()).is(n));
    }
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
