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
use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_ucan::{Parameters, Scope, UcanDelegation};
use dialog_ucan_core::DelegationChain;
use dialog_ucan_core::command::Command;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_varsig::Did;
use ipld_core::cid::Cid;
use tonk_account::customer::RevokeReceipt;
use tonk_common::log;
use tonk_macros::handler;
use tonk_schema::{Invitation, InvitationExecution};
use tonk_worker_api::{InvitationKind, InvitationSummary};

use super::AppState;
use super::create_invite::{ConfiguredRemoteRequirement, resolve_configured_remote_url};
use crate::{TonkState, TonkWorkerError};

/// The scope an invite covers: using the space. Invites are minted at
/// `/use`, so this is the level a proof search has to aim at; a `/` chain
/// (the founder's, an admin's) covers it too.
fn space_scope(subject: &Did) -> Scope {
    scope_at(subject, "/use")
}

fn scope_at(subject: &Did, command: &str) -> Scope {
    Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse(command).expect("the scopes searched here are fixed commands"),
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
    prove_path_with(branch, tonk, audience, space_scope(subject)).await
}

/// [`prove_path`] for a grant narrower than `/use`: a publication grants
/// the public principal `/use/get`, which a search aimed at `/use` never
/// reaches.
pub(super) async fn prove_path_at(
    branch: &dialog_repository::Branch,
    tonk: &TonkState,
    subject: &Did,
    audience: &Did,
    command: &str,
) -> Result<DelegationChain, TonkWorkerError> {
    prove_path_with(branch, tonk, audience, scope_at(subject, command)).await
}

