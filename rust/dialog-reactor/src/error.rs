use dialog_query::EvaluationError;
use dialog_repository::{CommitError, PullError, PushError, StackError};
use thiserror::Error;

/// Errors surfaced by the reactor's chain effects. Consumers map
/// these into their own error envelope (e.g. the worker's
/// `TonkWorkerError`).
#[derive(Debug, Error)]
#[allow(missing_docs)]
pub enum ReactorError {
    /// This reactor has terminally shut down and cannot open another
    /// subscription.
    #[error("reactor is shut down")]
    Shutdown,
    /// The named repository couldn't be loaded from the profile.
    #[error("repository {repo:?} not found: {reason}")]
    RepositoryNotFound { repo: String, reason: String },
    /// The named branch couldn't be opened on the repository.
    #[error("branch {branch:?} on repository {repo:?} not found: {reason}")]
    BranchNotFound {
        repo: String,
        branch: String,
        reason: String,
    },
    /// A query against the branch failed.
    #[error("query failed: {0:?}")]
    QueryFailed(#[from] EvaluationError),
    /// A commit against the branch failed.
    #[error("commit failed: {0}")]
    Commit(#[from] CommitError),
    /// A stack operation (open, advance, or a stack commit) failed.
    /// Boxed: the stack error carries a head and an address, and would
    /// otherwise make every reactor result large.
    #[error("stack failed: {0}")]
    Stack(Box<StackError>),
    /// A pull from upstream failed.
    #[error("pull failed: {0}")]
    Pull(#[from] PullError),
    /// Materializing a pulled revision locally failed.
    #[error("download failed: {0}")]
    Download(#[from] dialog_repository::DownloadError),
    /// A push to upstream failed.
    #[error("push failed: {0}")]
    Push(#[from] PushError),
}

impl From<StackError> for ReactorError {
    fn from(error: StackError) -> Self {
        ReactorError::Stack(Box::new(error))
    }
}
