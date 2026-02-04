//! Delegations endpoint for retrieving user's UCAN delegations.

use ::axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Response containing the user's delegations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DelegationsResponse {
    /// Base64-encoded DAG-CBOR delegation blobs.
    ///
    /// Each delegation is serialized as DAG-CBOR and then base64-encoded.
    /// These can be used directly with UcanAuthorizer for authorization.
    pub delegations: Vec<String>,
}

/// Returns all delegations granted to the current user for the given space.
///
/// This endpoint queries the space for UCAN delegations where the audience
/// matches the current user's DID. The delegations are returned as base64-encoded
/// DAG-CBOR blobs that can be used for authorization.
#[wasm_compat]
pub async fn delegations(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<DelegationsResponse>, TonkWorkerError> {
    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    // Query delegations where audience == user DID
    let user_delegations = session
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
