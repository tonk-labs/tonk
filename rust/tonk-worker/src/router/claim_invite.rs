//! `POST /api/claim`: redeem an invite URL.
//!
//! Parses the invite, persists the resulting delegation chain, opens a
//! local repo scoped to the invited subject, configures an `origin`
//! remote pointing at the invite's access service (if one was attached),
//! sets upstream on `main`, and registers the new local name in `home`.
//!
//! Response shape mirrors `PUT /api/repository/{repo}` — a
//! [`RepositoryInfo`] — so every successful create path (PUT or claim)
//! produces the same representation.

use std::sync::atomic::{AtomicU64, Ordering};

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
use super::home;
use super::repository::{RepositoryInfo, build_repository_info};
use crate::TonkWorkerError;

/// Body of `POST /api/claim`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClaimRequest {
    /// Full invite URL including any `#fragment`.
    ///
    /// Audience-open invites carry the ephemeral seed in the URL
    /// fragment; browsers never send fragments with `fetch`, so the
    /// UI must read `window.location.href` client-side and forward
    /// the complete string in this body.
    pub url: String,
}

/// Generate a unique local repo name.
///
/// Combines a nanosecond timestamp with a process-wide counter. Collisions
/// are not realistically possible under normal use.
fn generate_local_name() -> String {
    use dialog_common::time;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = time::now()
        .duration_since(time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("repo-{ts}-{seq}")
}

/// Redeem an invite URL.
#[wasm_compat]
pub async fn claim_invite(
    State(state): State<AppState>,
    Json(body): Json<ClaimRequest>,
) -> Result<Json<RepositoryInfo>, TonkWorkerError> {
    log!("POST /api/claim");

    let tonk = state.write().await;
    let audience = tonk.profile.did();

    let claimed = tonk_invite::Invite::parse_url(&body.url)
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?
        .claim(&audience)
        .await
        .map_err(|e| TonkWorkerError::Router(format!("invalid invite: {e}")))?;

    let subject = claimed.subject().clone();
    let remote_url = claimed.remote_url.clone();

    // 1. Persist the delegation chain. This is what grants the profile
    // access to the invited subject.
    tonk.profile
        .access()
        .save(UcanDelegation(claimed.chain))
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to persist delegation chain: {e}"))
        })?;

    // 2. Open a local repo keyed by a generated name. The local repo's
    // DID is fresh; the invited subject DID is tracked through the
    // remote configuration (step 3), not through the local DID.
    let local_name = generate_local_name();
    let repository = tonk
        .profile
        .repository(&local_name)
        .open()
        .perform(&tonk.operator)
        .await
        .map_err(|e| {
            TonkWorkerError::Internal(format!("failed to open local repo '{local_name}': {e}"))
        })?;

    // 3. Configure `origin` + upstream when the invite carried a remote.
    // Absent remote_url = local-only repo (no sync configured).
    if let Some(url) = remote_url {
        let address = SiteAddress::from(UcanAddress::new(url.as_str()));

        repository
            .remote("origin")
            .create(address)
            .subject(subject.clone())
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "failed to create remote for '{local_name}': {e}"
                ))
            })?;

        let branch = repository
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!(
                    "failed to open main branch on '{local_name}': {e}"
                ))
            })?;

        let remote_repo = repository
            .remote("origin")
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("failed to load remote 'origin': {e}"))
            })?;

        let remote_branch = remote_repo
            .branch("main")
            .open()
            .perform(&tonk.operator)
            .await
            .map_err(|e| {
                TonkWorkerError::Internal(format!("failed to open remote main: {e}"))
            })?;

        branch
            .set_upstream(&remote_branch)
            .perform(&tonk.operator)
            .await
            .map_err(|e| TonkWorkerError::Internal(format!("failed to set upstream: {e}")))?;
    }

    // 4. Register in home so the new repo shows up in the sidebar.
    home::register_repo(&tonk, &local_name).await?;

    log!(
        "Claimed invite for subject {} as local repo '{}'",
        subject,
        local_name,
    );

    let info = build_repository_info(&tonk, &local_name, &repository).await;
    Ok(Json(info))
}
