//! Authorization routes using UCAN-based authentication.
//!
//! The authorization endpoint uses the operator and delegation that were
//! created when the service worker started. No external input is needed.

use ::axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_artifacts::replica::RemoteCredentials;
use dialog_s3_credentials::ucan::{Credentials as UcanCredentials, DelegationChain};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{RemoteState, SpaceError};
use ucan::delegation::subject::DelegatedSubject;

use super::AppState;
use crate::TonkWorkerError;
use crate::worker::TonkState;

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

/// Handles authorization requests and configures the UCAN-based remote for the space.
///
/// Uses the operator and delegation from the worker's state (created at startup).
/// No external input is required - just call POST /api/{multikey}/authorize with an empty body.
#[wasm_compat]
pub async fn authorize(
    State(state): State<AppState>,
    Path(multikey): Path<String>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    log!("Authorizing with internal UCAN delegation...");

    let space_did = TonkState::multikey_to_did(&multikey);
    let tonk_state = state.read().await;

    let mut session = tonk_state
        .session_for_space(&space_did)
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to open session: {}", e)))?;

    // Get user's delegation for authorization
    let user_delegations = session
        .account_delegations()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to query delegations: {}", e)))?;

    let service_url = get_access_service_url();
    log!(
        "Setting up UCAN credentials for space: {} with URL: {}",
        space_did,
        service_url
    );

    // Find delegation where subject matches the space DID
    let delegation = user_delegations
        .into_iter()
        .find(|d| match d.subject() {
            DelegatedSubject::Specific(did) => did.to_string() == space_did,
            DelegatedSubject::Any => true, // Powerline delegations apply to any subject
        })
        .ok_or_else(|| {
            TonkWorkerError::Internal(format!("No delegation found for space {}", space_did))
        })?;

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
    match session.space_mut().add_remote(remote_state).await {
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
    if !session.space().has_upstream().await {
        log!("Setting 'origin' as upstream for main branch...");
        match session.space_mut().set_upstream("origin").await {
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

    // Update the cached session
    tonk_state.update_session(session).await;

    Ok(Json(AuthorizeResponse {
        success: true,
        error: None,
    }))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::super::tests::test_state;
    use crate::{AuthorizeResponse, api_router};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[dialog_common::test]
    async fn it_authorizes_space() {
        let (state, multikey) = test_state().await;
        let app = api_router(state);

        let request = Request::builder()
            .uri(format!("/api/{}/authorize", multikey))
            .method("POST")
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

        let auth_response: AuthorizeResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(auth_response.success);
    }
}
