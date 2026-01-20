use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{RemoteConfig, RemoteState};

use super::AppState;
use crate::TonkWorkerError;

/// R2 endpoint for Tonk spaces storage.
const R2_ENDPOINT: &str = "https://5f20ca8a0de0a5ac52a14fa8bf9c90db.r2.cloudflarestorage.com";
/// R2 bucket name for Tonk spaces.
const R2_BUCKET: &str = "tonk-spaces";

/// Authorization request with account credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    /// AWS access key ID.
    pub access_key_id: String,
    /// AWS secret access key.
    pub secret_access_key: String,
}

/// Authorization response indicating success.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    /// Whether the authorization and remote setup succeeded.
    pub success: bool,
}

/// Status response indicating the current space state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// The DID of the space.
    pub space_did: String,
    /// Whether the space has an upstream remote configured.
    pub has_upstream: bool,
}

/// Handles authorization requests and configures the R2 remote for the space.
#[wasm_compat]
pub async fn authorize(
    State(state): State<AppState>,
    Json(body): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    log!("Authorizing and configuring R2 remote...");

    let mut space = state.write().await;

    // Create remote config with R2 credentials
    // Prefix is space DID followed by /
    let prefix = format!("{}/", space.did);

    let remote_config = RemoteConfig {
        endpoint: R2_ENDPOINT.to_string(),
        region: "auto".to_string(),
        bucket: R2_BUCKET.to_string(),
        prefix: Some(prefix),
        access_key_id: Some(body.access_key_id),
        secret_access_key: Some(body.secret_access_key),
    };

    let remote_state = RemoteState {
        site: "r2".to_string(),
        address: remote_config,
    };

    // Add the remote to the space
    space.add_remote(remote_state).await.map_err(|e| {
        log!("Failed to add remote: {:?}", e);
        TonkWorkerError::Internal(format!("Failed to add remote: {}", e))
    })?;

    log!("R2 remote configured successfully");

    Ok(Json(AuthorizeResponse { success: true }))
}

/// Returns the current status of the space.
#[wasm_compat]
pub async fn status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, TonkWorkerError> {
    let space = state.read().await;

    Ok(Json(StatusResponse {
        space_did: space.did.clone(),
        has_upstream: space.has_upstream().await,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_space;
    use crate::StatusResponse;
    use crate::{AuthorizeRequest, AuthorizeResponse, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_authorizes_and_returns_presigned_url() {
        let artifacts = test_space().await;
        let app = api_router(artifacts);

        let auth_request = AuthorizeRequest {
            access_key_id: "test-account".to_string(),
            secret_access_key: "test-secret".to_string(),
        };

        let request = Request::builder()
            .uri("/api/authorize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&auth_request).expect("Failed to serialize request"),
            ))
            .expect("Failed to build request");

        let response = app
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let auth_response: AuthorizeResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(auth_response.success);
    }

    #[dialog_common::test]
    async fn it_returns_status_without_upstream() {
        let space = test_space().await;
        let app = api_router(space);

        let request = Request::builder()
            .uri("/api/status")
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

        let status_response: StatusResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(!status_response.has_upstream);
        assert!(!status_response.space_did.is_empty());
    }
}
