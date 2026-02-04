//! Space list endpoint - returns all known spaces for this identity.

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::router::AppState;

/// Information about a single space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceInfo {
    /// The space's DID (e.g., "did:key:z6Mk...")
    pub did: String,
    /// Optional human-readable name for the space.
    pub name: Option<String>,
    /// Optional description of the space.
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
pub async fn list_spaces(State(state): State<AppState>) -> Json<ListSpacesResponse> {
    let state = state.read().await;

    // Get known spaces from the account
    let known_space_dids = state
        .identity
        .account()
        .known_spaces()
        .await
        .unwrap_or_default();

    // Convert to SpaceInfo and sort alphabetically
    let mut spaces: Vec<SpaceInfo> = known_space_dids
        .into_iter()
        .map(|did| SpaceInfo {
            did,
            // TODO: In the future, fetch name/description from space metadata
            name: None,
            description: None,
        })
        .collect();

    spaces.sort_by(|a, b| a.did.cmp(&b.did));

    Json(ListSpacesResponse { spaces })
}