async fn prove_path_with(
    branch: &dialog_repository::Branch,
    tonk: &TonkState,
    audience: &Did,
    scope: Scope,
) -> Result<DelegationChain, TonkWorkerError> {
    let proof = branch
        .delegations()
        .prove(audience.clone(), scope)
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
#[handler]
pub async fn revoke(
    State(state): State<AppState>,
    Path((repo, target_cid)): Path<(String, String)>,
) -> Result<Json<RevokeReceipt>, TonkWorkerError> {
    let target: Cid = target_cid
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid target CID: {error}")))?;
    // A clone, not a guard: revoking reaches the service over the network.
    let tonk = state.read().await.clone();
    // The invitation's record and its retained path are in the space, which
    // a worker of its own holds: that worker resolves the target and has
    // this one sign ([`revoke_for_space`]).
    if tonk.spaces_elsewhere() {
        drop(tonk);
        let receipt = super::space_reach::ask(
            &repo,
            "POST",
            &format!("/api/repository/{repo}/invites/{target}/revoke"),
            Some(&serde_json::json!({})),
        )
        .await?;
        return serde_json::from_value(receipt).map(Json).map_err(|error| {
            TonkWorkerError::Internal(format!("the space's worker answered no receipt: {error}"))
        });
    }
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
        .space(&repo)
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

/// This profile's account's authority over the space: a `/` chain from the
/// space down to the account.
///
/// Searched on the space's own branch first, proving as the account: that
/// is where an admin's chain lives, retained by whoever promoted them. The
/// creation prefix persisted at space creation is the fallback, for a
/// founder whose space db holds no chains yet.
///
/// A space's own worker has no account and none of the person's records:
/// it proves as the account its delegation names, and asks the person's
/// profile for the creation prefix when the space's chains prove nothing.
pub(super) async fn account_authority(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
) -> Result<DelegationChain, TonkWorkerError> {
    if tonk.registry.standing == crate::device::Standing::Site {
        let account = super::account::acts_for(tonk).await?.ok_or_else(|| {
            TonkWorkerError::Forbidden("this worker has not been told whose space it holds".into())
        })?;
        if let Some(chain) = proved_authority(tonk, branch, subject, &account).await {
            return Ok(chain);
        }
        let answer =
            super::space_reach::ask_profile(&serde_json::json!({ "authority": true })).await?;
        let prefix = answer["authority"].as_str().ok_or_else(|| {
            TonkWorkerError::Forbidden("the profile holds no authority over this space".into())
        })?;
        return decode_chain("the authority", prefix);
    }
    let root = super::identity::local_root(tonk).await?;
    match proved_authority(tonk, branch, subject, &root.root_did).await {
        Some(chain) => Ok(chain),
        None => super::repository::space_root_prefix(tonk, subject).await,
    }
}

/// The `/` chain from the space down to `account` that the chains retained
/// on `branch` prove, if they prove one.
async fn proved_authority(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
    account: &Did,
) -> Option<DelegationChain> {
    let full = Scope {
        subject: UcanSubject::Specific(subject.clone()),
        command: Command::parse("/").expect("the root command always parses"),
        parameters: Parameters::default(),
    };
    let proof = branch
        .delegations()
        .prove(account.clone(), full)
        .perform(&tonk.operator)
        .await
        .ok()?;
    let mut certificates = proof.proofs.into_iter();
    let mut chain = DelegationChain::new(certificates.next()?.0);
    for certificate in certificates {
        chain = match chain.push(certificate.0) {
            Ok(chain) => chain,
            Err(error) => {
                log!("proved certificates do not chain: {error}");
                return None;
            }
        };
    }
    Some(chain)
}

/// This device's authority to act for the account on a space: the account's
/// chain with the root-to-device grant pushed on top, the pair every other
/// invocation on a space subject presents.
async fn device_authority(
    tonk: &TonkState,
    mut authority: DelegationChain,
) -> Result<DelegationChain, TonkWorkerError> {
    let root = super::identity::local_root(tonk).await?;
    for delegation in root.delegation.proofs() {
        authority = authority.push(delegation.clone()).map_err(|error| {
            TonkWorkerError::Internal(format!(
                "space authority and device grant do not chain: {error}"
            ))
        })?;
    }
    Ok(authority)
}

/// What a space's own worker asks the person's profile to sign: the
/// revocation of the grant `target`, which the path `path` (retained in the
/// space) ends at.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct RevocationRequest {
    /// The delegation path that reaches the grant, base58.
    path: String,
    /// The grant's CID.
    target: String,
    /// The `/` chain down to the account that the space's retained chains
    /// prove, base58: an admin's. Absent where they prove none, and the
    /// profile's own record of its authority stands in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authority: Option<String>,
}

fn encode_chain(chain: &DelegationChain) -> Result<String, TonkWorkerError> {
    chain
        .to_bytes()
        .map(|bytes| bs58::encode(bytes).into_string())
        .map_err(|error| TonkWorkerError::Internal(format!("a chain did not encode: {error}")))
}

fn decode_chain(what: &str, encoded: &str) -> Result<DelegationChain, TonkWorkerError> {
    let bytes = bs58::decode(encoded)
        .into_vec()
        .map_err(|error| TonkWorkerError::Router(format!("{what} is not base58: {error}")))?;
    DelegationChain::try_from(bytes.as_slice())
        .map_err(|error| TonkWorkerError::Router(format!("{what} did not decode: {error}")))
}

/// Have the person's profile sign and publish the revocation of `target`,
/// from the space's own worker, which holds the path that reaches it and
/// none of the authority to revoke it.
async fn revoke_through_profile(
    tonk: &TonkState,
    branch: &dialog_repository::Branch,
    subject: &Did,
    path: &DelegationChain,
    target: &Cid,
) -> Result<RevokeReceipt, TonkWorkerError> {
    let authority = match super::account::acts_for(tonk).await? {
        Some(account) => proved_authority(tonk, branch, subject, &account).await,
        None => None,
    };
    let request = RevocationRequest {
        path: encode_chain(path)?,
        target: target.to_string(),
        authority: authority.as_ref().map(encode_chain).transpose()?,
    };
    let answer = super::space_reach::ask_profile(&serde_json::json!({ "revoke": request })).await?;
    serde_json::from_value(answer["receipt"].clone()).map_err(|error| {
        TonkWorkerError::Internal(format!("the profile answered no receipt: {error}"))
    })
}

/// Sign and publish the revocation a space's own worker asked for
/// ([`revoke_through_profile`]), in the person's profile, which holds the
/// authority. The path has to be for `subject`, the space whose worker
/// asked, and end at the grant it names.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub(crate) async fn revoke_for_space(
    tonk: &TonkState,
    subject: &Did,
    request: RevocationRequest,
) -> Result<RevokeReceipt, TonkWorkerError> {
    let repository = super::space_directory::held(tonk, subject.as_str()).await?;
    let path = decode_chain("the path", &request.path)?;
    let target: Cid = request
        .target
        .parse()
        .map_err(|error| TonkWorkerError::Router(format!("invalid target CID: {error}")))?;
    if path.subject() != Some(subject) || leaf_cid(&path)? != target {
        return Err(TonkWorkerError::Forbidden(format!(
            "the path does not reach that grant on {subject}"
        )));
    }
    let account = match &request.authority {
        Some(authority) => {
            let authority = decode_chain("the authority", authority)?;
            if authority.subject() != Some(subject) {
                return Err(TonkWorkerError::Forbidden(format!(
                    "the authority is not over {subject}"
                )));
            }
            authority
        }
        None => super::repository::space_root_prefix(tonk, subject).await?,
    };
    let authority = device_authority(tonk, account).await?;
    publish_revocation_under(
        tonk,
        subject.as_str(),
        &repository,
        &authority,
        &path,
        &target,
    )
    .await
}

