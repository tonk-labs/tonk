//! `tonk push` / `tonk pull` — fast-forward sync between
//! `main` and its configured upstream.
//!
//! Wraps dialog's `Branch::push()` / `Branch::pull()` with a
//! tonk-flavored error type that surfaces the
//! upstream-not-configured and non-fast-forward cases as
//! actionable messages. No subscription bookkeeping (tonk has
//! no SSE clients), so this skips the reactor wrapper the
//! worker uses.

use dialog_capability::AuthorizeError;
use dialog_repository::{FetchError, PullError, PushError, Revision, TreeReference};
use thiserror::Error;
use tonk_schema::{SyncState, classify};

use crate::ExitCode;
use crate::site::TonkSite;

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

/// The local branch's relationship to its upstream and current tree hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStatus {
    /// How the local and upstream heads compare.
    pub state: SyncState,
    /// The local branch's current tree root, when it has a revision.
    pub hash: Option<TreeReference>,
}

/// Failure modes for [`push`] / [`pull`].
#[derive(Debug, Error)]
pub enum SyncError {
    /// `branch.upstream()` returned `None` — the local branch
    /// has no upstream linkage. The caller should set one with
    /// `tonk remote set-upstream`.
    #[error(
        "branch '{branch}' has no upstream configured; \
         set one with `tonk remote set-upstream <name>`"
    )]
    UpstreamNotConfigured {
        /// Name of the local branch missing an upstream.
        branch: String,
    },
    /// Push refused because upstream advanced since the last
    /// sync. Caller should pull and retry.
    #[error(
        "non-fast-forward push: upstream has moved since last \
         sync; run `tonk pull` and try again"
    )]
    NonFastForward,
    /// The access service refused the authority this device presented.
    ///
    /// This is the only place enforcement is real: possession of a replica
    /// is not permission to sync it, and the service decides against the
    /// space's own delegation chain. Kept distinct from [`Self::Io`] so the
    /// caller can compose copy naming the fix — which differs by whether the
    /// space belongs to the signed-in account — while the reason the service
    /// actually gave stays on the error for anyone who just prints it.
    #[error("the access service rejected this device's authority: {reason}")]
    Rejected {
        /// The access decision, as the service stated it.
        reason: String,
    },
    /// Anything else — network, storage, decode. Surfaced
    /// verbatim so the caller can pick the underlying message
    /// up in stderr.
    #[error("{0}")]
    Io(String),
}

impl crate::Coded for SyncError {
    /// CLI exit code for this failure mode.
    fn exit_code(&self) -> ExitCode {
        match self {
            SyncError::UpstreamNotConfigured { .. }
            | SyncError::Io(_)
            | SyncError::Rejected { .. } => ExitCode::IoError,
            SyncError::NonFastForward => ExitCode::CommitError,
        }
    }
}

/// The access decision behind a failure, when there is one.
///
/// The reason travels as a typed `AuthorizeError` from the responder up
/// through dialog's layered errors, so this reads the type rather than
/// matching on rendered text — what a responder says is not a stable API.
///
/// Three of those variants are explicitly *not* decisions: the service could
/// not answer, or could not read what it was sent. Reporting them as denials
/// would tell someone their authority was refused when nothing was refused,
/// and stop them where a retry is the right move.
fn rejection(error: &(dyn std::error::Error + 'static)) -> Option<String> {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(reason) = authorization(error)
            && !matches!(
                reason,
                AuthorizeError::Unavailable { .. }
                    | AuthorizeError::UnavailableProof { .. }
                    | AuthorizeError::Malformed { .. }
            )
        {
            return Some(reason.to_string());
        }
        current = error.source();
    }
    None
}

