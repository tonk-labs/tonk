//! `slide export` / `slide import` — CSV round-trip of a branch's
//! artifacts.
//!
//! Wraps dialog's `Branch::export()` / `Branch::import()` with
//! `dialog_csv`'s CSV exporter / importer. Like [`crate::sync`],
//! this talks to dialog directly — slide has no SSE clients, so it
//! skips the reactor wrapper the worker uses.

use std::path::PathBuf;

use dialog_csv::{CsvExporter, CsvImporter};
use dialog_repository::Revision;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

use crate::ExitCode;
use crate::site::SlideSite;

/// Failure modes for [`export`] / [`import`].
#[derive(Debug, Error)]
pub enum TransferError {
    /// An I/O error reading or writing a file / stdout.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// The dialog export stream failed.
    #[error("export failed: {0}")]
    Export(dialog_artifacts::DialogArtifactsError),

    /// The dialog import / commit failed.
    #[error("import failed: {0}")]
    Import(dialog_repository::CommitError),
}

impl TransferError {
    /// Map to a process exit code.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            TransferError::Io(_) => ExitCode::IoError,
            TransferError::Export(_) | TransferError::Import(_) => ExitCode::CommitError,
        }
    }
}

/// Where an [`export`] writes its CSV.
pub enum Destination {
    /// Write to the given file path.
    File(PathBuf),
    /// Write to stdout.
    Stdout,
}

/// Export every artifact on the site's `main` branch as CSV to
/// `destination`. Returns the number of bytes written.
pub async fn export(site: &SlideSite, destination: Destination) -> Result<usize, TransferError> {
    // Buffer in memory: the dialog exporter wants an `AsyncWrite`,
    // and we want a byte count + a single flush to the real sink.
    let mut buf: Vec<u8> = Vec::new();
    let session = site
        .branch()
        .await
        .map_err(|e| std::io::Error::other(format!("acquire branch: {e}")))?;
    session
        .handle()
        .export(CsvExporter::from(&mut buf))
        .perform(&site.operator)
        .await
        .map_err(TransferError::Export)?;

    let len = buf.len();
    match destination {
        Destination::File(path) => {
            let mut file = tokio::fs::File::create(&path).await?;
            file.write_all(&buf).await?;
            file.flush().await?;
        }
        Destination::Stdout => {
            let mut out = tokio::io::stdout();
            out.write_all(&buf).await?;
            out.flush().await?;
        }
    }
    Ok(len)
}

/// Import artifacts from a CSV file at `path`, committing each row
/// as an assertion on the site's `main` branch in one transaction.
/// Returns the post-commit revision.
pub async fn import(site: &SlideSite, path: &PathBuf) -> Result<Revision, TransferError> {
    let file = tokio::fs::File::open(path).await?;
    let importer = CsvImporter::from(file);
    let session = site
        .branch()
        .await
        .map_err(|e| std::io::Error::other(format!("acquire branch: {e}")))?;
    session
        .handle()
        .import(importer)
        .perform(&site.operator)
        .await
        .map_err(TransferError::Import)
}