/// Mint the delegated revocation of `target` under this device's authority
/// for the space and record it at the space's access service.
///
/// The revocation's subject is the space, but this device signs it, so the
/// invocation carries the chain that proves the device may act for that
/// subject; `/ucan/` runs the full chain check before dispatch and refuses
/// a subject the presented proofs do not authorize.
///
/// That same chain also goes into the witness. `path` ends at whichever key
/// signed the leaf: another device of the account, or this profile's key
/// before it was linked. The witness must show the space reaching the
/// signer of the revocation, and only this device's own authority does that.
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
    let subject = repository.did();
    // A space's own worker holds the space and none of the person's
    // authority over it: the person's profile signs.
    if tonk.registry.standing == crate::device::Standing::Site {
        return revoke_through_profile(tonk, branch, &subject, path, target).await;
    }
    let account = account_authority(tonk, branch, &subject).await?;
    let authority = device_authority(tonk, account).await?;
    publish_revocation_under(tonk, repo, repository, &authority, path, target).await
}

/// The person's account's `/` chain over `subject`, as the profile has it
/// on record: what a space's own worker asks for when the space's retained
/// chains prove none ([`account_authority`]). Base58.
#[cfg(all(target_arch = "wasm32", feature = "persona"))]
pub(crate) async fn authority_for_space(
    tonk: &TonkState,
    subject: &Did,
) -> Result<String, TonkWorkerError> {
    super::space_directory::held(tonk, subject.as_str()).await?;
    encode_chain(&super::repository::space_root_prefix(tonk, subject).await?)
}

