//! Identity endpoint for retrieving the user's DID.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;

/// Response containing the user's DID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentifyResponse {
    /// The user's decentralized identifier (DID).
    pub user_did: String,
}

/// Returns the current user's DID.
///
/// This endpoint allows the UI to retrieve the user's persistent identity.
/// The DID is generated on first use and persists across sessions.
#[wasm_compat]
pub async fn identify(
    State(state): State<AppState>,
) -> Result<Json<IdentifyResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;

    Ok(Json(IdentifyResponse {
        user_did: tonk_state.identity.did().to_string(),
    }))
}
