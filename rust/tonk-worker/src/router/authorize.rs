//! Authorization routes using UCAN-based authentication.
//!
//! The authorization endpoint uses the operator and delegation that were
//! created when the service worker started. No external input is needed.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_s3_credentials::{DelegationChain, OperatorIdentity, UcanAuthorizer};
use dialog_storage::s3::Bucket as S3Bucket;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{PlatformStorage, RemoteBackend};

use super::AppState;
use crate::TonkWorkerError;

/// Access service endpoint for UCAN-based authorization.
const ACCESS_SERVICE_URL: &str = "https://tonk-access-service.tonk.workers.dev";

/// Authorization response indicating success.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorizeResponse {
    /// Whether the authorization and remote setup succeeded.
    pub success: bool,
    /// Error message if authorization failed (for debugging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Status response indicating the current space state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    /// The DID of the space.
    pub space_did: String,
    /// The DID of the operator.
    pub operator_did: String,
    /// Whether the space has an upstream remote configured.
    pub has_upstream: bool,
}

/// Handles authorization requests and configures the UCAN-based remote for the space.
///
/// Uses the operator and delegation from the worker's state (created at startup).
/// No external input is required - just call POST /api/authorize with an empty body.
#[wasm_compat]
pub async fn authorize(
    State(state): State<AppState>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    log!("Authorizing with internal UCAN delegation...");

    let mut tonk_state = state.write().await;

    // Get the operator secret for creating UCAN invocations
    let operator_secret = tonk_state.operator.to_secret();
    let operator_identity = OperatorIdentity::from_secret(&operator_secret);

    // Serialize the delegation to create the proof chain
    let delegation_bytes = tonk_state.delegation.to_bytes();
    let delegation_cid = tonk_state.delegation.cid();

    // Create delegation chain with single proof
    let delegation_chain = DelegationChain::new(vec![delegation_bytes], vec![delegation_cid]);

    let space_did = tonk_state.space.did.clone();
    log!("Setting up UCAN authorizer for space: {}", space_did);

    // Build UCAN authorizer
    let authorizer = UcanAuthorizer::builder()
        .service_url(ACCESS_SERVICE_URL)
        .operator(operator_identity)
        .delegation(&space_did, delegation_chain)
        .build()
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to build authorizer: {}", e)))?;

    // Create S3 bucket with UCAN authorizer
    let bucket: S3Bucket<Vec<u8>, Vec<u8>> = S3Bucket::open(authorizer)
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open bucket: {}", e)))?;

    // Create platform storage from the bucket, scoped to the space DID
    let backend = dialog_artifacts::ErrorMappingBackend::new(bucket.at(&space_did));
    let remote_storage: PlatformStorage<RemoteBackend> =
        PlatformStorage::new(backend, dialog_artifacts::CborEncoder);

    // Set the upstream storage
    tonk_state
        .space
        .set_upstream_storage("ucan".to_string(), remote_storage)
        .await
        .map_err(|e| {
            log!("Failed to set upstream: {:?}", e);
            TonkWorkerError::Internal(format!("Failed to set upstream: {}", e))
        })?;

    log!("UCAN remote configured successfully");

    Ok(Json(AuthorizeResponse {
        success: true,
        error: None,
    }))
}

/// Returns the current status of the space.
#[wasm_compat]
pub async fn status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, TonkWorkerError> {
    let tonk_state = state.read().await;

    Ok(Json(StatusResponse {
        space_did: tonk_state.space.did.clone(),
        operator_did: tonk_state.operator.did().to_string(),
        has_upstream: tonk_state.space.has_upstream().await,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_space_with_delegation;
    use super::*;
    use crate::api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_returns_status_without_upstream() {
        let (space, operator, delegation) = test_space_with_delegation().await;
        let app = api_router(space, operator, delegation);

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
        assert!(!status_response.operator_did.is_empty());
    }
}
