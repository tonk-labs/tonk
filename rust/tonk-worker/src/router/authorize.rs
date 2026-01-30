//! Authorization routes using UCAN-based authentication.
//!
//! The authorization endpoint uses the operator and delegation that were
//! created when the service worker started. No external input is needed.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::replica::RemoteCredentials;
use dialog_s3_credentials::ucan::{Credentials as UcanCredentials, DelegationChain};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{RemoteState, SpaceError};

use super::AppState;
use crate::TonkWorkerError;

/// Access service path (will be resolved to absolute URL at runtime).
const ACCESS_SERVICE_PATH: &str = "/ucan/";

/// Get the absolute URL for the access service.
///
/// In WASM (service worker), we resolve against the current origin.
/// In native, we return the path as-is (for testing).
fn get_access_service_url() -> String {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        use wasm_bindgen::JsCast;
        use web_sys::ServiceWorkerGlobalScope;

        let global = js_sys::global()
            .dyn_into::<ServiceWorkerGlobalScope>()
            .expect("Expected ServiceWorkerGlobalScope");

        let origin = global.location().origin();
        format!("{}{}", origin, ACCESS_SERVICE_PATH)
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        ACCESS_SERVICE_PATH.to_string()
    }
}

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
#[allow(dead_code)] // Used in wasm builds via #[wasm_compat] macro
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

    // Get user's delegation for authorization
    let user_delegations = tonk_state
        .session
        .account_delegations()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to query delegations: {}", e)))?;

    let delegation = user_delegations
        .into_iter()
        .next()
        .ok_or_else(|| TonkWorkerError::Internal("No delegations found for user".to_string()))?;

    let space_did = tonk_state.session.space_did().to_string();
    let service_url = get_access_service_url();
    log!(
        "Setting up UCAN credentials for space: {} with URL: {}",
        space_did,
        service_url
    );

    // Create delegation chain from the user's delegation
    let delegation_chain = DelegationChain::new(delegation.inner().clone());

    // Create UCAN credentials with the resolved access service URL
    let ucan_credentials = UcanCredentials::new(service_url, delegation_chain);

    let remote_state = RemoteState {
        site: "origin".into(),
        credentials: RemoteCredentials::Ucan(ucan_credentials),
    };

    // Add the remote to the space
    // If the remote already exists, that's fine - treat it as success
    match tonk_state
        .session
        .space_mut()
        .add_remote(remote_state)
        .await
    {
        Ok(site) => {
            log!("Remote '{}' added successfully", site);
        }
        Err(SpaceError::Replica(ref e)) if format!("{:?}", e).contains("RemoteAlreadyExists") => {
            log!("Remote 'origin' already configured, skipping add");
        }
        Err(e) => {
            log!("Failed to add remote: {:?}", e);
            return Err(TonkWorkerError::Internal(format!(
                "Failed to add remote: {}",
                e
            )));
        }
    }

    // Set upstream on branch if not already configured
    if !tonk_state.session.space().has_upstream().await {
        log!("Setting 'origin' as upstream for main branch...");
        match tonk_state.session.space_mut().set_upstream("origin").await {
            Ok(()) => {
                log!("Upstream set successfully");
            }
            Err(e) => {
                log!("Failed to set upstream: {:?}", e);
                return Err(TonkWorkerError::Internal(format!(
                    "Failed to set upstream: {}",
                    e
                )));
            }
        }
    } else {
        log!("Upstream already configured, skipping");
    }

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
    let space = tonk_state.session.space();

    Ok(Json(StatusResponse {
        space_did: space.did.clone(),
        operator_did: tonk_state.identity.operator().did().to_string(),
        has_upstream: space.has_upstream().await,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_state;
    use crate::StatusResponse;
    use crate::{AuthorizeResponse, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_returns_status_without_upstream() {
        let state = test_state().await;
        let app = api_router(state);

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
