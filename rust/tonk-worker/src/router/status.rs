//! Status route for querying current space state.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;

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