/// The access decision one link of the chain carries, if it is one.
///
/// Each wrapper below holds its `AuthorizeError` in an `#[error(transparent)]`
/// variant, which forwards `source()` past itself — so the decision is never
/// reachable by walking sources alone, and every wrapper that can hold one
/// has to be named. A wrapper not listed here degrades to a plain I/O
/// failure, which is what an unrecognized failure honestly is.
fn authorization<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a AuthorizeError> {
    use dialog_effects::archive::ArchiveError;
    use dialog_effects::blob::BlobError;
    use dialog_effects::memory::MemoryError;
    use dialog_repository::{PublishError, ResolveError};
    use dialog_storage::DialogStorageError;

    if let Some(reason) = error.downcast_ref::<AuthorizeError>() {
        return Some(reason);
    }
    if let Some(ResolveError::Authorization(reason)) = error.downcast_ref::<ResolveError>() {
        return Some(reason);
    }
    if let Some(PublishError::Authorization(reason)) = error.downcast_ref::<PublishError>() {
        return Some(reason);
    }
    if let Some(ArchiveError::Authorization(reason)) = error.downcast_ref::<ArchiveError>() {
        return Some(reason);
    }
    if let Some(BlobError::Authorization(reason)) = error.downcast_ref::<BlobError>() {
        return Some(reason);
    }
    if let Some(MemoryError::Authorization(reason)) = error.downcast_ref::<MemoryError>() {
        return Some(reason);
    }
    if let Some(DialogStorageError::Authorization(reason)) =
        error.downcast_ref::<DialogStorageError>()
    {
        return Some(reason);
    }
    None
}

/// Push the site's main branch to its upstream.
///
/// `OK(outcome)` always means the operation completed; consult
/// `outcome.advanced` to learn whether anything actually went
/// over the wire.
pub async fn push(site: &TonkSite) -> Result<SyncOutcome, SyncError> {
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
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|error| SyncError::Io(format!("open meta branch: {error}")))?;
    if meta.upstream().is_some() {
        meta.push()
            .perform(&site.operator)
            .await
            .map_err(map_push_error)?;
    }
    Ok(SyncOutcome {
        before: before.clone(),
        after: before,
        advanced: upstream_after.is_some(),
    })
}

