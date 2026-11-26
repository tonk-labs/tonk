//! Restore by creating a branch at a saved revision and switching to it.

use dialog_effects::archive::prelude::{ArchiveScope, GetBlockExt as _};
use dialog_repository::Revision;

use crate::{Coded, ExitCode, branches, revisions, site::TonkSite};

/// A checkout planned against an exact active branch and head.
#[derive(Debug)]
pub struct Plan {
    /// Branch retained with its existing data and history.
    pub previous_branch: String,
    /// New branch name (or the current one for a no-op).
    pub branch: String,
    /// Exact signed snapshot to check out.
    pub target: Revision,
    /// Active head when this checkout was planned.
    pub before: Revision,
}

impl Plan {
    /// Whether executing this plan changes the selected branch.
    pub fn switches(&self) -> bool {
        self.branch != self.previous_branch
    }
}

/// Why a historical checkout could not complete.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    /// Malformed version/hash.
    #[error("invalid revision '{0}': pass <edition>@<origin> from `tonk log`, or a tree hash")]
    Invalid(String),
    /// The history ID alone is insufficient to open a snapshot.
    #[error(
        "no saved snapshot for revision {0} on this device; this CLI retains signed heads as it reads and writes them, but older log entries alone do not contain the tree root needed for checkout"
    )]
    Unavailable(String),
    /// Another write or switch invalidated the plan.
    #[error("the active branch changed while planning the restore; run the command again")]
    Changed,
    /// Storage, snapshot validation, or branch operation failed.
    #[error("{0}")]
    Io(String),
}

impl Coded for RestoreError {
    fn exit_code(&self) -> ExitCode {
        match self {
            Self::Invalid(_) | Self::Unavailable(_) => ExitCode::ParseError,
            Self::Changed => ExitCode::CommitError,
            Self::Io(_) => ExitCode::IoError,
        }
    }
}

fn io(error: impl std::fmt::Display) -> RestoreError {
    RestoreError::Io(error.to_string())
}

/// Resolve the saved snapshot and verify its root without creating a branch.
pub async fn plan(site: &TonkSite, reference: &str) -> Result<Plan, RestoreError> {
    let key = revisions::key(reference).map_err(|_| RestoreError::Invalid(reference.into()))?;
    let session = site.branch().await.map_err(io)?;
    let source = session.handle();
    let before = source
        .revision()
        .ok_or_else(|| RestoreError::Unavailable(reference.into()))?;
    let target = revisions::resolve(site, &key)
        .await
        .map_err(io)?
        .ok_or_else(|| RestoreError::Unavailable(reference.into()))?;
    let root = ArchiveScope::new(site.repository.subject())
        .index()
        .read()
        .get(dialog_common::Blake3Hash::from(*target.tree.hash()))
        .perform(&site.operator)
        .await
        .map_err(io)?;
    if root.is_none() {
        return Err(RestoreError::Io(format!(
            "the saved revision's tree {} is not available locally",
            target.tree
        )));
    }
    let previous_branch = source.name().to_owned();
    let branch = if target == before {
        previous_branch.clone()
    } else {
        format!(
            "restore-{}-{:016x}",
            target.edition.value(),
            rand::random::<u64>()
        )
    };
    Ok(Plan {
        previous_branch,
        branch,
        target,
        before,
    })
}

/// Create the new branch, then switch the replica's active reference to it.
/// The old branch and its upstream remain intact; the new branch has no upstream.
pub async fn apply(site: &TonkSite, plan: &Plan) -> Result<(), RestoreError> {
    let session = site.branch().await.map_err(io)?;
    let source = session.handle();
    source.refresh(&site.operator).await.map_err(io)?;
    if source.name() != plan.previous_branch || source.revision().as_ref() != Some(&plan.before) {
        return Err(RestoreError::Changed);
    }
    if !plan.switches() {
        return Ok(());
    }
    branches::register(site, &plan.previous_branch)
        .await
        .map_err(io)?;
    branches::create(site, &plan.branch, plan.target.clone())
        .await
        .map_err(io)?;
    branches::switch(site, &plan.branch).await.map_err(|error| {
        io(format!(
            "created branch '{}', but could not select it: {error}; retry with `tonk switch {}`",
            plan.branch, plan.branch
        ))
    })
}
