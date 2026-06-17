//! [`Import`] — wrap [`dialog_repository::Branch::import`] so the
//! imported artifacts commit as assertions and every subscription
//! on the branch re-polls, the same notify-on-commit discipline
//! [`super::Pull`] and [`super::transaction::Commit`] follow. The
//! caller supplies any [`Importer`] (e.g. `dialog_csv::CsvImporter`).

use dialog_common::ConditionalSend;
use dialog_repository::{CommitError, Importer, Revision};

use super::BranchReference;
use super::env::{BranchOpenProvider, CommitProvider, LoadProvider, SelectProvider};
use super::error::ReactorError;

/// Import-artifacts effect. Generic over the [`Importer`] source so
/// the format (CSV, …) is the caller's concern.
pub struct Import<'a, I> {
    /// The branch to import into.
    pub branch: BranchReference<'a>,
    /// The artifact source.
    pub importer: I,
}

impl<'a, I: Importer + Unpin + ConditionalSend> Import<'a, I> {
    /// Build a new `Import` effect.
    pub fn new(branch: BranchReference<'a>, importer: I) -> Self {
        Self { branch, importer }
    }

    /// Execute the import: every artifact is committed as an
    /// assertion in one transaction, then subscriptions re-poll so
    /// SSE clients see the new state. Returns the post-commit
    /// revision.
    pub async fn perform<Env>(self, env: &Env) -> Result<Revision, ImportError>
    where
        Env: LoadProvider + BranchOpenProvider + CommitProvider + SelectProvider,
    {
        let cached = self.branch.acquire(env).await?;
        let revision = cached
            .handle()
            .import(self.importer)
            .perform(env)
            .await
            .map_err(ImportError::Commit)?;
        cached.poll(env).await;
        Ok(revision)
    }
}

/// Failure modes of an [`Import`].
#[derive(Debug)]
pub enum ImportError {
    /// The branch could not be resolved / opened.
    Reactor(ReactorError),
    /// The dialog import / commit failed.
    Commit(CommitError),
}

impl From<ReactorError> for ImportError {
    fn from(error: ReactorError) -> Self {
        ImportError::Reactor(error)
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Reactor(e) => write!(f, "{e}"),
            ImportError::Commit(e) => write!(f, "import failed: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}
