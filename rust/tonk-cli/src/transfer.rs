//! `tonk export` / `tonk import` — CSV round-trip of a branch's
//! artifacts.
//!
//! Wraps dialog's `Branch::export()` / `Branch::import()` with
//! `dialog_csv`'s CSV exporter / importer. Like [`crate::sync`],
//! this talks to dialog directly — tonk has no SSE clients, so it
//! skips the reactor wrapper the worker uses.

use std::path::PathBuf;

use dialog_csv::{CsvExporter, CsvImporter};
use dialog_repository::Revision;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

use crate::ExitCode;
use crate::site::TonkSite;

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
/// [`export`] for a named branch.
///
/// Branches carry separate data and migrate separately, so an upgrade walks
/// them one at a time rather than assuming `main` is the whole space.
pub async fn export_branch(
    site: &TonkSite,
    branch: &str,
    destination: Destination,
) -> Result<usize, TransferError> {
    let session = site
        .named_branch(branch)
        .await
        .map_err(|e| std::io::Error::other(format!("acquire branch {branch:?}: {e}")))?;
    export_session(&session, site, destination).await
}

/// Write the site's `main` branch out as CSV.
pub async fn export(site: &TonkSite, destination: Destination) -> Result<usize, TransferError> {
    let session = site
        .branch()
        .await
        .map_err(|e| std::io::Error::other(format!("acquire branch: {e}")))?;
    export_session(&session, site, destination).await
}

/// Write one already-acquired branch out as CSV.
async fn export_session(
    session: &dialog_reactor::BranchSession,
    site: &TonkSite,
    destination: Destination,
) -> Result<usize, TransferError> {
    // Buffer in memory: the dialog exporter wants an `AsyncWrite`,
    // and we want a byte count + a single flush to the real sink.
    let mut buf: Vec<u8> = Vec::new();
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
/// [`import`] onto a named branch.
pub async fn import_branch(
    site: &TonkSite,
    branch: &str,
    path: &PathBuf,
) -> Result<Revision, TransferError> {
    let file = tokio::fs::File::open(path).await?;
    let session = site
        .named_branch(branch)
        .await
        .map_err(|e| std::io::Error::other(format!("acquire branch {branch:?}: {e}")))?;
    session
        .handle()
        .import(CsvImporter::from(file))
        .perform(&site.operator)
        .await
        .map_err(TransferError::Import)
}

/// Import artifacts from a CSV file onto the site's `main` branch,
/// committing each row as an assertion in one transaction.
pub async fn import(site: &TonkSite, path: &PathBuf) -> Result<Revision, TransferError> {
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
