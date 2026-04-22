//! `GET /api/repositories`: list local names of every repo registered
//! in the profile's `home` meta-index.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use super::home;
use crate::TonkWorkerError;

/// Response body for `GET /api/repositories`.
///
/// Only local names are returned — the UI renders rows against these
/// and fetches `GET /api/repository/{name}` per row when it needs
/// metadata. Keeping the list shallow avoids N probes per sidebar open.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListRepositoriesResponse {
    /// Local names of repos this profile has created or claimed.
    pub repositories: Vec<String>,
}

/// List every repo registered in home.
#[wasm_compat]
pub async fn list_repositories(
    State(state): State<AppState>,
) -> Result<Json<ListRepositoriesResponse>, TonkWorkerError> {
    log!("GET /api/repositories");

    let tonk = state.read().await;
    let repositories = home::list_registered(&tonk).await?;
    Ok(Json(ListRepositoriesResponse { repositories }))
}
