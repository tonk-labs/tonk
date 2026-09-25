//! Revoke a recorded invitation at the space's own access service.
//!
//! The artifact is an ordinary `ucan/revoke` invocation, so it goes to
//! the same `/ucan/` endpoint every other invocation does: the access
//! service records it in the index its presign path already screens
//! against. There is no separate relay to configure or to miss.

use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use tonk_account::customer::RevokeReceipt;
use tonk_common::log;

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

/// This profile's account's authority over the space: a `/` chain from the
/// space down to the account.
///
/// Searched on the space's own branch first, proving as the account: that
/// is where an admin's chain lives, retained by whoever promoted them. The
/// creation prefix persisted at space creation is the fallback, for a
/// founder whose space db holds no chains yet.
pub(super) async fn account_authority(
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
    match branch
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
                    Ok(chain)
                }
                None => super::repository::space_root_prefix(tonk, subject).await,
            }
        }
        Err(_) => super::repository::space_root_prefix(tonk, subject).await,
    }
}

/// This device's authority to revoke under the space: the account's chain
/// with the root-to-device grant pushed on top, the pair every other
/// invocation on a space subject presents.
async fn revoking_authority(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
) -> Result<DelegationChain, TonkWorkerError> {
    let root = super::identity::local_root(tonk).await?;
    let mut authority = account_authority(tonk, branch, subject).await?;
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
/// a subject the presented proofs do not authorize.
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
