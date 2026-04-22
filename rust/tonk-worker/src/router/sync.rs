//! Sync routes for push/pull operations with upstream.
//!
//! These endpoints allow synchronizing a branch with its upstream remote.

use ::axum::{Json, extract::Path, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_repository::RepositoryExt as _;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Path parameters for sync endpoints.
#[derive(Debug, Deserialize)]
pub struct SyncPath {
    /// The repository name.
    pub repo: String,
    /// The branch name.
    pub branch: String,
}

/// Response for sync operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Whether the sync operation succeeded.
    pub success: bool,
    /// Whether any changes were synced.
    pub changed: bool,
    /// Error message if sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pull changes from the upstream remote.
///
/// Fetches changes from the remote and merges them into the local branch.
#[wasm_compat]
pub async fn pull(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pulling from upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

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

    match branch.pull().perform(&tonk_state.operator).await {
        Ok(_) => {
            log!("Pull succeeded");
            Ok(Json(SyncResponse {
                success: true,
                changed: true,
                error: None,
            }))
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Push local changes to the upstream remote.
///
/// Sends local changes to the remote.
#[wasm_compat]
pub async fn push(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Pushing to upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

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

    match branch.push().perform(&tonk_state.operator).await {
        Ok(_) => {
            log!("Push succeeded");
            Ok(Json(SyncResponse {
                success: true,
                changed: true,
                error: None,
            }))
        }
        Err(e) => {
            log!("Push failed: {:?}", e);
            Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Full sync: pull then push.
///
/// First pulls changes from upstream, then pushes local changes.
#[wasm_compat]
pub async fn sync(
    State(state): State<AppState>,
    Path(params): Path<SyncPath>,
) -> Result<Json<SyncResponse>, TonkWorkerError> {
    log!(
        "Syncing with upstream: repo={}, branch={}",
        params.repo,
        params.branch
    );

    let tonk_state = state.write().await;

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

    // First pull
    let pull_changed = match branch.pull().perform(&tonk_state.operator).await {
        Ok(_) => {
            log!("Pull succeeded");
            true
        }
        Err(e) => {
            log!("Pull failed: {:?}", e);
            return Ok(Json(SyncResponse {
                success: false,
                changed: false,
                error: Some(format!("Pull failed: {}", e)),
            }));
        }
    };

    // Then push
    let push_changed = match branch.push().perform(&tonk_state.operator).await {
        Ok(_) => {
            log!("Push succeeded");
            true
        }
        Err(e) => {
            log!("Push failed: {:?}", e);
            return Ok(Json(SyncResponse {
                success: false,
                changed: pull_changed,
                error: Some(format!("Push failed: {}", e)),
            }));
        }
    };

    Ok(Json(SyncResponse {
        success: true,
        changed: pull_changed || push_changed,
        error: None,
    }))
}