/// [`publish_revocation`], signed under `authority`: this device's chain
/// from the space, however it was found.
async fn publish_revocation_under<R>(
    tonk: &TonkState,
    repo: &str,
    repository: &dialog_repository::Repository<R>,
    authority: &DelegationChain,
    path: &DelegationChain,
    target: &Cid,
) -> Result<RevokeReceipt, TonkWorkerError>
where
    R: dialog_varsig::Principal + Clone,
{
    let artifact = tonk_identity::revocation::mint_delegated_revocation_with_witness(
        tonk.profile.credential().signer().clone(),
        path,
        target,
        authority,
    )
    .await
    .map_err(|error| TonkWorkerError::Forbidden(format!("cannot revoke this grant: {error}")))?;
    tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(|error| match error {
            // The artifact is sound but this device's authority does not
            // reach the grant: a refusal, not a fault.
            tonk_identity::revocation::VerifyError::Unauthorized(_) => {
                TonkWorkerError::Forbidden(format!("this device cannot revoke this grant: {error}"))
            }
            tonk_identity::revocation::VerifyError::Malformed(_) => {
                TonkWorkerError::Internal(format!("revocation preflight failed: {error}"))
            }
        })?;
    // The revocation belongs at the access service the space actually
    // syncs through, which is the remote `main` tracks.
    let endpoint = match resolve_configured_remote_url(tonk, repository).await? {
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
#[handler]
pub async fn list(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<Vec<InvitationSummary>>, TonkWorkerError> {
    // A clone, not a guard: a first-use mount can pull over the network.
    let tonk = state.read().await.clone();
    // The invitations are recorded in the space, which a worker of its own
    // holds and answers for.
    if tonk.spaces_elsewhere() {
        drop(tonk);
        let listed = super::space_reach::ask(
            &repo,
            "GET",
            &format!("/api/repository/{repo}/invites"),
            None,
        )
        .await?;
        return serde_json::from_value(listed).map(Json).map_err(|error| {
            TonkWorkerError::Internal(format!("the space's worker listed no invitations: {error}"))
        });
    }
    super::adopt::mount_on_first_use(&tonk, &repo).await;
    let session = tonk
        .reactor
        .repository(&repo)
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::NotFound(format!("repository not found: {error}")))?;
    let repository = tonk
        .profile
        .space(&repo)
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
                invitation: invitation.this.to_string(),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.target_cid.cmp(&right.target_cid));
    Ok(Json(rows))
}

#[cfg(all(test, not(all(target_arch = "wasm32", target_os = "unknown"))))]
mod tests {
    use axum::extract::{Path, State};
    use axum::{Extension, Json};
    use dialog_repository::RepositoryExt as _;

    use crate::router::AppState;

    /// Pull the member's replica of `key` from the service, the way a sync
    /// tick does.
    async fn pull(member: &AppState, key: &str) -> Result<(), String> {
        let member = member.read().await;
        let branch = member
            .profile
            .space(key)
            .load()
            .perform(&member.operator)
            .await
            .expect("the member's replica loads")
            .branch("main")
            .open()
            .perform(&member.operator)
            .await
            .expect("the member's branch opens");
        branch
            .pull()
            .perform(&member.operator)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// The connected handle of the remote `main` the `key` replica of
    /// `state` pulls from, as a watch of it opens one.
    async fn upstream_of(state: &AppState, key: &str) -> dialog_repository::ConnectedBranch {
        let tonk = state.read().await;
        let session = tonk
            .reactor
            .repository(key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens");
        let pulls = session.handle().pulls();
        let Some(dialog_repository::Upstream::Remote { remote, branch, .. }) = pulls.iter().next()
        else {
            panic!("the replica pulls from a remote");
        };
        remote
            .branch(branch.clone())
            .open()
            .perform(&tonk.operator)
            .await
            .expect("the remote branch opens")
    }

    /// Where the `key` replica of `state` stands.
    async fn head_of(state: &AppState, key: &str) -> Option<dialog_repository::Revision> {
        let tonk = state.read().await;
        tonk.reactor
            .repository(key)
            .branch("main")
            .acquire(&tonk.operator)
            .await
            .expect("the branch opens")
            .handle()
            .revision()
    }

    /// Whether the member's device still retains the certificate `target`.
    async fn retains(member: &AppState, target: &str) -> bool {
        let member = member.read().await;
        member
            .operator
            .state()
            .delegations()
            .retained(target.parse().expect("the target is a CID"))
            .perform(&member.operator)
            .await
            .expect("the retained certificates read")
            .is_some()
    }

    /// A link minted on another device of the same account is revocable
    /// from this one. The invite path ends at the device that signed the
    /// leaf, so it alone never shows this device holding the space; the
    /// revocation has to carry this device's own authority as witness.
    #[dialog_common::test]
    async fn it_revokes_a_link_minted_on_another_device_of_the_account() {
        use dialog_credentials::{Ed25519Signer, Signer};
        use dialog_ucan_core::DelegationBuilder;
        use dialog_ucan_core::subject::Subject;
        use dialog_varsig::Principal as _;

        let (tonk, service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "sibling-revoke-test".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .expect("the root persists");
        let state: AppState = std::sync::Arc::new(crate::StateLock::new(tonk));
        let key = super::super::repository::create_space_inner(&state, "Sibling", None)
            .await
            .expect("the space creates");
        super::super::repository::enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");

        {
            let tonk = state.read().await;
            let session = tonk
                .reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("content branch opens");
            let subject = tonk
                .profile
                .space(&key)
                .load()
                .perform(&tonk.operator)
                .await
                .expect("the space loads")
                .did();

            // The path a sibling device's mint leaves on the shared branch:
            // space -> account -> sibling device -> recipient.
            let sibling = Ed25519Signer::import(&[91; 32])
                .await
                .expect("the sibling key imports");
            let recipient = Ed25519Signer::import(&[92; 32])
                .await
                .expect("the recipient key imports");
            let mut path = super::account_authority(&tonk, session.handle(), &subject)
                .await
                .expect("the account holds the space");
            let device =
                tonk_identity::delegation::mint_device_delegation(root.clone(), &sibling.did())
                    .await
                    .expect("the sibling's device grant mints");
            for hop in device.proofs() {
                path = path.push(hop.clone()).expect("the sibling grant chains");
            }
            let leaf = DelegationBuilder::new()
                .issuer(Signer::from(sibling))
                .audience(&recipient.did())
                .subject(Subject::Specific(subject.clone()))
                .command(vec!["use".to_string()])
                .meta(tonk_invite::inviter_meta(&root.did()))
                .try_build()
                .await
                .expect("the sibling signs the invite");
            let path = path.push(leaf).expect("the invite chains");
            assert!(
                !path
                    .proofs()
                    .any(|hop| hop.audience() == &tonk.profile.did()),
                "this device is not on the sibling's invite path"
            );

            session
                .handle()
                .delegations()
                .retain_all(vec![dialog_ucan::UcanDelegation(path.clone())])
                .perform(&tonk.operator)
                .await
                .expect("the sibling's path is retained");
            let invitation =
                tonk_schema::Invitation::from_chain(&path).expect("the path names the space");
            let execution = tonk_schema::InvitationExecution::new(&invitation, "scoped");
            tonk.reactor
                .repository(&key)
                .branch("main")
                .transaction()
                .assert(invitation)
                .assert(execution)
                .commit()
                .perform(&tonk.operator)
                .await
                .expect("the invitation is recorded");
            session
                .handle()
                .push()
                .perform(&tonk.operator)
                .await
                .expect("the content reaches the service");
        }

        let Json(rows) = super::list(State(state.clone()), Path(key.clone()))
            .await
            .expect("the invitations list");
        assert_eq!(rows.len(), 1, "the sibling's link is listed: {rows:?}");
        let target = rows[0].target_cid.clone();
        let Json(receipt) =
            super::revoke(State(state.clone()), Path((key.clone(), target.clone())))
                .await
                .expect("this device revokes the sibling's link");
        assert!(
            receipt.recorded,
            "the access service records the revocation"
        );
        assert_eq!(receipt.revoked.to_string(), target);
        let Json(rows) = super::list(State(state.clone()), Path(key.clone()))
            .await
            .expect("the invitations list again");
        assert!(rows.is_empty(), "the revoked link is gone: {rows:?}");

        let account_key = {
            let tonk = state.read().await;
            crate::router::account_state::require_ready_account_state(&tonk)
                .await
                .expect("the linked account is ready")
                .key
                .clone()
        };
        let tonk = std::sync::Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("the state has no other holders"))
            .into_inner();
        crate::router::account_state::tests::discard(tonk, &account_key);
        drop(service);
    }

    /// A member whose invite link was revoked is cut off, and a fresh link
    /// lets the same member back in. The refusal of the old chain carries
    /// the signed revocation, so the member's device drops the revoked hop
    /// and proves through the new grant instead of presenting the revoked
    /// chain on every sync.
    #[dialog_common::test]
    async fn it_lets_a_member_cut_off_by_a_revoked_invite_back_in_through_a_new_one() {
        let (tonk, service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "reinvite-test".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .expect("the root persists");
        let state: AppState = std::sync::Arc::new(crate::StateLock::new(tonk));
        let key = super::super::repository::create_space_inner(&state, "Reinvited", None)
            .await
            .expect("the space creates");
        super::super::repository::enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .acquire(&tonk.operator)
                .await
                .expect("content branch opens")
                .handle()
                .push()
                .perform(&tonk.operator)
                .await
                .expect("the content reaches the service");
        }

        let origin = crate::axum::RequestOrigin::parse("https://local.example/")
            .unwrap_or_else(|_| panic!("the origin parses"));
        let mint = || async {
            let Json(response) = super::super::create_invite::create_invite(
                State(state.clone()),
                Path(key.clone()),
                Extension(origin.clone()),
                axum::body::Bytes::new(),
            )
            .await
            .expect("the founder mints an invite");
            response.url().to_string()
        };

        let member = crate::router::command::tests::native::test_state().await;
        let first = mint().await;
        {
            let member = member.read().await;
            let joined = crate::router::join::join_invite(&member, &first)
                .await
                .expect("the member joins");
            assert!(!joined.renewed, "the first join installs a replica");
        }
        pull(&member, &key)
            .await
            .expect("a member reads the space it joined");
        // A local write before the revocation, rewritten after it: the
        // member's device keeps working on its own replica even while the
        // service refuses it.
        let declare = |description: &str| {
            format!(
                "attribute!: &revocation-marker\n  the: xyz.tonk.test/revocation\n  as: text\n  description: {description}\n"
            )
        };
        {
            let member = member.read().await;
            crate::router::evaluate::evaluate_body(
                &member,
                &key,
                "main",
                declare("before revocation"),
                true,
            )
            .await
            .expect("the member writes before the revocation");
            member
                .reactor
                .repository(&key)
                .branch("main")
                .acquire(&member.operator)
                .await
                .expect("the member's branch opens")
                .handle()
                .push()
                .perform(&member.operator)
                .await
                .expect("the member's write reaches the service");
        }

        let Json(rows) = super::list(State(state.clone()), Path(key.clone()))
            .await
            .expect("the invitations list");
        assert_eq!(rows.len(), 1, "one invitation is live: {rows:?}");
        let target = rows[0].target_cid.clone();
        assert!(
            retains(&member, &target).await,
            "the member proves through the invite hop it claimed"
        );
        let Json(receipt) =
            super::revoke(State(state.clone()), Path((key.clone(), target.clone())))
                .await
                .expect("the founder revokes the link");
        assert!(
            receipt.recorded,
            "the access service records the revocation"
        );
        assert!(
            pull(&member, &key).await.is_err(),
            "revoking the link cuts off the member who joined through it"
        );
        // Which retained chain a proof tries first is not fixed, so the
        // re-join below can pass by luck. This is what makes it reliable:
        // the refusal carried the signed revocation, and the member's
        // device dropped the revoked hop rather than keep presenting it.
        assert!(
            !retains(&member, &target).await,
            "the refusal's revocation lets the member drop the revoked hop"
        );
        {
            let member = member.read().await;
            crate::router::evaluate::evaluate_body(
                &member,
                &key,
                "main",
                declare("after revocation"),
                true,
            )
            .await
            .expect("a cut-off member still writes to its own replica");
        }

        let second = mint().await;
        {
            let member = member.read().await;
            let rejoined = crate::router::join::join_invite(&member, &second)
                .await
                .expect("the member claims the new link");
            assert!(rejoined.renewed, "the replica is still here");
        }
        pull(&member, &key)
            .await
            .expect("a member let back in reads the space again");

        let account_key = {
            let tonk = state.read().await;
            crate::router::account_state::require_ready_account_state(&tonk)
                .await
                .expect("the linked account is ready")
                .key
                .clone()
        };
        let tonk = std::sync::Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("the state has no other holders"))
            .into_inner();
        crate::router::account_state::tests::discard(tonk, &account_key);
        drop(service);
    }

    /// A member told of a head after its invite was revoked does not adopt
    /// it. A watch begun before the revocation can deliver the owner's
    /// next head, and a fast-forward adopts by reference without reading
    /// the root; the member then held a head whose root the service no
    /// longer serves it, and every later write failed on that root. The
    /// pull now reads the root first, is refused, and leaves the head
    /// where it was, so the member keeps writing to its own replica.
    #[dialog_common::test]
    async fn it_does_not_adopt_a_head_the_member_can_no_longer_read() {
        let (tonk, service, root, remote) =
            crate::router::account_state::tests::ready_account_state(None).await;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await
                .expect("the device grant mints");
        crate::router::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "unreadable-head-test".to_string(),
                delegation_hex: hex::encode(grant.to_bytes().expect("the grant serializes")),
                passkey: None,
                encryption_key: None,
            },
        )
        .await
        .expect("the root persists");
        let state: AppState = std::sync::Arc::new(crate::StateLock::new(tonk));
        let key = super::super::repository::create_space_inner(&state, "Unreadable", None)
            .await
            .expect("the space creates");
        super::super::repository::enable_sync_inner(&state, &key, &remote)
            .await
            .expect("the remote attaches");

        let origin = crate::axum::RequestOrigin::parse("https://local.example/")
            .unwrap_or_else(|_| panic!("the origin parses"));
        let Json(invited) = super::super::create_invite::create_invite(
            State(state.clone()),
            Path(key.clone()),
            Extension(origin),
            axum::body::Bytes::new(),
        )
        .await
        .expect("the founder mints an invite");
        let push = |state: AppState| {
            let key = key.clone();
            async move {
                let tonk = state.read().await;
                tonk.reactor
                    .repository(&key)
                    .branch("main")
                    .acquire(&tonk.operator)
                    .await
                    .expect("the branch opens")
                    .handle()
                    .push()
                    .perform(&tonk.operator)
                    .await
                    .expect("the branch reaches the service");
            }
        };
        push(state.clone()).await;

        let member = crate::router::command::tests::native::test_state().await;
        {
            let member = member.read().await;
            crate::router::join::join_invite(&member, invited.url().as_str())
                .await
                .expect("the member joins");
        }
        pull(&member, &key)
            .await
            .expect("a member reads the space it joined");
        let declare = |description: &str| {
            format!(
                "attribute!: &revocation-marker\n  the: xyz.tonk.test/revocation\n  as: text\n  description: {description}\n"
            )
        };
        {
            let member = member.read().await;
            crate::router::evaluate::evaluate_body(
                &member,
                &key,
                "main",
                declare("before revocation"),
                true,
            )
            .await
            .expect("the member writes before the revocation");
        }
        push(member.clone()).await;
        {
            // The founder builds on the member's head, so the head it
            // pushes after revoking is a fast-forward for the member.
            let tonk = state.read().await;
            tonk.reactor
                .repository(&key)
                .branch("main")
                .pull()
                .perform(&tonk.operator)
                .await
                .expect("the founder pulls the member's write");
        }

        let Json(rows) = super::list(State(state.clone()), Path(key.clone()))
            .await
            .expect("the invitations list");
        let target = rows
            .first()
            .map(|row| row.target_cid.clone())
            .expect("one invitation is live");
        let Json(receipt) = super::revoke(State(state.clone()), Path((key.clone(), target)))
            .await
            .expect("the founder revokes the link");
        assert!(
            receipt.recorded,
            "the access service records the revocation"
        );
        push(state.clone()).await;

        // What a watch begun before the revocation delivers: the head the
        // founder just pushed, recorded on the member's side.
        let founder_head = head_of(&state, &key).await;
        let pushed = upstream_of(&state, &key)
            .await
            .cache()
            .content()
            .expect("the founder's push recorded the remote head");
        assert_eq!(Some(&pushed.content), founder_head.as_ref());
        let watched = upstream_of(&member, &key).await;
        let observed = {
            let member = member.read().await;
            watched
                .observe(pushed)
                .perform(&member.operator)
                .await
                .expect("the delivered head is recorded")
        };
        assert!(
            matches!(observed, dialog_repository::Observation::Advanced(_)),
            "the founder's head is ahead of the one the member knew: {observed:?}"
        );

        let before = head_of(&member, &key).await;
        assert_ne!(before, founder_head, "the member is behind the founder");
        let pulled = {
            let member = member.read().await;
            member
                .reactor
                .repository(&key)
                .branch("main")
                .pull()
                .assuming_upstream()
                .perform(&member.operator)
                .await
        };
        assert!(
            matches!(
                pulled,
                Err(crate::reactor::ReactorError::UnreadableHead { .. })
            ),
            "a head whose root the member cannot read is refused: {pulled:?}"
        );
        assert_eq!(
            head_of(&member, &key).await,
            before,
            "the refused pull leaves the member's head where it was"
        );
        {
            let member = member.read().await;
            crate::router::evaluate::evaluate_body(
                &member,
                &key,
                "main",
                declare("after revocation"),
                true,
            )
            .await
            .expect("a cut-off member still writes to its own replica");
        }

        let account_key = {
            let tonk = state.read().await;
            crate::router::account_state::require_ready_account_state(&tonk)
                .await
                .expect("the linked account is ready")
                .key
                .clone()
        };
        drop(member);
        let tonk = std::sync::Arc::try_unwrap(state)
            .unwrap_or_else(|_| panic!("the state has no other holders"))
            .into_inner();
        crate::router::account_state::tests::discard(tonk, &account_key);
        drop(service);
    }
}
