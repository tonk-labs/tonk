//! Space list endpoint - returns all known spaces for this identity.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use crate::TonkWorkerError;
use crate::router::AppState;

/// Information about a single space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceInfo {
    /// The space's DID (e.g., "did:key:z6Mk...")
    pub did: String,
    /// Optional human-readable name for the space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Optional description of the space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Response for the space list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSpacesResponse {
    /// List of spaces the user has access to, sorted alphabetically by DID.
    pub spaces: Vec<SpaceInfo>,
}

/// Handler for GET /api/space/list
///
/// Returns all spaces known to this identity, sorted alphabetically by DID.
/// Includes name and description metadata for each space.
#[wasm_compat]
pub async fn list_spaces(
    State(state): State<AppState>,
) -> Result<Json<ListSpacesResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;
    let identity = tonk_state.identity.read().await;

    // Get known spaces from the account
    let known_space_dids: Vec<String> = identity.account().known_spaces().await.unwrap_or_default();

    // Drop identity lock before fetching metadata (we need tonk_state for session_for_space)
    drop(identity);

    // Fetch metadata for each space
    let mut spaces: Vec<SpaceInfo> = Vec::with_capacity(known_space_dids.len());

    for did in known_space_dids {
        let (name, description) = match tonk_state.session_for_space(&did).await {
            Ok(session) => {
                let name = session.space().get_name().await.ok().flatten();
                let description = session.space().get_description().await.ok().flatten();
                (name, description)
            }
            Err(e) => {
                // Log error but continue - return space without metadata
                log!("Failed to fetch metadata for space {}: {:?}", did, e);
                (None, None)
            }
        };

        spaces.push(SpaceInfo {
            did,
            name,
            description,
        });
    }

    // Sort alphabetically by DID
    spaces.sort_by(|a, b| a.did.cmp(&b.did));

    Ok(Json(ListSpacesResponse { spaces }))
}
