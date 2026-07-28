//! Revoke a recorded invitation through its configured global relay.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use ipld_core::cid::Cid;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_schema::Invitation;

use super::AppState;
use crate::TonkWorkerError;

/// Revoke only an invitation path recorded in the named repository.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path((repo, target_cid)): Path<(String, String)>,
) -> Result<StatusCode, TonkWorkerError> {
    let target: Cid = target_cid
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid target CID: {error}")))?;
    let tonk = state.read().await;
    let session = tonk
        .reactor
        .repository(&repo)
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let invitations: Vec<Invitation> = session
        .handle()
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
            target_cid: Term::var("target_cid"),
            path_hex: Term::var("path_hex"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invitation query failed: {error:?}"))
        })?;
    let invitation = invitations
        .into_iter()
        .find(|invitation| invitation.target_cid.0 == target_cid)
        .ok_or_else(|| {
            TonkWorkerError::NotFound(
                "the target CID is not a recorded invitation for this repository".to_string(),
            )
        })?;
    let bytes = hex::decode(&invitation.path_hex.0).map_err(|error| {
        TonkWorkerError::Internal(format!("stored invitation path is invalid: {error}"))
    })?;
    let path = dialog_ucan_core::DelegationChain::try_from(bytes.as_slice()).map_err(|error| {
        TonkWorkerError::Internal(format!("stored invitation path is invalid: {error}"))
    })?;
    let artifact = tonk_identity::revocation::mint_root_revocation(
        tonk.profile.signer().signer().clone(),
        &path,
        &target,
    )
    .await
    .map_err(|error| {
        TonkWorkerError::Forbidden(format!("cannot revoke this invitation: {error}"))
    })?;
    tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("revocation preflight failed: {error}"))
        })?;
    let relay = if crate::router::account_backup::account_service_url()
        .as_deref()
        .is_some_and(|url| url.contains("staging"))
    {
        "https://accounts-staging.tonk.xyz/revocations"
    } else {
        "https://accounts.tonk.xyz/revocations"
    };
    crate::router::account_backup::post_for_bytes(relay, artifact).await?;
    Ok(StatusCode::NO_CONTENT)
}
