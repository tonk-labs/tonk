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
use futures_util::StreamExt as _;
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
    Export(dialog_corpus::DialogCorpusError),

    /// The dialog import / commit failed.
    #[error("import failed: {0}")]
    Import(dialog_repository::CommitError),

    /// Whatever read stdout closed it before the export finished, as
    /// `tonk export | head` does. Not a failure: the reader got what it
    /// asked for, but the export was not written out in full.
    #[error("stdout closed before the export finished")]
    ReaderClosed,
}

impl crate::Coded for TransferError {
    /// Map to a process exit code.
    fn exit_code(&self) -> ExitCode {
        match self {
            TransferError::Io(_) => ExitCode::IoError,
            TransferError::ReaderClosed => ExitCode::Success,
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

/// What a CSV import would feed to the branch without committing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportPlan {
    /// Compatible artifacts that would be asserted.
    pub artifacts: usize,
    /// Incompatible rows the committing importer would skip.
    pub skipped: usize,
}

/// Read and validate a CSV import without opening or changing a branch.
pub async fn plan_import(path: &PathBuf) -> Result<ImportPlan, TransferError> {
    let file = tokio::fs::File::open(path).await?;
    let mut importer = CsvImporter::from(file);
    let mut plan = ImportPlan {
        artifacts: 0,
        skipped: 0,
    };
    while let Some(row) = importer.next().await {
        match row {
            Ok(_) => plan.artifacts += 1,
            Err(_) => plan.skipped += 1,
        }
    }
    Ok(plan)
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
        Destination::Stdout => write_stdout(&mut tokio::io::stdout(), &buf).await?,
    }
    Ok(len)
}

/// Write `buf` to stdout, treating a reader that hung up early (a
/// broken pipe) as [`TransferError::ReaderClosed`] rather than an I/O
/// failure. Any other write error is still an error.
async fn write_stdout<W>(out: &mut W, buf: &[u8]) -> Result<(), TransferError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let written = match out.write_all(buf).await {
        Ok(()) => out.flush().await,
        Err(err) => Err(err),
    };
    match written {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
            Err(TransferError::ReaderClosed)
        }
        Err(err) => Err(err.into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// A sink whose writes and flushes all fail with one error kind.
    struct Failing(std::io::ErrorKind);

    impl tokio::io::AsyncWrite for Failing {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(self.0.into()))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(self.0.into()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[dialog_common::test]
    async fn it_reports_a_closed_reader_as_success() {
        let mut out = Failing(std::io::ErrorKind::BrokenPipe);
        let err = write_stdout(&mut out, b"a,b\n").await.unwrap_err();
        assert!(matches!(err, TransferError::ReaderClosed), "{err:?}");
        assert_eq!(crate::Coded::exit_code(&err), ExitCode::Success);
    }

    #[dialog_common::test]
    async fn it_still_fails_on_other_write_errors() {
        let mut out = Failing(std::io::ErrorKind::PermissionDenied);
        let err = write_stdout(&mut out, b"a,b\n").await.unwrap_err();
        assert!(matches!(err, TransferError::Io(_)), "{err:?}");
        assert_eq!(crate::Coded::exit_code(&err), ExitCode::IoError);
    }

    #[dialog_common::test]
    async fn it_writes_the_whole_buffer() {
        let mut out = Vec::new();
        write_stdout(&mut out, b"a,b\n").await.unwrap();
        assert_eq!(out, b"a,b\n");
    }
}
