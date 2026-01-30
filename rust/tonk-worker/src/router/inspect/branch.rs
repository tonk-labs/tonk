//! Branch inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use base58::ToBase58;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_space::Revision;

/// Wrapper for displaying Edition as base58 hash.
struct EditionHash<'a, T>(&'a T);

impl<T: Hash> Display for EditionHash<'_, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Use Hash trait to extract the underlying bytes
        let mut hasher = ByteCaptureHasher::new();
        self.0.hash(&mut hasher);
        write!(f, "#{}", hasher.into_bytes().to_base58())
    }
}

/// A hasher that captures the bytes being hashed.
struct ByteCaptureHasher {
    bytes: Vec<u8>,
}

impl ByteCaptureHasher {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Hasher for ByteCaptureHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn finish(&self) -> u64 {
        0 // We don't care about the hash value
    }
}

use super::super::AppState;
use crate::TonkWorkerError;

/// Serializable revision info with all fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionResponse {
    /// Period indicating when this revision was created.
    pub period: usize,
    /// Moment at which this revision was created.
    pub moment: usize,
    /// The issuer DID who created this revision.
    pub issuer: String,
    /// The tree root hash (base58 encoded).
    pub tree: String,
    /// Causal ancestor hashes (base58 encoded).
    pub cause: Vec<String>,
}

impl RevisionResponse {
    /// Create a RevisionResponse from a Revision.
    pub fn from_revision(revision: &Revision) -> Self {
        Self {
            period: *revision.period(),
            moment: *revision.moment(),
            issuer: revision.issuer().to_string(),
            tree: format!("{}", revision.tree()),
            cause: revision
                .cause()
                .iter()
                .map(|c| format!("{}", EditionHash(c)))
                .collect(),
        }
    }
}

/// Response for branch status query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchStatusResponse {
    /// The subject DID (space DID).
    pub subject: String,
    /// The branch name.
    pub branch: String,
    /// Current revision with full details.
    pub revision: RevisionResponse,
    /// Base tree hash (the tree we're based off for tracking local changes).
    pub base: String,
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
    /// The subject DID of the upstream repository (None for local upstream).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The upstream revision with full details (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<RevisionResponse>,
}

/// Returns the status of a specific branch.
#[wasm_compat]
pub async fn branch(
    State(state): State<AppState>,
    Path(branch_name): Path<String>,
) -> Result<Json<BranchStatusResponse>, TonkWorkerError> {
    log!("Querying branch status for: {}", branch_name);
    let tonk_state = state.read().await;

    match tonk_state.session.space().branch_info(&branch_name).await {
        Ok(branch_info) => {
            let upstream = branch_info.upstream.map(|u| {
                UpstreamStatusResponse {
                    site: u.site,
                    branch: u.branch,
                    subject: u.subject,
                    // Revision is not available without connecting to remote
                    // Use the sync endpoints to get the latest revision
                    revision: None,
                }
            });

            Ok(Json(BranchStatusResponse {
                subject: tonk_state.session.space_did().to_string(),
                branch: branch_info.name,
                revision: RevisionResponse::from_revision(&branch_info.revision),
                base: branch_info.base,
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
