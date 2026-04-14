//! Status route for querying current repository state.

use ::axum::{Json, extract::Path, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_repository::RepositoryExt as _;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;

/// Status response indicating the current repository state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// The repository name.
    pub repo_name: String,
    /// The DID of the repository (space).
    pub space_did: String,
    /// The DID of the operator (profile).
    pub operator_did: String,
    /// Whether the default branch has an upstream remote configured.
    pub has_upstream: bool,
}

/// Returns the current status of a repository.
///
/// Loads the repository by name, opens its "main" branch, and reports
/// whether an upstream remote is configured.
#[wasm_compat]
pub async fn status(
    State(state): State<AppState>,
    Path(repo_name): Path<String>,
) -> Result<Json<StatusResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;

    let repo = tonk_state
        .profile
        .repository(&repo_name)
        .load()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("Failed to load repository '{}': {}", repo_name, e))
        })?;

    // Try to load the main branch to check upstream status
    let has_upstream = match repo
        .branch("main")
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(branch) => branch.upstream().is_some(),
        Err(_) => false,
    };

    Ok(Json(StatusResponse {
        repo_name,
        space_did: repo.did().to_string(),
        operator_did: tonk_state.profile.did().to_string(),
        has_upstream,
    }))
}
