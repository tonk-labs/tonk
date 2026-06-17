//! `slide push` / `slide pull` — fast-forward sync between
//! `main` and its configured upstream.
//!
//! Wraps dialog's `Branch::push()` / `Branch::pull()` with a
//! slide-flavored error type that surfaces the
//! upstream-not-configured and non-fast-forward cases as
//! actionable messages. No subscription bookkeeping (slide has
//! no SSE clients), so this skips the reactor wrapper the
//! worker uses.

use dialog_repository::{FetchError, PullError, PushError, Revision};
use thiserror::Error;
use tonk_schema::{SyncState, classify};

use crate::ExitCode;
use crate::site::SlideSite;

/// Result of a successful push or pull.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// Local branch revision before the operation.
    pub before: Option<Revision>,
    /// Local branch revision after the operation. Equals
    /// [`Self::before`] for push (push doesn't move local).
    /// May advance for pull.
    pub after: Option<Revision>,
    /// True when the operation moved the *relevant* side
    /// forward — the upstream for push, the local branch for
    /// pull. Lets the caller print "pushed" / "nothing to push"
    /// without comparing revisions itself.
    pub advanced: bool,
}

/// Failure modes for [`push`] / [`pull`].
#[derive(Debug, Error)]
pub enum SyncError {
    /// `branch.upstream()` returned `None` — the local branch
    /// has no upstream linkage. The caller should set one with
    /// `slide remote set-upstream`.
    #[error(
        "branch '{branch}' has no upstream configured; \
         set one with `slide remote set-upstream <name>`"
    )]
    UpstreamNotConfigured {
        /// Name of the local branch missing an upstream.
        branch: String,
    },
    /// Push refused because upstream advanced since the last
    /// sync. Caller should pull and retry.
    #[error(
        "non-fast-forward push: upstream has moved since last \
         sync; run `slide pull` and try again"
    )]
    NonFastForward,
    /// Anything else — network, auth, storage, decode. Surfaced
    /// verbatim so the caller can pick the underlying message
    /// up in stderr.
    #[error("{0}")]
    Io(String),
}

impl SyncError {
    /// CLI exit code for this failure mode.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            SyncError::UpstreamNotConfigured { .. } | SyncError::Io(_) => ExitCode::IoError,
            SyncError::NonFastForward => ExitCode::CommitError,
        }
    }
}

/// Push the site's main branch to its upstream.
///
/// `OK(outcome)` always means the operation completed; consult
/// `outcome.advanced` to learn whether anything actually went
/// over the wire.
pub async fn push(site: &SlideSite) -> Result<SyncOutcome, SyncError> {
    let session = site
        .branch()
        .await
        .map_err(|e| SyncError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();
    let before = branch.revision();
    let upstream_after = branch
        .push()
        .perform(&site.operator)
        .await
        .map_err(map_push_error)?;
    Ok(SyncOutcome {
        before: before.clone(),
        after: before,
        advanced: upstream_after.is_some(),
    })
}

/// Pull from the site's upstream into the main branch.
pub async fn pull(site: &SlideSite) -> Result<SyncOutcome, SyncError> {
    let session = site
        .branch()
        .await
        .map_err(|e| SyncError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();
    let before = branch.revision();
    let merged = branch
        .pull()
        .perform(&site.operator)
        .await
        .map_err(map_pull_error)?;
    let after = branch.revision();
    Ok(SyncOutcome {
        before,
        after,
        // dialog returns `Some` when it merged, `None` when the
        // upstream had nothing new — that's our "advanced"
        // signal, more reliable than revision comparison.
        advanced: merged.is_some(),
    })
}

/// Classify the site's main branch against its upstream without
/// mutating local state.
///
/// Reads the local head, fetches the upstream head read-only, and
/// runs the shared classifier. A branch with no upstream is
/// [`SyncState::NoUpstream`] — not an error — so `slide status`
/// always has something to print.
pub async fn status(site: &SlideSite) -> Result<SyncState, SyncError> {
    let session = site
        .branch()
        .await
        .map_err(|e| SyncError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();
    let local = branch.revision();
    if branch.upstream().is_none() {
        return Ok(SyncState::NoUpstream);
    }
    let remote = branch
        .fetch()
        .perform(&site.operator)
        .await
        .map_err(map_fetch_error)?;
    Ok(classify(local.as_ref(), remote.as_ref()).into())
}

fn map_fetch_error(error: FetchError) -> SyncError {
    match error {
        FetchError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        other => SyncError::Io(other.to_string()),
    }
}

fn map_push_error(error: PushError) -> SyncError {
    match error {
        PushError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        PushError::NonFastForward { .. } => SyncError::NonFastForward,
        other => SyncError::Io(other.to_string()),
    }
}

fn map_pull_error(error: PullError) -> SyncError {
    match error {
        PullError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        other => SyncError::Io(other.to_string()),
    }
}
