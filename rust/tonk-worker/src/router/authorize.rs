//! Authorization routes using UCAN-based authentication.
//!
//! The authorization endpoint uses the operator and delegation that were
//! created when the service worker started. An optional access service URL
//! can be provided for testing purposes.

use ::axum::{Json, extract::State};
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

/// Access service path (will be resolved to absolute URL at runtime).
const ACCESS_SERVICE_PATH: &str = "/ucan/";

/// Get the default absolute URL for the access service.
///
/// In WASM (service worker), we resolve against the current origin.
/// In native, we return the default path.
fn get_default_access_service_url() -> String {
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

/// Request body for authorization.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthorizeRequest {
    /// Optional access service URL override.
    /// If not provided, defaults to the origin-relative `/ucan/` path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_service_url: Option<String>,
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
/// Optionally accepts an `access_service_url` in the request body for testing.
#[wasm_compat]
pub async fn authorize(
    State(state): State<AppState>,
    body: Option<Json<AuthorizeRequest>>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    log!("Authorizing with internal UCAN delegation...");

    let mut tonk_state = state.write().await;

    // Get user's delegation for authorization
    let user_delegations = tonk_state
        .session
        .account_delegations()
        .await
        .map_err(|e| TonkWorkerError::Internal(format!("Failed to query delegations: {}", e)))?;

    let space_did = tonk_state.session.space_did().to_string();

    // Use provided URL or fall back to default
    let service_url = body
        .and_then(|b| b.access_service_url.clone())
        .unwrap_or_else(get_default_access_service_url);
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

#[cfg(test)]
mod tests {
    use super::super::tests::test_state;
    use super::AuthorizeRequest;
    use crate::StatusResponse;
    use crate::{AuthorizeResponse, SyncResponse, api_router};

    use anyhow::Result;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tonk_access_service::helpers::AccessServiceAddress;
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

    #[dialog_common::test]
    async fn it_authorizes_with_access_service(env: AccessServiceAddress) -> Result<()> {
        let state = test_state().await;
        let app = api_router(state);

        // Authorize with the test access service URL
        let authorize_request = AuthorizeRequest {
            access_service_url: Some(format!("{}/ucan/", env.access_service_url)),
        };

        let request = Request::builder()
            .uri("/api/authorize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&authorize_request).unwrap(),
            ))
            .expect("Failed to build request");

        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("Failed to read response body");

        let authorize_response: AuthorizeResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(authorize_response.success);
        assert!(authorize_response.error.is_none());

        // Verify status now shows upstream configured
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

        assert!(status_response.has_upstream);
        Ok(())
    }

    #[dialog_common::test]
    async fn it_syncs_with_access_service(env: AccessServiceAddress) -> Result<()> {
        let state = test_state().await;
        let app = api_router(state);

        // First authorize with the test access service
        let authorize_request = AuthorizeRequest {
            access_service_url: Some(format!("{}/ucan/", env.access_service_url)),
        };

        let request = Request::builder()
            .uri("/api/authorize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&authorize_request).unwrap(),
            ))
            .expect("Failed to build request");

        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("Failed to execute request");

        assert_eq!(response.status(), StatusCode::OK);

        // Now perform sync
        let request = Request::builder()
            .uri("/api/sync")
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

        let sync_response: SyncResponse =
            serde_json::from_slice(&body).expect("Failed to deserialize response");

        assert!(sync_response.success, "Sync should succeed");
        Ok(())
    }
}
