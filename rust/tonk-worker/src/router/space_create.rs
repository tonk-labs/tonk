//! Space creation endpoint - creates a new space with optional metadata.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use crate::TonkWorkerError;
use crate::router::AppState;

/// Request body for creating a new space.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateSpaceRequest {
    /// Optional human-readable name for the space.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional description of the space.
    #[serde(default)]
    pub description: Option<String>,
}

/// Response for space creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSpaceResponse {
    /// The DID of the newly created space (e.g., "did:key:z6Mk...")
    pub did: String,
    /// The name of the space, if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The description of the space, if provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Handler for POST /api/space/create
///
/// Creates a new space with optional name and description.
/// The space is automatically added to the user's known spaces.
#[wasm_compat]
pub async fn create_space(
    State(state): State<AppState>,
    Json(request): Json<CreateSpaceRequest>,
) -> Result<Json<CreateSpaceResponse>, TonkWorkerError> {
    log!("Creating new space with name: {:?}", request.name);

    let tonk_state = state.read().await;
    let mut identity = tonk_state.identity.write().await;

    // Create a new session (which creates a new space)
    let mut session = identity
        .create_session()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to create space: {}", e)))?;

    let space_did = session.space_did().to_string();
    log!("Created new space: {}", space_did);

    // Set metadata if provided
    if let Some(ref name) = request.name {
        session
            .space_mut()
            .set_name(name)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("Failed to set space name: {}", e)))?;
        log!("Set space name: {}", name);
    }

    if let Some(ref description) = request.description {
        session
            .space_mut()
            .set_description(description)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("Failed to set space description: {}", e))
            })?;
        log!("Set space description: {}", description);
    }

    // Cache the session for future use
    tonk_state.update_session(session).await;

    Ok(Json(CreateSpaceResponse {
        did: space_did,
        name: request.name,
        description: request.description,
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
    async fn it_creates_space_without_metadata() {
        let (state, _multikey) = test_state().await;
        let app = api_router(state);

        let request = Request::builder()
            .uri("/api/space/create")
            .method("POST")
            .header("Content-Type", "application/json")
            .body(Body::from("{}"))
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let create_response: CreateSpaceResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(create_response.did.starts_with("did:key:z6Mk"));
        assert!(create_response.name.is_none());
        assert!(create_response.description.is_none());
    }

    #[dialog_common::test]
    async fn it_creates_space_with_metadata() {
        let (state, _multikey) = test_state().await;
        let app = api_router(state);

        let request_body = serde_json::json!({
            "name": "Test Space",
            "description": "A test space"
        });

        let request = Request::builder()
            .uri("/api/space/create")
            .method("POST")
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

        let create_response: CreateSpaceResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(create_response.did.starts_with("did:key:z6Mk"));
        assert_eq!(create_response.name, Some("Test Space".to_string()));
        assert_eq!(
            create_response.description,
            Some("A test space".to_string())
        );
    }
}
