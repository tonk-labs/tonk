//! Remote inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_repository::RepositoryExt as _;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::super::AppState;
use crate::TonkWorkerError;

/// Path parameters for remote inspection.
#[derive(Debug, Deserialize)]
pub struct InspectRemotePath {
    /// The repository name.
    pub repo: String,
    /// The remote name.
    pub remote: String,
}

/// Path parameters for remote branch inspection.
#[derive(Debug, Deserialize)]
pub struct InspectRemoteBranchPath {
    /// The repository name.
    pub repo: String,
    /// The remote name.
    pub remote: String,
    /// The branch name.
    pub branch: String,
}

/// Response for remote status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteStatusResponse {
    /// The remote name.
    pub name: String,
    /// The subject DID of the remote repository.
    pub subject: String,
    /// Whether the remote exists and is configured.
    pub exists: bool,
}

/// Response for remote branch resolution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteBranchStatusResponse {
    /// The remote name.
    pub remote: String,
    /// The branch name.
    pub branch: String,
    /// Whether the resolution succeeded.
    pub success: bool,
    /// Whether the remote branch has been fetched.
    pub has_revision: bool,
    /// Error message if resolution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Returns the status of a specific remote.
#[wasm_compat]
pub async fn inspect_remote(
    State(state): State<AppState>,
    Path(params): Path<InspectRemotePath>,
) -> Result<Json<RemoteStatusResponse>, TonkWorkerError> {
    log!(
        "Inspecting remote: repo={}, remote={}",
        params.repo,
        params.remote
    );
    let tonk_state = state.read().await;

    let repo = tonk_state
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "Failed to load repository '{}': {}",
                params.repo, e
            ))
        })?;

    match repo
        .remote(params.remote.as_str())
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(remote_repo) => Ok(Json(RemoteStatusResponse {
            name: remote_repo.site().name().to_string(),
            subject: remote_repo.did().to_string(),
            exists: true,
        })),
        Err(_) => Ok(Json(RemoteStatusResponse {
            name: params.remote,
            subject: String::new(),
            exists: false,
        })),
    }
}

/// Returns the status of a specific remote branch.
#[wasm_compat]
pub async fn inspect_remote_branch(
    State(state): State<AppState>,
    Path(params): Path<InspectRemoteBranchPath>,
) -> Result<Json<RemoteBranchStatusResponse>, TonkWorkerError> {
    log!(
        "Inspecting remote branch: repo={}, remote={}, branch={}",
        params.repo,
        params.remote,
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
            TonkWorkerError::Internal(format!(
                "Failed to load repository '{}': {}",
                params.repo, e
            ))
        })?;

    let remote_repo = match repo
        .remote(params.remote.as_str())
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(RemoteBranchStatusResponse {
                remote: params.remote,
                branch: params.branch,
                success: false,
                has_revision: false,
                error: Some(format!("Remote not found: {}", e)),
            }));
        }
    };

    match remote_repo
        .branch(params.branch.as_str())
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(remote_branch) => Ok(Json(RemoteBranchStatusResponse {
            remote: params.remote,
            branch: remote_branch.name().to_string(),
            success: true,
            has_revision: remote_branch.revision().is_some(),
            error: None,
        })),
        Err(e) => Ok(Json(RemoteBranchStatusResponse {
            remote: params.remote,
            branch: params.branch,
            success: false,
            has_revision: false,
            error: Some(format!("{}", e)),
        })),
    }
}
