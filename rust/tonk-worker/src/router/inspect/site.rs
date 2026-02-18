//! Site inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use base58::FromBase58;
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

/// Path parameters for archive block fetch.
#[derive(Debug, Deserialize)]
pub struct ArchiveBlockPath {
    site: String,
    repo_did: String,
    hash: String,
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
        Err(SpaceError::Repository(_)) => {
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

/// Response for archive block fetch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveBlockResponse {
    /// The site name.
    pub site: String,
    /// The subject DID.
    pub subject: String,
    /// The requested hash (base58).
    pub hash: String,
    /// Whether the block was found.
    pub found: bool,
    /// The block data as base64 (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The block size in bytes (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
    /// Error message if fetch failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Fetches a block from a remote site's archive by its blake3 hash.
///
/// The hash should be provided as base58 encoded string.
#[wasm_compat]
pub async fn archive_block(
    State(state): State<AppState>,
    Path(params): Path<ArchiveBlockPath>,
) -> Result<Json<ArchiveBlockResponse>, TonkWorkerError> {
    log!(
        "Fetching archive block: site={}, repo={}, hash={}",
        params.site,
        params.repo_did,
        params.hash
    );

    // Decode the base58 hash
    let hash_bytes = match params.hash.from_base58() {
        Ok(bytes) => bytes,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                site: params.site,
                subject: params.repo_did,
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Invalid base58 hash: {:?}", e)),
            }));
        }
    };

    // Ensure it's exactly 32 bytes
    let hash: [u8; 32] = match hash_bytes.try_into() {
        Ok(h) => h,
        Err(bytes) => {
            return Ok(Json(ArchiveBlockResponse {
                site: params.site,
                subject: params.repo_did,
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Hash must be 32 bytes, got {}", bytes.len())),
            }));
        }
    };

    let tonk_state = state.read().await;

    match tonk_state
        .session
        .space()
        .fetch_remote_archive_block(&params.site, &params.repo_did, &hash)
        .await
    {
        Ok(Some(data)) => {
            use base64::Engine;
            let size = data.len();
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(&data);

            Ok(Json(ArchiveBlockResponse {
                site: params.site,
                subject: params.repo_did,
                hash: params.hash,
                found: true,
                data: Some(data_base64),
                size: Some(size),
                error: None,
            }))
        }
        Ok(None) => Ok(Json(ArchiveBlockResponse {
            site: params.site,
            subject: params.repo_did,
            hash: params.hash,
            found: false,
            data: None,
            size: None,
            error: None,
        })),
        Err(e) => {
            log!("Error fetching archive block: {:?}", e);
            Ok(Json(ArchiveBlockResponse {
                site: params.site,
                subject: params.repo_did,
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("{}", e)),
            }))
        }
    }
}
