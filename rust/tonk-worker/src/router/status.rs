//! Status route for querying current space state.

use ::axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

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
    Path(multikey): Path<String>,
) -> Result<Json<StatusResponse>, TonkWorkerError> {
    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    let space = session.space();
    let identity = tonk_state.identity.read().await;

    Ok(Json(StatusResponse {
        space_did: space.did.clone(),
        operator_did: identity.did().to_string(),
        has_upstream: space.has_upstream().await,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_state;
    use super::*;
    use crate::api_router;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_returns_status_without_upstream() {
        let (tonk_state, multikey) = test_state().await;
        let app = api_router(tonk_state);

        let request = Request::builder()
            .uri(format!("/api/{}/status", multikey))
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
