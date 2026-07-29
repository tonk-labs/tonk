//! Revoke a recorded invitation through its configured global relay.

use axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use ipld_core::cid::Cid;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_schema::{Invitation, InvitationExecution};
use tonk_worker_api::{InvitationKind, InvitationSummary, RevokeInvitationAcknowledgement};

use super::AppState;
use crate::TonkWorkerError;

/// Revoke only an invitation path recorded in the named repository.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path((repo, target_cid)): Path<(String, String)>,
) -> Result<Json<RevokeInvitationAcknowledgement>, TonkWorkerError> {
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
    let executions: Vec<InvitationExecution> = session
        .handle()
        .query()
        .select(Query::<InvitationExecution> {
            this: Term::var("this"),
            kind: Term::var("kind"),
            revocation_url: Term::var("revocation_url"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invitation execution query failed: {error:?}"))
        })?;
    let execution = executions
        .into_iter()
        .find(|execution| execution.this == invitation.this)
        .ok_or_else(|| {
            TonkWorkerError::Conflict(
                "this legacy invitation has no revocation relay; configure an explicit relay and mint a new invitation"
                    .to_string(),
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
    let relay = url::Url::parse(&execution.revocation_url.0)
        .map_err(|error| TonkWorkerError::Internal(format!("invalid revocation relay: {error}")))?;
    let response = super::http::post_cbor(&relay, &artifact).await?;
    let acknowledgement: RevokeInvitationAcknowledgement = serde_json::from_slice(&response.body)
        .map_err(|error| {
        TonkWorkerError::Internal(format!(
            "revocation relay returned an invalid acknowledgement: {error}"
        ))
    })?;
    if acknowledgement.target_cid != target_cid {
        return Err(TonkWorkerError::Internal(
            "revocation relay acknowledged a different invitation".to_string(),
        ));
    }
    Ok(Json(acknowledgement))
}

/// List secret-free invitation management rows for one repository.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<Vec<InvitationSummary>>, TonkWorkerError> {
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
    let executions: Vec<InvitationExecution> = session
        .handle()
        .query()
        .select(Query::<InvitationExecution> {
            this: Term::var("this"),
            kind: Term::var("kind"),
            revocation_url: Term::var("revocation_url"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invitation execution query failed: {error:?}"))
        })?;

    let mut rows = invitations
        .into_iter()
        .map(|invitation| {
            let execution = executions
                .iter()
                .find(|execution| execution.this == invitation.this);
            let kind = match execution.map(|execution| execution.kind.0.as_str()) {
                Some("open") => InvitationKind::Open,
                Some("scoped") => InvitationKind::Scoped,
                _ => InvitationKind::Unknown,
            };
            let recipient_root = (kind == InvitationKind::Scoped)
                .then(|| invitation.audience.0.to_string().parse().ok())
                .flatten();
            InvitationSummary {
                target_cid: invitation.target_cid.0,
                kind,
                recipient_root,
                status: if execution.is_some() {
                    "active".to_string()
                } else {
                    "unconfigured".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.target_cid.cmp(&right.target_cid));
    Ok(Json(rows))
}
