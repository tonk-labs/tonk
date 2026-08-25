//! Revoke a recorded invitation at the space's own access service.
//!
//! The artifact is an ordinary `ucan/revoke` invocation, so it goes to
//! the same `/ucan/` endpoint every other invocation does: the access
//! service records it in the index its presign path already screens
//! against. There is no separate relay to configure or to miss.

use axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::Did;
use ipld_core::cid::Cid;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_account::customer::RevokeReceipt;
use tonk_common::log;
use tonk_schema::{Invitation, InvitationExecution};
use tonk_worker_api::{InvitationKind, InvitationSummary};

use super::AppState;
use super::create_invite::{ConfiguredRemoteRequirement, resolve_configured_remote_url_with};
use crate::{TonkState, TonkWorkerError};

/// The scope an invite covers: using the space. Invites are minted at
/// `/use`, so this is the level a proof search has to aim at; a `/` chain
/// (the founder's, an admin's) covers it too.
fn space_scope(subject: &Did) -> Scope {
    Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse("/use").expect("the use command always parses"),
        parameters: Parameters::default(),
    }
}

/// Rebuild the delegation path that reaches `audience`, from the delegation
/// facts retained on the repository's content branch.
///
/// This replaces reading a hex blob off the invitation record. The facts are
/// the authoritative copy: `prove` walks them from the claimant back toward
/// the subject, so the chain it returns is the real path as it stands now,
/// not a snapshot taken at mint time. Proving as the invite's AUDIENCE (not
/// as this profile, and not as the account) is what makes the invite hop the
/// chain's last link, and the revocation witness has to contain that hop.
pub(super) async fn prove_path(
    branch: &dialog_repository::Branch,
    tonk: &TonkState,
    subject: &Did,
    audience: &Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let proof = branch
        .delegations()
        .prove(audience.clone(), space_scope(subject))
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!(
                "no retained delegation path reaches {audience}: {error}"
            ))
        })?;
    let mut certificates = proof.proofs.into_iter();
    let first = certificates
        .next()
        .ok_or_else(|| TonkWorkerError::NotFound(format!("the proof for {audience} is empty")))?;
    let mut chain = DelegationChain::new(first.0);
    for certificate in certificates {
        chain = chain.push(certificate.0).map_err(|error| {
            TonkWorkerError::Internal(format!("proved certificates do not chain: {error}"))
        })?;
    }
    Ok(chain)
}

/// The revocation target a proved path names: its leaf, the hop into the
/// invite's audience.
pub(super) fn leaf_cid(path: &DelegationChain) -> Result<Cid, TonkWorkerError> {
    path.proof_cids()
        .last()
        .copied()
        .ok_or_else(|| TonkWorkerError::Internal("a proved path has no leaf".to_string()))
}

/// Every recorded invitation on `branch`, each paired with the delegation
/// path that currently reaches its audience and the CID of that path's leaf.
///
/// An invitation whose path can no longer be proved is dropped: that is what
/// a revoked or never-retained invite looks like from here, and neither is
/// listable or revocable.
async fn proved_invitations(
    branch: &dialog_repository::Branch,
    tonk: &TonkState,
    subject: &Did,
) -> Result<Vec<(Invitation, DelegationChain, Cid)>, TonkWorkerError> {
    let invitations: Vec<Invitation> = branch
        .query()
        .select(Query::<Invitation> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            inviter: Term::var("inviter"),
            audience: Term::var("audience"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invitation query failed: {error:?}"))
        })?;

    let mut proved = Vec::new();
    for invitation in invitations {
        let Ok(audience) = invitation.audience.0.to_string().parse::<Did>() else {
            log!(
                "invitation {} has an unparseable audience; skipping",
                invitation.this
            );
            continue;
        };
        // Two cases land here and they are not the same: an invite that was
        // revoked (its leaf is retracted, so it should disappear) and one
        // minted before chains were retained (nothing was ever written, so it
        // disappears without having been revoked). Neither is actionable from
        // here, but they are worth telling apart in a log.
        let Ok(path) = prove_path(branch, tonk, subject, &audience).await else {
            log!(
                "invitation {} has no provable path to {audience}; \
                 it was revoked, or minted before its chain was retained",
                invitation.this
            );
            continue;
        };
        let cid = leaf_cid(&path)?;
        proved.push((invitation, path, cid));
    }
    Ok(proved)
}

