//! Invite claim endpoint.
//!
//! Accepts an invite URL from the UI, runs [`tonk_invite::Invite::claim`]
//! against the profile's DID, and persists the resulting delegation chain
//! to the profile. Configuring a sync remote from the invite's `remote_url`
//! is intentionally left to a follow-up endpoint.

use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan::UcanDelegation;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::AppState;
use crate::TonkWorkerError;

/// Body for `POST /api/invite/claim`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimInviteRequest {
    /// Full invite URL, including any `#fragment` carrying the ephemeral
    /// seed for open invites. The service worker is the only consumer, so
    /// the fragment survives transport.
    pub url: String,
}

/// Response from `POST /api/invite/claim`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimInviteResponse {
    /// Whether the chain was successfully claimed and persisted.
    pub success: bool,
    /// DID of the repo the invite targeted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Sync remote URL declared by the inviter, if any. The UI can use
    /// this to decide whether to kick off a separate remote-setup flow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// Error message on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Claim an invite URL and persist the resulting delegation chain.
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

    let subject = claimed.subject().to_string();
    let remote_url = claimed.remote_url.map(|u| u.to_string());

    tonk_state
        .profile
        .access()
        .save(UcanDelegation(claimed.chain))
        .perform(&tonk_state.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to persist delegation chain: {e}"))
        })?;

    log!("Claimed invite for subject {subject} (remote_url={remote_url:?})",);

    Ok(Json(ClaimInviteResponse {
        success: true,
        subject: Some(subject),
        remote_url,
        error: None,
    }))
}
