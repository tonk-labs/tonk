//! [`Export`] — wrap [`dialog_repository::Branch::export`] so a
//! route or CLI can stream a branch's artifacts into any
//! [`Exporter`] (e.g. `dialog_csv::CsvExporter`) without reaching
//! for dialog handles directly.
//!
//! Read-only: export never mutates the branch, so unlike
//! [`super::Pull`] / [`super::transaction::Commit`] it does not
//! re-poll subscriptions.

use dialog_artifacts::{DialogArtifactsError, Exporter};

use super::BranchReference;
use super::env::{BranchOpenProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// Export-all-artifacts effect. Generic over the [`Exporter`] sink
/// so the format (CSV, …) is the caller's concern.
pub struct Export<'a, E> {
    /// The branch to export from.
    pub branch: BranchReference<'a>,
    /// The sink the artifacts stream into.
    pub exporter: E,
}

impl<'a, E: Exporter> Export<'a, E> {
    /// Build a new `Export` effect.
    pub fn new(branch: BranchReference<'a>, exporter: E) -> Self {
        Self { branch, exporter }
    }

    /// Execute the export, writing every artifact on the branch
    /// into the exporter. Returns once the exporter is closed.
    pub async fn perform<Env>(self, env: &Env) -> Result<(), ExportError>
    where
        Env: LoadProvider + BranchOpenProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;
        cached
            .handle()
            .export(self.exporter)
            .perform(env)
            .await
            .map_err(ExportError::Export)?;
        Ok(())
    }
}

/// Failure modes of an [`Export`].
#[derive(Debug)]
pub enum ExportError {
    /// The branch could not be resolved / opened.
    Reactor(ReactorError),
    /// The dialog export stream failed.
    Export(DialogArtifactsError),
}

impl From<ReactorError> for ExportError {
    fn from(error: ReactorError) -> Self {
        ExportError::Reactor(error)
    }
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Reactor(e) => write!(f, "{e}"),
            ExportError::Export(e) => write!(f, "export failed: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}