/// The recorded invitation and proved path whose leaf is `target`.
async fn resolve_target(
    branch: &dialog_repository::Branch,
    tonk: &TonkState,
    subject: &Did,
    target: &Cid,
) -> Result<(DelegationChain, Invitation), TonkWorkerError> {
    proved_invitations(branch, tonk, subject)
        .await?
        .into_iter()
        .find(|(_, _, cid)| cid == target)
        .map(|(invitation, path, _)| (path, invitation))
        .ok_or_else(|| {
            TonkWorkerError::NotFound(
                "the target CID is not a live invitation for this repository".to_string(),
            )
        })
}

/// Revoke only an invitation path recorded in the named repository.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path((repo, target_cid)): Path<(String, String)>,
) -> Result<Json<RevokeReceipt>, TonkWorkerError> {
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
    // The subject comes from the repository rather than off the stored
    // path: an invite is scoped to the space, so the space's own DID is
    // what a proof search has to aim at.
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!("repository '{repo}' not found: {error}"))
        })?;
    let subject = repository.did();

    // The target names a hop, and the hop is reachable only by proving as
    // the principal it lands on. So resolve the recorded invitation whose
    // audience the target belongs to, rather than searching the facts for a
    // CID they do not carry (the facts are keyed by the blob store's blake3
    // of the envelope, while a UCAN CID is dag-cbor/sha2-256).
    let (path, invitation) = resolve_target(session.handle(), &tonk, &subject, &target).await?;

    let receipt =
        publish_revocation(&tonk, &repo, &repository, session.handle(), &path, &target).await?;
    retract_leaf(&tonk, session.handle(), &path).await;
    // The record is what `list` enumerates, so it goes with the hop it
    // described.
    if let Err(error) = tonk
        .reactor
        .repository(&repo)
        .branch("main")
        .transaction()
        .retract(invitation)
        .commit()
        .perform(&tonk.operator)
        .await
    {
        log!("revoked invitation record was not retracted: {error}");
    }

    Ok(Json(receipt))
}

/// This device's authority to revoke under the space: a `/` chain from the
/// space down to this device.
///
/// Searched on the space's own branch first, proving as this profile's
/// account: that is where an admin's chain lives, retained by whoever
/// promoted them. The creation prefix persisted at space creation is the
/// fallback, for a founder whose space db holds no chains yet. Either way
/// the chain reaches the account, so the root-to-device grant is pushed on
/// top: that is the pair every other invocation on a space subject presents.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn revoking_authority(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let root = super::identity::local_root(tonk).await?;
    let full = Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse("/").expect("the root command always parses"),
        parameters: Parameters::default(),
    };
    let mut authority = match branch
        .delegations()
        .prove(root.root_did.clone(), full)
        .perform(&tonk.operator)
        .await
    {
        Ok(proof) => {
            let mut certificates = proof.proofs.into_iter();
            match certificates.next() {
                Some(first) => {
                    let mut chain = DelegationChain::new(first.0);
                    for certificate in certificates {
                        chain = chain.push(certificate.0).map_err(|error| {
                            TonkWorkerError::Internal(format!(
                                "proved certificates do not chain: {error}"
                            ))
                        })?;
                    }
                    chain
                }
                None => super::repository::space_root_prefix(tonk, subject).await?,
            }
        }
        Err(_) => super::repository::space_root_prefix(tonk, subject).await?,
    };
    for delegation in root.delegation.proofs() {
        authority = authority.push(delegation.clone()).map_err(|error| {
            TonkWorkerError::Internal(format!(
                "space authority and device grant do not chain: {error}"
            ))
        })?;
    }
    Ok(authority)
}

