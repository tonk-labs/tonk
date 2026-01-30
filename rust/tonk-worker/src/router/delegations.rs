//! Delegations endpoint for retrieving user's UCAN delegations.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;

/// Response containing the user's delegations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DelegationsResponse {
    /// Base64-encoded DAG-CBOR delegation blobs.
    ///
    /// Each delegation is serialized as DAG-CBOR and then base64-encoded.
    /// These can be used directly with UcanAuthorizer for authorization.
    pub delegations: Vec<String>,
}

/// Returns all delegations granted to the current user for the current space.
///
/// This endpoint queries the space for UCAN delegations where the audience
/// matches the current user's DID. The delegations are returned as base64-encoded
/// DAG-CBOR blobs that can be used for authorization.
#[wasm_compat]
pub async fn delegations(
    State(state): State<AppState>,
) -> Result<Json<DelegationsResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;

    // Query delegations where audience == user DID
    let user_delegations = tonk_state
        .session
        .account_delegations()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to query delegations: {}", e)))?;

    // Encode each delegation as base64 DAG-CBOR
    let encoded: Vec<String> = user_delegations
        .iter()
        .map(|d| STANDARD.encode(d.to_bytes()))
        .collect();

    Ok(Json(DelegationsResponse {
        delegations: encoded,
    }))
}
