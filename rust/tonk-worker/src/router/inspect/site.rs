//! Site inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::SpaceError;

use super::super::AppState;
use super::branch::RevisionResponse;
use crate::TonkWorkerError;

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
    /// The subject DID (repository/space DID).
    pub subject: String,
    /// The branch name.
    pub branch: String,
    /// Whether the resolution succeeded.
    pub success: bool,
    /// The resolved revision with full details (if successful).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<RevisionResponse>,
    /// Error message if resolution failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Returns the status of a specific remote site.
#[wasm_compat]
pub async fn site(
    State(state): State<AppState>,
    Path(site_name): Path<String>,
) -> Result<Json<SiteStatusResponse>, TonkWorkerError> {
    log!("Querying site status for: {}", site_name);
    let tonk_state = state.read().await;

    match tonk_state.session.space().resolve_site(&site_name).await {
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

/// Resolves a remote branch by actually connecting to the remote.
///
/// This endpoint validates that credentials work and the remote is reachable.
#[wasm_compat]
pub async fn branch(
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
        .session
        .space()
        .resolve_remote_branch(&params.site, &params.repo_did, &params.branch)
        .await
    {
        Ok(info) => {
            let revision = info.revision.as_ref().map(RevisionResponse::from_revision);

            Ok(Json(RemoteBranchStatusResponse {
                site: info.site,
                subject: info.repo_did,
                branch: info.branch,
                success: true,
                revision,
                error: None,
            }))
        }
        Err(e) => {
            log!("Error resolving remote branch: {:?}", e);
            Ok(Json(RemoteBranchStatusResponse {
                site: params.site,
                subject: params.repo_did,
                branch: params.branch,
                success: false,
                revision: None,
                error: Some(format!("{}", e)),
            }))
        }
    }
}