/// Pull from the site's upstream into the main branch.
pub async fn pull(site: &TonkSite) -> Result<SyncOutcome, SyncError> {
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
    let meta = site
        .repository
        .branch(crate::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await
        .map_err(|error| SyncError::Io(format!("open meta branch: {error}")))?;
    if meta.upstream().is_some() {
        meta.pull()
            .perform(&site.operator)
            .await
            .map_err(map_pull_error)?;
    }
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
/// [`SyncState::NoUpstream`] — not an error — so `tonk status`
/// always has something to print.
pub async fn status(site: &TonkSite) -> Result<SyncState, SyncError> {
    Ok(status_with_hash(site).await?.state)
}

/// Classify the site's sync state and retain its current local tree hash.
pub async fn status_with_hash(site: &TonkSite) -> Result<SyncStatus, SyncError> {
    let session = site
        .branch()
        .await
        .map_err(|e| SyncError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();
    let local = branch.revision();
    let hash = local.as_ref().map(|revision| revision.tree.clone());
    if branch.upstream().is_none() {
        return Ok(SyncStatus {
            state: SyncState::NoUpstream,
            hash,
        });
    }
    let remote = branch
        .fetch()
        .perform(&site.operator)
        .await
        .map_err(map_fetch_error)?;
    Ok(SyncStatus {
        state: classify(local.as_ref(), remote.as_ref()).into(),
        hash,
    })
}

/// What to tell someone whose sync the access service refused.
///
/// The CLI never pre-judges a sync — it relays the boundary's answer — but
/// the likeliest fix differs by state, so the copy is composed from the
/// roster the replica already holds. A device the roster already names, as
/// owner or as member, has had its authority refused rather than its identity
/// mistaken, so the message points at the device list. A device the roster
/// does not name, in a space that names some other founder, is most likely
/// signed into the wrong account, and the message leads with signing in.
///
/// `reason` is the service's own words, carried through verbatim on its own
/// line. The guidance either side of it is this CLI's inference from local
/// state and can be wrong; the reason is the only part of the message that
/// came from the boundary that actually said no, and it is what a bug report
/// needs.
pub async fn rejection_report(site: &TonkSite, name: &str, reason: &str) -> String {
    let said = format!("the access service said: {reason}");
    let roster = crate::inventory::read_roster(site).await.ok();
    let identity = crate::site::Identity::of(site).await.ok();
    let signed_in = identity
        .as_ref()
        .and_then(|identity| identity.account())
        .map(str::to_owned);
    // Signing in is only the fix for a device the space has never heard of.
    // A device that holds a roster row of its own — under any identity it
    // has, so a signed-out member still counts — is somebody this space
    // already knows, and what changed is its authority, not who is signed
    // in. Telling a member to go sign into the owner's account would send
    // them somewhere they cannot go.
    let listed = match (&roster, &identity) {
        (Some(roster), Some(identity)) => identity.dids().any(|did| roster.row_for(did).is_some()),
        _ => false,
    };
    let owner = roster.and_then(|roster| roster.founder().cloned());
    let Some(owner) = owner.filter(|_| !listed) else {
        return format!(
            "could not sync '{name}': the access service rejected this device's \
             authority\n{said}\nthis device may have been revoked; check \
             `tonk account devices`, or ask a member for a new invite and claim \
             it with `tonk join <URL>`"
        );
    };
    // Both roots appear in one sentence, so they are abbreviated against
    // each other: two identical-looking prefixes would leave the reader
    // unable to tell whose account is whose.
    let length = crate::inventory::abbreviation_length(
        std::iter::once(owner.did.as_str()).chain(signed_in.as_deref()),
    );
    let you = match &signed_in {
        Some(root) => format!(
            "you are signed in as {}",
            crate::inventory::describe(root, None, length)
        ),
        None => "you are not signed in".to_owned(),
    };
    format!(
        "could not sync '{name}': this device holds no authority its access \
         service accepts\n{said}\n'{name}' is owned by {owner}; {you}. sign \
         into the owning account with `tonk account login`, or ask a member \
         for an invite and claim it with `tonk join <URL>`",
        owner = crate::inventory::describe(&owner.did, owner.name.as_deref(), length),
    )
}

/// Read the local branch without touching the network.
///
/// `tonk context` is what bare `tonk` runs, and orientation should not
/// wait on a round trip to answer. Without a fetch there is no upstream
/// head to classify against — dialog caches none — so this reports the
/// one thing it can know locally: whether an upstream is configured at
/// all. The `not-fetched` state says so rather than implying `synced`.
pub async fn status_offline(site: &TonkSite) -> Result<crate::context::SyncContext, SyncError> {
    let session = site
        .branch()
        .await
        .map_err(|e| SyncError::Io(format!("acquire branch: {e}")))?;
    let branch = session.handle();
    let hash = branch.revision().map(|revision| revision.tree.to_string());
    let state = if branch.upstream().is_none() {
        "no-upstream"
    } else {
        "not-fetched"
    };
    Ok(crate::context::SyncContext {
        state: state.to_string(),
        hash,
        fetched: false,
    })
}

fn map_fetch_error(error: FetchError) -> SyncError {
    match error {
        FetchError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        other => classify_failure(&other),
    }
}

fn map_push_error(error: PushError) -> SyncError {
    match error {
        PushError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        PushError::NonFastForward { .. } => SyncError::NonFastForward,
        other => classify_failure(&other),
    }
}

fn map_pull_error(error: PullError) -> SyncError {
    match error {
        PullError::BranchHasNoUpstream { branch } => SyncError::UpstreamNotConfigured { branch },
        other => classify_failure(&other),
    }
}

fn classify_failure(error: &(dyn std::error::Error + 'static)) -> SyncError {
    match rejection(error) {
        Some(reason) => SyncError::Rejected { reason },
        None => SyncError::Io(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_repository::{PublishError, ResolveError};

    mod classifying_a_failure {
        use super::*;

        /// The decision is buried under two layers that render it
        /// transparently, which is where a source-walk alone loses it.
        #[dialog_common::test]
        fn it_reads_a_decision_out_of_the_wrappers_that_hide_it() {
            let denial = AuthorizeError::PolicyViolation {
                predicate: "subject is provisioned".to_owned(),
            };
            let error = PushError::FetchRemoteBranch(ResolveError::from(denial).into());

            let SyncError::Rejected { reason } = map_push_error(error) else {
                panic!("an access decision must not read as transport failure");
            };
            assert!(reason.contains("subject is provisioned"), "{reason}");
        }

        /// "We could not answer" is not "no". Reporting it as a denial would
        /// stop someone whose next move is simply to try again.
        #[dialog_common::test]
        fn it_leaves_a_service_that_could_not_answer_as_a_plain_failure() {
            for undecided in [
                AuthorizeError::Unavailable {
                    detail: "key store unreachable".to_owned(),
                },
                AuthorizeError::UnavailableProof {
                    link: "bafy".to_owned(),
                },
                AuthorizeError::Malformed {
                    detail: "not cbor".to_owned(),
                },
            ] {
                let error = PushError::Publish(PublishError::from(undecided));

                assert!(
                    matches!(map_push_error(error), SyncError::Io(_)),
                    "an unanswered request must not read as a denial"
                );
            }
        }
    }
}
