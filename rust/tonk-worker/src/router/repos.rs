//! List-repositories endpoint.
//!
//! Returns every [`RepoEntry`] the worker knows about, drawn from
//! [`RepoIndex`][crate::RepoIndex]. The sidebar consumes this to populate
//! its rows; the empty-state modal gates on an empty response.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::{RepoEntry, TonkWorkerError};

/// Response from `GET /api/repositories`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ListRepositoriesResponse {
    /// All repos the profile has access to, in insertion order.
    pub repositories: Vec<RepoEntry>,
}

/// List every repo the profile has access to.
#[wasm_compat]
pub async fn list_repositories(
    State(state): State<AppState>,
) -> Result<Json<ListRepositoriesResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;
    Ok(Json(ListRepositoriesResponse {
        repositories: tonk_state.repo_index.list(),
    }))
}
