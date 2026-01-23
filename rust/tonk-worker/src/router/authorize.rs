//! Authorization routes using UCAN-based authentication.
//!
//! The authorization endpoint uses the operator and delegation that were
//! created when the service worker started. No external input is needed.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_s3_credentials::Credentials;
use dialog_s3_credentials::ucan::{Credentials as UcanCredentials, DelegationChain};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::{RemoteState, SpaceError};

use super::AppState;
use crate::TonkWorkerError;

/// Access service endpoint for UCAN-based authorization.
/// This URL is used for both local dev (via Trunk proxy) and production.
const ACCESS_SERVICE_URL: &str = "/ucan/";

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

/// Response for site status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteStatusResponse {
    /// The site name.
    pub name: String,
    /// Whether the site exists and is configured.
    pub exists: bool,
    /// Credentials info if the site exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<CredentialsResponse>,
}

/// Credentials configuration info.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CredentialsResponse {
    /// S3-based credentials
    S3 {
        /// The S3 region
        region: String,
        /// The S3 bucket name
        bucket: String,
        /// Whether private (signed) access is configured
        is_private: bool,
    },
    /// UCAN-based credentials
    Ucan {
        /// The access service endpoint
        service_url: String,
        /// The audience DID (operator)
        audience_did: String,
        /// The subject DID (from delegation)
        #[serde(skip_serializing_if = "Option::is_none")]
        subject_did: Option<String>,
        /// The command scope
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
    },
}

/// Response for branch status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchStatusResponse {
    /// The branch name.
    pub name: String,
    /// Current revision period.
    pub period: usize,
    /// Current revision moment.
    pub moment: usize,
    /// Upstream info if configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<UpstreamStatusResponse>,
}

/// Upstream status info.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamStatusResponse {
    /// The site name (None for local upstream).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// The branch name on the upstream.
    pub branch: String,
    /// The upstream revision period if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<usize>,
    /// The upstream revision moment if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moment: Option<usize>,
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
    log!("Setting up UCAN credentials for space: {}", space_did);

    // Create UCAN credentials with the access service endpoint
    let ucan_credentials = UcanCredentials::new(ACCESS_SERVICE_URL.to_string(), delegation_chain);

    let remote_state = RemoteState {
        site: "origin".to_string(),
        credentials: Credentials::Ucan(ucan_credentials),
    };

    // Add the remote to the space (without setting as upstream for now)
    // If the remote already exists, that's fine - treat it as success
    match tonk_state.space.add_remote(remote_state).await {
        Ok(site) => {
            log!("Remote '{}' added successfully (upstream not set)", site);
        }
        Err(SpaceError::Replica(ref e)) if format!("{:?}", e).contains("RemoteAlreadyExists") => {
            log!("Remote 'origin' already configured, skipping");
        }
        Err(e) => {
            log!("Failed to add remote: {:?}", e);
            return Err(TonkWorkerError::Internal(format!(
                "Failed to add remote: {}",
                e
            )));
        }
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
        site: request.site_name.clone(),
        credentials: Credentials::Ucan(ucan_credentials),
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

/// Returns the status of a specific remote site.
#[wasm_compat]
pub async fn site_status(
    State(state): State<AppState>,
    Path(site_name): Path<String>,
) -> Result<Json<SiteStatusResponse>, TonkWorkerError> {
    log!("Querying site status for: {}", site_name);
    let tonk_state = state.read().await;

    match tonk_state.space.resolve_site(&site_name).await {
        Ok(site_info) => {
            let credentials = site_info.credentials.map(|c| match c {
                tonk_space::CredentialsInfo::S3 {
                    region,
                    bucket,
                    is_private,
                } => CredentialsResponse::S3 {
                    region,
                    bucket,
                    is_private,
                },
                tonk_space::CredentialsInfo::Ucan {
                    service_url,
                    audience_did,
                    subject_did,
                    command,
                } => CredentialsResponse::Ucan {
                    service_url,
                    audience_did,
                    subject_did,
                    command,
                },
            });

            Ok(Json(SiteStatusResponse {
                name: site_info.name,
                exists: true,
                credentials,
            }))
        }
        Err(SpaceError::Replica(_)) => {
            // Site doesn't exist
            Ok(Json(SiteStatusResponse {
                name: site_name,
                exists: false,
                credentials: None,
            }))
        }
        Err(e) => {
            log!("Error resolving site: {:?}", e);
            Err(TonkWorkerError::Internal(format!(
                "Failed to resolve site: {}",
                e
            )))
        }
    }
}

/// Returns the status of a specific branch.
#[wasm_compat]
pub async fn branch_status(
    State(state): State<AppState>,
    Path(branch_name): Path<String>,
) -> Result<Json<BranchStatusResponse>, TonkWorkerError> {
    log!("Querying branch status for: {}", branch_name);
    let tonk_state = state.read().await;

    match tonk_state.space.branch_info(&branch_name).await {
        Ok(branch_info) => {
            let upstream = branch_info.upstream.map(|u| {
                let (period, moment) = u
                    .revision
                    .map(|r| (Some(r.period), Some(r.moment)))
                    .unwrap_or((None, None));
                UpstreamStatusResponse {
                    site: u.site,
                    branch: u.branch,
                    period,
                    moment,
                }
            });

            Ok(Json(BranchStatusResponse {
                name: branch_info.name,
                period: branch_info.revision.period,
                moment: branch_info.revision.moment,
                upstream,
            }))
        }
        Err(e) => {
            log!("Error getting branch info: {:?}", e);
            Err(TonkWorkerError::Internal(format!(
                "Failed to get branch info: {}",
                e
            )))
        }
    }
}

/// Path parameters for remote branch resolution.
#[derive(Debug, Deserialize)]
pub struct RemoteBranchPath {
    site: String,
    repo_did: String,
    branch: String,
}

/// Response for remote branch resolution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteBranchStatusResponse {
    /// The site name.
    pub site: String,
    /// The repository DID.
    pub repo_did: String,
    /// The branch name.
    pub branch: String,
    /// Whether the resolution succeeded.
    pub success: bool,
    /// The resolved revision period (if successful).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<usize>,
    /// The resolved revision moment (if successful).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moment: Option<usize>,
    /// Error message if resolution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Resolves a remote branch by actually connecting to the remote.
///
/// This endpoint validates that credentials work and the remote is reachable.
#[wasm_compat]
pub async fn resolve_remote_branch(
    State(state): State<AppState>,
    Path(params): Path<RemoteBranchPath>,
) -> Result<Json<RemoteBranchStatusResponse>, TonkWorkerError> {
    log!(
        "Resolving remote branch: site={}, repo={}, branch={}",
        params.site,
        params.repo_did,
        params.branch
    );
    let tonk_state = state.read().await;

    match tonk_state
        .space
        .resolve_remote_branch(&params.site, &params.repo_did, &params.branch)
        .await
    {
        Ok(info) => {
            let (period, moment) = info
                .revision
                .map(|r| (Some(r.period), Some(r.moment)))
                .unwrap_or((None, None));

            Ok(Json(RemoteBranchStatusResponse {
                site: info.site,
                repo_did: info.repo_did,
                branch: info.branch,
                success: true,
                period,
                moment,
                error: None,
            }))
        }
        Err(e) => {
            log!("Error resolving remote branch: {:?}", e);
            Ok(Json(RemoteBranchStatusResponse {
                site: params.site,
                repo_did: params.repo_did,
                branch: params.branch,
                success: false,
                period: None,
                moment: None,
                error: Some(format!("{}", e)),
            }))
        }
    }
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
