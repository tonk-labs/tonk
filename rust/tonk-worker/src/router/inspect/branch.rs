//! Branch inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_repository::{RepositoryExt as _, Revision};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::super::AppState;
use crate::TonkWorkerError;

/// Path parameters for branch inspection.
#[derive(Debug, Deserialize)]
pub struct InspectBranchPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response for branch status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchStatusResponse {
    /// The subject DID (repository DID).
    pub subject: String,
    /// The branch name.
    pub branch: String,
    /// The branch's current revision, or `null` if it has no commits.
    pub revision: Option<Revision>,
    /// Upstream info if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<UpstreamInfo>,
}

/// Upstream status info.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UpstreamInfo {
    /// A local upstream (another branch in the same repo).
    Local {
        /// The upstream branch name.
        branch: String,
    },
    /// A remote upstream (a branch at a remote site).
    Remote {
        /// The remote name.
        remote: String,
        /// The branch name on the remote.
        branch: String,
    },
}

/// Returns the status of a specific branch.
#[wasm_compat]
pub async fn inspect_branch(
    State(state): State<AppState>,
    Path(params): Path<InspectBranchPath>,
) -> Result<Json<BranchStatusResponse>, TonkWorkerError> {
    log!(
        "Inspecting branch: repo={}, branch={}",
        params.repo,
        params.branch
    );
    let tonk_state = state.read().await;

    let repo = tonk_state
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::NotFound(format!("Repository '{}' not found: {}", params.repo, e))
        })?;

    let branch = repo
        .branch(params.branch.as_str())
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to open branch '{}': {}", params.branch, e))
        })?;

    let upstream = branch.upstream().map(|u| {
        use dialog_repository::Upstream;
        match u {
            Upstream::Local { branch, .. } => UpstreamInfo::Local {
                branch: branch.to_string(),
            },
            Upstream::Remote { remote, branch, .. } => UpstreamInfo::Remote {
                remote: remote.to_string(),
                branch: branch.to_string(),
            },
        }
    });

    Ok(Json(BranchStatusResponse {
        subject: repo.did().to_string(),
        branch: branch.name().to_string(),
        revision: branch.revision(),
        upstream,
    }))
}
