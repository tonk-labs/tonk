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
use super::create_invite::{ConfiguredRemoteRequirement, resolve_configured_remote_url};
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
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
    let artifact = tonk_identity::revocation::mint_delegated_revocation(
        tonk.profile.credential().signer().clone(),
        path,
        target,
        authority,
    )
    .await
    .map_err(|error| TonkWorkerError::Forbidden(format!("cannot revoke this grant: {error}")))?;
    tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(|error| {
            TonkWorkerError::Internal(format!("revocation preflight failed: {error}"))
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
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
    Path(repo): Path<String>,
) -> Result<Json<Vec<InvitationSummary>>, TonkWorkerError> {
    let tonk = state.read().await;
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
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.target_cid.cmp(&right.target_cid));
    Ok(Json(rows))
}

/// What a space's own worker does where it has none of the person's
/// authority: ask the person's profile, up the port its script holds.
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod site_tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_repository::RepositoryExt as _;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use dialog_varsig::{Did, Principal as _};
    use js_sys::{Array, Function, Reflect};
    use tonk_account::customer::RevokeReceipt;
    use wasm_bindgen::JsValue;

    use super::{account_authority, encode_chain, leaf_cid, revoke_through_profile};
    use crate::helpers::state::test_state_for_site;
    use crate::router::account::act_for;
    use crate::router::join::mount_replica;
    use crate::worker::TonkState;

    /// A space's own worker holding a space it was told is `account`'s, the
    /// space's content branch, and the `/` chain from the space to that
    /// account, which nothing has retained in the space.
    async fn held_space() -> (TonkState, Did, dialog_repository::Branch, DelegationChain) {
        let worker = test_state_for_site().await;
        let space = Ed25519Signer::generate().await.unwrap();
        let subject = space.did();
        let account = Ed25519Signer::generate().await.unwrap().did();
        let repository = mount_replica(&worker, &subject, None, None).await.unwrap();
        let branch = repository
            .branch("main")
            .open()
            .perform(&worker.operator)
            .await
            .unwrap();
        act_for(&worker, &account).await.unwrap();
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(space))
            .audience(&account)
            .subject(UcanSubject::Specific(subject.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        (worker, subject, branch, DelegationChain::new(grant))
    }

    /// Stand in for the person's profile: answer what this worker asks up
    /// its port with `answer` (a JSON object), keeping what was asked.
    fn answer_as_the_profile(answer: &str) {
        let hook = Function::new_with_args(
            "request",
            &format!(
                r#"
                (globalThis.tonkAskedProfile ??= []).push(JSON.stringify(request));
                return Promise.resolve({answer});
                "#
            ),
        );
        Reflect::set(&js_sys::global(), &"tonkAskProfile".into(), &hook).unwrap();
        Reflect::set(&js_sys::global(), &"tonkAskedProfile".into(), &Array::new()).unwrap();
    }

    /// What the profile was asked, and the stand-in taken away.
    fn asked_of_the_profile() -> Vec<serde_json::Value> {
        let asked = Reflect::get(&js_sys::global(), &"tonkAskedProfile".into()).unwrap();
        Reflect::set(
            &js_sys::global(),
            &"tonkAskProfile".into(),
            &JsValue::UNDEFINED,
        )
        .unwrap();
        Array::from(&asked)
            .iter()
            .filter_map(|entry| entry.as_string())
            .map(|entry| serde_json::from_str(&entry).unwrap())
            .collect()
    }

    #[dialog_common::test]
    async fn it_asks_the_profile_for_the_authority_the_spaces_chains_do_not_prove() {
        let (worker, subject, branch, prefix) = held_space().await;
        let encoded = encode_chain(&prefix).unwrap();
        answer_as_the_profile(&serde_json::json!({ "authority": encoded }).to_string());

        let authority = account_authority(&worker, &branch, &subject).await;
        let asked = asked_of_the_profile();

        assert_eq!(
            encode_chain(&authority.expect("the profile's record stands in")).unwrap(),
            encoded
        );
        assert_eq!(asked, [serde_json::json!({ "authority": true })]);
    }

    #[dialog_common::test]
    async fn it_has_the_profile_sign_the_revocation_of_a_grant_it_found() {
        let (worker, subject, branch, path) = held_space().await;
        let target = leaf_cid(&path).unwrap();
        let receipt = RevokeReceipt {
            revoked: target,
            subject: subject.clone(),
            recorded: true,
        };
        answer_as_the_profile(&serde_json::json!({ "receipt": receipt }).to_string());

        let revoked = revoke_through_profile(&worker, &branch, &subject, &path, &target).await;
        let asked = asked_of_the_profile();

        assert_eq!(revoked.expect("the profile signs"), receipt);
        assert_eq!(
            asked,
            [serde_json::json!({
                "revoke": {
                    "path": encode_chain(&path).unwrap(),
                    "target": target.to_string(),
                }
            })],
            "it sends the path and the grant, and no authority it could not prove"
        );
    }
}
