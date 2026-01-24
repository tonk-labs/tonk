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

    // Create delegation chain from our delegation
    let delegation_chain = DelegationChain::new(tonk_state.delegation.inner().clone());

    let space_did = tonk_state.space.did.clone();
    let service_url = get_access_service_url();
    log!(
        "Setting up UCAN credentials for space: {} with URL: {}",
        space_did,
        service_url
    );

    // Create UCAN credentials with the resolved access service URL
    let ucan_credentials = UcanCredentials::new(service_url, delegation_chain);

    let remote_state = RemoteState {
        site: "origin".into(),
        credentials: RemoteCredentials::Ucan(ucan_credentials),
    };

    // Add the remote to the space
    // If the remote already exists, that's fine - treat it as success
    match tonk_state.space.add_remote(remote_state).await {
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
    if !tonk_state.space.has_upstream().await {
        log!("Setting 'origin' as upstream for main branch...");
        match tonk_state.space.set_upstream("origin").await {
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

/// Request body for adding a test site with a custom URL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddTestSiteRequest {
    /// The absolute URL for the access service (e.g., "https://example.com/ucan/")
    pub service_url: String,
    /// The site name (defaults to "test")
    #[serde(default = "default_test_site_name")]
    pub site_name: String,
}

fn default_test_site_name() -> String {
    "test".to_string()
}

/// Adds a test remote site with a custom absolute URL.
///
/// Use this to test if the relative URL `/ucan/` is causing issues by providing
/// an absolute URL instead.
#[wasm_compat]
pub async fn add_test_site(
    State(state): State<AppState>,
    Json(request): Json<AddTestSiteRequest>,
) -> Result<Json<AuthorizeResponse>, TonkWorkerError> {
    log!(
        "Adding test site '{}' with URL: {}",
        request.site_name,
        request.service_url
    );

    let mut tonk_state = state.write().await;

    // Create delegation chain from our delegation
    let delegation_chain = DelegationChain::new(tonk_state.delegation.inner().clone());

    // Create UCAN credentials with the provided absolute URL
    let ucan_credentials = UcanCredentials::new(request.service_url.clone(), delegation_chain);

    let remote_state = RemoteState {
        site: request.site_name.clone().into(),
        credentials: RemoteCredentials::Ucan(ucan_credentials),
    };

    // Add the remote to the space
    match tonk_state.space.add_remote(remote_state).await {
        Ok(site) => {
            log!("Test site '{}' added successfully", site);
        }
        Err(SpaceError::Replica(ref e))
            if format!("{:?}", e).contains("RemoteAlreadyExists") =>
        {
            log!("Test site '{}' already configured, skipping", request.site_name);
        }
        Err(e) => {
            log!("Failed to add test site: {:?}", e);
            return Err(TonkWorkerError::Internal(format!(
                "Failed to add test site: {}",
                e
            )));
        }
    }

    Ok(Json(AuthorizeResponse {
        success: true,
        error: None,
    }))
}
