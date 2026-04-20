//! Invite claim endpoint.
//!
//! Accepts an invite URL from the UI, runs [`tonk_invite::Invite::claim`]
//! against the profile's DID, persists the resulting delegation chain,
//! opens a local repo handle scoped to the invited subject, and — if the
//! invite carried a `remote_url` — configures a UCAN sync remote and
//! sets upstream on the default branch.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::{RepositoryExt as _, SiteAddress};
use dialog_ucan::UcanDelegation;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use super::create::generate_local_name;
use crate::TonkWorkerError;

/// Body for `POST /api/invite/claim`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimInviteRequest {
    /// Full invite URL, including any `#fragment` carrying the ephemeral
    /// seed for open invites. The service worker is the only consumer, so
    /// the fragment survives transport.
    pub url: String,
}

/// Response from `POST /api/invite/claim`. Fields mirror
/// [`crate::router::CreateRepositoryResponse`] so both flows feed the
/// same sidebar row shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimInviteResponse {
    /// Whether the chain was successfully claimed and persisted.
    pub success: bool,
    /// Local repo name (storage key, URL path segment, API path segment).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_repo: Option<String>,
    /// DID of the repo the invite targeted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Sync remote URL declared by the inviter, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// Whether the default branch has an upstream configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_upstream: Option<bool>,
    /// Error message on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Claim an invite URL and persist the resulting delegation chain.
///
/// Always persists the chain. If `remote_url` is present, additionally
/// opens a local repo keyed by an auto-generated name, points its
/// `origin` remote at the invite's access service (scoped to the
/// invited subject), and sets upstream on `main`.
#[wasm_compat]
pub async fn claim_invite(
    State(state): State<AppState>,
    Json(body): Json<ClaimInviteRequest>,
) -> Result<Json<ClaimInviteResponse>, TonkWorkerError> {
    log!("Claiming invite…");

    let tonk_state = state.write().await;
    let audience = tonk_state.profile.did();

    let claimed = tonk_invite::Invite::parse_url(&body.url)
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?
        .claim(&audience)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?;

    let subject = claimed.subject().clone();
    let subject_str = subject.to_string();
    let remote_url = claimed.remote_url.clone();

    tonk_state
        .profile
        .access()
        .save(UcanDelegation(claimed.chain))
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to persist delegation chain: {e}"))
        })?;

    // Local repo handle through which the redeemer will interact with
    // the invited subject. Name is arbitrary; the subject DID carries
    // the sync identity via the remote configuration.
    let local_name = generate_local_name();
    let repo = tonk_state
        .profile
        .repository(&local_name)
        .open()
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!(
                "failed to open local repo '{local_name}' for invited subject: {e}"
            ))
        })?;

    let has_upstream = if let Some(ref url) = remote_url {
        let address = SiteAddress::from(UcanAddress::new(url.as_str()));

        match repo
            .remote("origin")
            .create(address)
            .subject(subject.clone())
            .perform(&tonk_state.operator)
            .await
        {
            Ok(_) => {}
            Err(e) if format!("{e:?}").contains("RemoteAlreadyExists") => {}
            Err(e) => {
                return Err(TonkWorkerError::Internal(format!(
                    "failed to create remote for invited repo '{local_name}': {e}"
                )));
            }
        }

        let branch = repo
            .branch("main")
            .open()
            .perform(&tonk_state.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "failed to open main branch on '{local_name}': {e}"
                ))
            })?;

        if branch.upstream().is_none() {
            let remote_repo = repo
                .remote("origin")
                .load()
                .perform(&tonk_state.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!("failed to load remote 'origin': {e}"))
                })?;
            let remote_branch = remote_repo
                .branch("main")
                .open()
                .perform(&tonk_state.operator)
                .await
                .map_err(|e| {
                    TonkWorkerError::Internal(format!("failed to open remote main: {e}"))
                })?;
            branch
                .set_upstream(&remote_branch)
                .perform(&tonk_state.operator)
                .await
                .map_err(|e| TonkWorkerError::Internal(format!("failed to set upstream: {e}")))?;
        }
        Some(true)
    } else {
        Some(false)
    };

    log!("Claimed invite for subject {subject_str} as local repo '{local_name}' (remote_url={remote_url:?})");

    Ok(Json(ClaimInviteResponse {
        success: true,
        local_repo: Some(local_name),
        subject: Some(subject_str),
        remote_url: remote_url.map(|u| u.to_string()),
        has_upstream,
        error: None,
    }))
}
