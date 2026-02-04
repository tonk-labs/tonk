//! Space metadata endpoints - get and update space name/description.

use axum::{Json, extract::{Path, State}};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use crate::router::AppState;
use crate::worker::TonkState;
use crate::TonkWorkerError;

/// Response for getting space metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceMetadataResponse {
    /// The space's DID.
    pub did: String,
    /// The human-readable name for the space, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The description of the space, if set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Request body for updating space metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateMetadataRequest {
    /// New name for the space. If provided, updates the name.
    #[serde(default)]
    pub name: Option<String>,
    /// New description for the space. If provided, updates the description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Handler for GET /api/{multikey}/metadata
///
/// Returns the current metadata (name, description) for a space.
#[wasm_compat]
pub async fn get_metadata(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<SpaceMetadataResponse>, TonkWorkerError> {
    let space_did = TonkState::multikey_to_did(&multikey);
    log!("Getting metadata for space: {}", space_did);

    let tonk_state = state.read().await;
    let session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    let name = session
        .space()
        .get_name()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to get name: {}", e)))?;

    let description = session
        .space()
        .get_description()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to get description: {}", e)))?;

    Ok(Json(SpaceMetadataResponse {
        did: space_did,
        name,
        description,
    }))
}

/// Handler for PUT /api/{multikey}/metadata
///
/// Updates the metadata (name, description) for a space.
/// Only provided fields are updated; omitted fields remain unchanged.
#[wasm_compat]
pub async fn update_metadata(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
    Json(request): Json<UpdateMetadataRequest>,
) -> Result<Json<SpaceMetadataResponse>, TonkWorkerError> {
    let space_did = TonkState::multikey_to_did(&multikey);
    log!("Updating metadata for space: {}", space_did);

    let tonk_state = state.read().await;
    let mut session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    // Update name if provided
    if let Some(ref name) = request.name {
        session
            .space_mut()
            .set_name(name)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("Failed to set name: {}", e)))?;
        log!("Updated space name to: {}", name);
    }

    // Update description if provided
    if let Some(ref description) = request.description {
        session
            .space_mut()
            .set_description(description)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("Failed to set description: {}", e)))?;
        log!("Updated space description to: {}", description);
    }

    // Get the current metadata (after updates)
    let name = session
        .space()
        .get_name()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to get name: {}", e)))?;

    let description = session
        .space()
        .get_description()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to get description: {}", e)))?;

    // Update the cached session
    tonk_state.update_session(session).await;

    Ok(Json(SpaceMetadataResponse {
        did: space_did,
        name,
        description,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::api_router;
    use crate::router::tests::test_state;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_gets_metadata_for_space() {
        let (state, multikey) = test_state().await;
        let app = api_router(state);

        let request = Request::builder()
            .uri(format!("/api/{}/metadata", multikey))
            .method("GET")
            .body(Body::empty())
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let metadata: SpaceMetadataResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(metadata.did.starts_with("did:key:z6Mk"));
    }

    #[dialog_common::test]
    async fn it_updates_metadata_for_space() {
        let (state, multikey) = test_state().await;
        let app = api_router(state.clone());

        let request_body = serde_json::json!({
            "name": "Updated Name",
            "description": "Updated description"
        });

        let request = Request::builder()
            .uri(format!("/api/{}/metadata", multikey))
            .method("PUT")
            .header("Content-Type", "application/json")
            .body(Body::from(request_body.to_string()))
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let metadata: SpaceMetadataResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert_eq!(metadata.name, Some("Updated Name".to_string()));
        assert_eq!(metadata.description, Some("Updated description".to_string()));
    }
}