/// Mint the delegated revocation of `target` under this device's authority
/// for the space and record it at the space's access service.
///
/// The revocation's subject is the space, but this device signs it, so the
/// invocation carries the chain that proves the device may act for that
/// subject; `/ucan/` runs the full chain check before dispatch and refuses
/// a subject the presented proofs do not authorize. Native builds carry
/// this for the router's shape, not to run it: the authority is persisted
/// by the browser.
pub(super) async fn publish_revocation<R>(
    tonk: &TonkState,
    repo: &str,
    repository: &dialog_repository::Repository<R>,
    branch: &dialog_repository::Branch,
    path: &DelegationChain,
    target: &Cid,
) -> Result<RevokeReceipt, TonkWorkerError>
where
    R: dialog_varsig::Principal + Clone,
{
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    let artifact = {
        let subject = repository.did();
        let authority = revoking_authority(tonk, branch, &subject).await?;
        tonk_identity::revocation::mint_delegated_revocation(
            tonk.profile.signer().signer().clone(),
            path,
            target,
            &authority,
        )
        .await
        .map_err(|error| TonkWorkerError::Forbidden(format!("cannot revoke this grant: {error}")))?
    };
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let artifact = {
        let _ = branch;
        tonk_identity::revocation::mint_root_revocation(
            tonk.profile.signer().signer().clone(),
            path,
            target,
        )
        .await
        .map_err(|error| TonkWorkerError::Forbidden(format!("cannot revoke this grant: {error}")))?
    };
    tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("revocation preflight failed: {error}"))
        })?;
    // The revocation belongs at the access service the space actually
    // syncs through, which is the remote `main` tracks.
    let endpoint = match resolve_configured_remote_url_with(repository, &tonk.operator).await? {
        ConfiguredRemoteRequirement::Ready(remote) => remote.access_url,
        ConfiguredRemoteRequirement::Refused(reason) => {
            return Err(TonkWorkerError::Conflict(format!(
                "cannot revoke a grant on '{repo}': {} ({})",
                reason.detail(),
                reason.code()
            )));
        }
    };
    let response = super::http::post_cbor(&endpoint, &artifact).await?;
    let receipt: RevokeReceipt = serde_json::from_slice(&response.body).map_err(|error| {
        TonkWorkerError::Internal(format!(
            "the access service returned an unreadable revoke receipt: {error}"
        ))
    })?;
    if receipt.revoked != *target {
        return Err(TonkWorkerError::Internal(
            "the access service acknowledged a different grant".to_string(),
        ));
    }
    Ok(receipt)
}

/// Retract the leaf of a revoked path from the space's retained chains.
///
/// Only the leaf. `path` runs space -> ... -> device -> holder, and every
/// other grant (and this device's everyday access) proves through that same
/// prefix. Retracting the whole path would pull the profile-to-account union
/// and the space-to-profile hop out from under all of them, revoking far
/// more than the one grant that was asked for. Best-effort: the revocation
/// is already durable at the access service, which is what denies the
/// holder; a leaf left behind is listed, not live.
pub(super) async fn retract_leaf(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    path: &DelegationChain,
) {
    let Some(leaf) = path.proofs().last().cloned() else {
        log!("a proved path has no leaf to retract");
        return;
    };
    if let Err(error) = branch
        .delegations()
        .retract(UcanDelegation(DelegationChain::new(leaf)))
        .perform(&tonk.operator)
        .await
    {
        log!("revoked grant was not retracted locally: {error}");
    }
}

/// List secret-free invitation management rows for one repository.
///
/// The target CID a row reports is not stored: it is the leaf of the
/// delegation path proved from the invitation's audience, computed the same
/// way [`revoke`] resolves the target it is handed. Deriving both from one
/// walk is what keeps a listed CID revocable, rather than being a stale
/// mint-time snapshot the live facts no longer agree with.
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
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|error| {
            TonkWorkerError::NotFound(format!("repository '{repo}' not found: {error}"))
        })?;
    let subject = repository.did();

    let executions: Vec<InvitationExecution> = session
        .handle()
        .query()
        .select(Query::<InvitationExecution> {
            this: Term::var("this"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invitation execution query failed: {error:?}"))
        })?;

    let mut rows = proved_invitations(session.handle(), &tonk, &subject)
        .await?
        .into_iter()
        .map(|(invitation, _, target)| {
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
                target_cid: target.to_string(),
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
