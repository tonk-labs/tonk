//! Account-managed ordinary grants. Invitation secrets leave only through transient UI.
use super::{
    AppState,
    create_invite::{RemoteRequirement, generate_ephemeral, resolve_remote_url},
};
use crate::{TonkState, TonkWorkerError, axum::RequestOrigin};
use axum::{
    Json,
    extract::{Path, State},
};
use axum_wasm_macros::wasm_compat;
use dialog_capability::{
    Subject,
    access::{Access, Prove},
};
use dialog_credentials::Signer;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_ucan::Scope;
use dialog_ucan::{Ucan, UcanDelegation};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, time::Timestamp};
use dialog_varsig::{Did, Principal};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_invite::connection::{
    AgentInvite, DEFAULT_GRANT_TTL_SECONDS, SpaceGrantBundle, candidate_build_scopes, grant_set_id,
    require_grant_deadline,
};
use tonk_schema::{AgentGrantGroup, AgentGrantRevocation, agent_connection as fields};
use tonk_worker_api::{
    AgentConnectionInviteResponse, AgentConnectionSummary, AgentConnectionTarget,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal_request: Option<String>,
    version: u8,
    id: String,
    account: String,
    repo: String,
    subject: String,
    recipient: String,
    label: String,
    remote: String,
    issued_at: u64,
    chains: Vec<String>,
}
fn failure(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Internal(format!("agent connection: {error}"))
}
fn enabled() -> Result<(), TonkWorkerError> {
    if cfg!(feature = "connection-invites") {
        Ok(())
    } else {
        Err(TonkWorkerError::NotFound(
            "agent invitations are not enabled".into(),
        ))
    }
}
fn group_entity(id: &str) -> Result<dialog_artifacts::Entity, TonkWorkerError> {
    format!("id:tonk:agent-grant:{id}").parse().map_err(failure)
}
fn receipt_entity(id: &str, cid: &str) -> Result<dialog_artifacts::Entity, TonkWorkerError> {
    format!("id:tonk:agent-revocation:{id}:{cid}")
        .parse()
        .map_err(failure)
}
fn cids(bundle: &SpaceGrantBundle) -> Vec<String> {
    let mut ids: Vec<_> = bundle
        .chains()
        .iter()
        .map(|chain| chain.proof_cids().last().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}
async fn validate(group: &PublicGroup) -> Result<SpaceGrantBundle, TonkWorkerError> {
    if group.version != 1 {
        return Err(failure("unsupported public group"));
    }
    let subject: Did = group.subject.parse().map_err(failure)?;
    let recipient: Did = group.recipient.parse().map_err(failure)?;
    let chains = group
        .chains
        .iter()
        .map(|bytes| {
            let bytes = hex::decode(bytes).map_err(failure)?;
            DelegationChain::try_from(bytes.as_slice()).map_err(failure)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let bundle = SpaceGrantBundle::validate(
        chains,
        &recipient,
        &candidate_build_scopes(&subject),
        &group.remote.parse().map_err(failure)?,
        Timestamp::try_from(group.issued_at as i128).map_err(failure)?,
    )
    .await
    .map_err(failure)?;
    if grant_set_id(&group.subject, &group.recipient, &cids(&bundle)) != group.id {
        return Err(failure("public group does not match its signed targets"));
    }
    Ok(bundle)
}

async fn issue(
    seed: [u8; 32],
    signer: Signer,
    ancestors: Vec<DelegationChain>,
    scopes: &[Scope],
    remote: &url::Url,
    now: Timestamp,
    expires: Timestamp,
) -> Result<AgentInvite, TonkWorkerError> {
    let recipient = dialog_credentials::Ed25519Signer::import(&seed)
        .await
        .map_err(failure)?
        .did();
    let bundle =
        issue_to_recipient(&recipient, signer, ancestors, scopes, remote, now, expires).await?;
    AgentInvite::new(seed, bundle.chains().to_vec(), scopes, remote, now)
        .await
        .map_err(failure)
}

/// The browser delegates to a terminal's exact public key; it never receives or
/// generates that terminal's private key. The same leaf builder serves Flow A.
async fn issue_to_recipient(
    recipient: &Did,
    signer: Signer,
    ancestors: Vec<DelegationChain>,
    scopes: &[Scope],
    remote: &url::Url,
    now: Timestamp,
    expires: Timestamp,
) -> Result<SpaceGrantBundle, TonkWorkerError> {
    require_grant_deadline(&ancestors, expires)
        .map_err(|error| TonkWorkerError::Forbidden(error.to_string()))?;
    if ancestors.len() != scopes.len() {
        return Err(failure("issuer proof count mismatch"));
    }
    let mut chains = Vec::new();
    for (chain, scope) in ancestors.into_iter().zip(scopes) {
        let leaf = DelegationBuilder::new()
            .issuer(signer.clone())
            .audience(recipient)
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(expires)
            .meta(tonk_invite::home_address_meta(remote))
            .try_build()
            .await
            .map_err(failure)?;
        chains.push(chain.push(leaf).map_err(failure)?);
    }
    SpaceGrantBundle::validate(chains, recipient, scopes, remote, now)
        .await
        .map_err(failure)
}

/// Keep a conclusive permission refusal distinct from an unavailable decision.
fn issuer_proof_failure(error: dialog_capability::access::AuthorizeError) -> TonkWorkerError {
    use dialog_capability::access::{AuthorizeError, Recourse};
    match error {
        AuthorizeError::UnavailableProof { .. }
        | AuthorizeError::Unavailable { .. }
        | AuthorizeError::Malformed { .. }
        | AuthorizeError::Declined {
            recourse: Recourse::Retry,
            ..
        } => failure(error),
        AuthorizeError::UnprovenSubject { .. }
        | AuthorizeError::CommandEscalation { .. }
        | AuthorizeError::PolicyViolation { .. }
        | AuthorizeError::InvalidAudience { .. }
        | AuthorizeError::NotValidBefore { .. }
        | AuthorizeError::Expired { .. }
        | AuthorizeError::Revoked { .. }
        | AuthorizeError::InvalidSignature { .. }
        | AuthorizeError::Declined {
            recourse: Recourse::None,
            ..
        } => {
            tonk_common::log!("invitation issuer permission refused: {error}");
            TonkWorkerError::Forbidden(
                "this account cannot grant the requested space access".into(),
            )
        }
    }
}

async fn issuer_ancestors(
    tonk: &TonkState,
    scopes: &[Scope],
    now: Timestamp,
    expires: Timestamp,
) -> Result<Vec<DelegationChain>, TonkWorkerError> {
    let mut ancestors = Vec::new();
    for scope in scopes {
        let mut requested = Prove::<Ucan>::new(tonk.profile.did(), scope.clone());
        requested.duration = dialog_capability::access::TimeRange {
            not_before: Some(now.to_unix()),
            expiration: Some(expires.to_unix()),
        };
        let proof = match Subject::from(tonk.profile.did())
            .attenuate(Access)
            .invoke(requested)
            .perform(&tonk.operator)
            .await
        {
            Ok(proof) => proof,
            // Recover the present chain only to expose its actual limiting
            // deadline; issue() refuses to silently shorten the requested term.
            Err(_) => Subject::from(tonk.profile.did())
                .attenuate(Access)
                .invoke(Prove::<Ucan>::new(tonk.profile.did(), scope.clone()))
                .perform(&tonk.operator)
                .await
                .map_err(issuer_proof_failure)?,
        };
        let mut certificates = proof.proofs.into_iter();
        let first = certificates
            .next()
            .ok_or_else(|| failure("issuer proof is empty"))?;
        let mut chain = DelegationChain::new(first.0);
        for certificate in certificates {
            chain = chain.push(certificate.0).map_err(failure)?;
        }
        ancestors.push(chain);
    }
    require_grant_deadline(&ancestors, expires)
        .map_err(|error| TonkWorkerError::Forbidden(error.to_string()))?;
    Ok(ancestors)
}

/// Issue one independent bearer. A state read guard pins account and space until
/// all public records are durable; the returned URL is never stored.
pub(crate) async fn mint(
    state: AppState,
    repo: String,
    origin: RequestOrigin,
) -> Result<AgentConnectionInviteResponse, TonkWorkerError> {
    enabled()?;
    let tonk = state.read().await;
    if super::account::provider(&tonk).await.is_none() {
        return Err(TonkWorkerError::Forbidden(
            "sign in before creating an agent invitation".into(),
        ));
    }
    let root = super::identity::local_root(&tonk).await?;
    let repository = tonk
        .profile
        .repository(&repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    let subject = repository.did();
    let remote = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => {
            return Err(TonkWorkerError::Conflict(
                "space needs a configured sync remote".into(),
            ));
        }
    };
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    if super::repository::remote_is_own_service(remote.as_str())
        && !super::customer::space_provider_recorded(&tonk, &subject).await
    {
        match super::repository::provision_space_consumer(&tonk, &subject).await {
            Ok(()) => {}
            Err(error @ TonkWorkerError::Upstream { .. })
                if !super::customer::is_retryable(&error) =>
            {
                return Err(error);
            }
            // Shared-space members can lack the owner's billing credential.
            // Only a terminal service response establishes refusal.
            Err(error) => tonk_common::log!("agent invitation provisioning skipped: {error}"),
        }
    }
    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(failure)?;
    let (_, seed) = generate_ephemeral().await?;
    let recipient = dialog_credentials::Ed25519Signer::import(&seed)
        .await
        .map_err(failure)?
        .did();
    let scopes = candidate_build_scopes(&subject);
    let ancestors = issuer_ancestors(&tonk, &scopes, now, expires).await?;
    let invite = issue(
        seed,
        tonk.profile.signer().signer().clone(),
        ancestors,
        &scopes,
        &remote,
        now,
        expires,
    )
    .await?;
    let group = PublicGroup {
        terminal_request: None,
        version: 1,
        id: grant_set_id(subject.as_str(), recipient.as_str(), &cids(invite.grants())),
        account: root.root_did.to_string(),
        repo: repo.clone(),
        subject: subject.to_string(),
        recipient: recipient.to_string(),
        label: format!(
            "{} · {}",
            super::repository::repository_display_name(&tonk, &repository, &repo)
                .await
                .unwrap_or_else(|| repo.clone()),
            now.to_unix()
        ),
        remote: remote.to_string(),
        issued_at: now.to_unix(),
        chains: invite
            .grants()
            .chains()
            .iter()
            .map(|chain| chain.to_bytes().map(hex::encode).map_err(failure))
            .collect::<Result<_, _>>()?,
    };
    let account = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    account
        .handle()
        .delegations()
        .retain_all(
            invite
                .grants()
                .chains()
                .iter()
                .cloned()
                .map(UcanDelegation)
                .collect::<Vec<_>>(),
        )
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction()
        .assert(AgentGrantGroup {
            this: group_entity(&group.id)?,
            account: fields::Account(group.account.clone()),
            subject: fields::Subject(group.subject.clone()),
            public_record: fields::PublicRecord(serde_json::to_string(&group).map_err(failure)?),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    let url = invite
        .to_url(origin.url().join("join").map_err(failure)?.as_str())
        .map_err(failure)?;
    let connection = summarize(&tonk, &group).await?;
    Ok(AgentConnectionInviteResponse { url, connection })
}

/// A lost transient bearer requires an explicit new invitation, even after
/// worker restart. Historical public records remain sufficient to detect it.
pub(crate) async fn has_issued_for_subject(
    tonk: &TonkState,
    subject: &Did,
) -> Result<bool, TonkWorkerError> {
    Ok(groups(tonk)
        .await?
        .iter()
        .any(|group| group.terminal_request.is_none() && group.subject == subject.as_str()))
}

async fn groups(tonk: &TonkState) -> Result<Vec<PublicGroup>, TonkWorkerError> {
    let root = super::identity::local_root(tonk).await?;
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<AgentGrantGroup> = session
        .handle()
        .query()
        .select(Query::<AgentGrantGroup> {
            this: Term::var("this"),
            account: Term::from(fields::Account(root.root_did.to_string())),
            subject: Term::var("subject"),
            public_record: Term::var("public_record"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let mut groups = Vec::new();
    for row in rows {
        if row.public_record.0.len() > 2 * 1024 * 1024 {
            return Err(failure("public group exceeds size limit"));
        }
        let group: PublicGroup = serde_json::from_str(&row.public_record.0).map_err(failure)?;
        if group.account != root.root_did.as_str()
            || group.subject != row.subject.0
            || row.this != group_entity(&group.id)?
        {
            return Err(failure("public group account or subject mismatch"));
        }
        validate(&group).await?;
        groups.push(group);
    }
    Ok(groups)
}
async fn summarize(
    tonk: &TonkState,
    group: &PublicGroup,
) -> Result<AgentConnectionSummary, TonkWorkerError> {
    let bundle = validate(group).await?;
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let mut targets = Vec::new();
    for cid in cids(&bundle) {
        let receipts: Vec<AgentGrantRevocation> = session
            .handle()
            .query()
            .select(Query::<AgentGrantRevocation> {
                this: Term::from(receipt_entity(&group.id, &cid)?),
                target: Term::from(fields::Target(cid.clone())),
                receipt: Term::var("receipt"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .map_err(failure)?;
        let acknowledged = receipts.iter().any(|row| {
            serde_json::from_str::<tonk_account::customer::RevokeReceipt>(&row.receipt.0)
                .is_ok_and(|receipt| receipt.revoked.to_string() == cid)
        });
        targets.push(AgentConnectionTarget {
            cid,
            acknowledged,
            error: None,
        });
    }
    let intents: Vec<fields::AgentGrantRevocationIntent> = session
        .handle()
        .query()
        .select(Query::<fields::AgentGrantRevocationIntent> {
            this: Term::from(group_entity(&group.id)?),
            requested_at: Term::var("requested_at"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let acknowledged = targets.iter().filter(|target| target.acknowledged).count();
    let status = if acknowledged == targets.len() {
        "revoked"
    } else if acknowledged > 0 || !intents.is_empty() {
        "partial"
    } else if bundle.expires_at().to_unix() <= Timestamp::now().to_unix() {
        "expired"
    } else {
        "active"
    };
    let confirmed = confirmation(tonk, group).await?;
    Ok(AgentConnectionSummary {
        kind: group.terminal_request.as_ref().map(|_| "terminal".into()),
        request_id: group.terminal_request.clone(),
        id: group.id.clone(),
        repo: group.repo.clone(),
        subject: group.subject.clone(),
        recipient: group.recipient.clone(),
        label: group.label.clone(),
        scope: "Build space data and views (main)".into(),
        expires_at: bundle.expires_at().to_unix(),
        status: status.into(),
        confirmed,
        targets,
    })
}

async fn confirmation(tonk: &TonkState, group: &PublicGroup) -> Result<bool, TonkWorkerError> {
    use fields::AgentConnectionConfirmation;
    let repository = match tonk
        .profile
        .repository(&group.repo)
        .load()
        .perform(&tonk.operator)
        .await
    {
        Ok(repository) => repository,
        Err(_) => return Ok(false),
    };
    if repository.did().as_str() != group.subject {
        return Ok(false);
    }
    let session = tonk
        .reactor
        .repository(&group.repo)
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let entity: dialog_artifacts::Entity = format!("id:tonk:agent-connection:{}", group.id)
        .parse()
        .map_err(failure)?;
    let rows: Vec<AgentConnectionConfirmation> = session
        .handle()
        .query()
        .select(Query::<AgentConnectionConfirmation> {
            this: Term::from(entity),
            status: Term::from(fields::Status("Agent connection confirmed".into())),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    Ok(!rows.is_empty())
}

#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentConnectionSummary>>, TonkWorkerError> {
    enabled()?;
    let tonk = state.read().await;
    let mut result = Vec::new();
    for group in groups(&tonk).await? {
        result.push(summarize(&tonk, &group).await?);
    }
    Ok(Json(result))
}

// The ordinary invitation publisher requires `/` over the space. A member
// holding `/use` can withdraw their own issued leaf without gaining that right:
// sign as its issuer, or delegate the account's withdrawal of its witnessed path.
async fn publish_target(
    tonk: &TonkState,
    group: &PublicGroup,
    path: &DelegationChain,
    target: &ipld_core::cid::Cid,
) -> Result<tonk_account::customer::RevokeReceipt, TonkWorkerError> {
    let signer = tonk.profile.signer().signer().clone();
    let artifact = if path
        .proofs()
        .last()
        .is_some_and(|leaf| leaf.issuer() == &tonk.profile.did())
    {
        tonk_identity::revocation::mint_root_revocation(signer, path, target)
            .await
            .map_err(failure)?
    } else {
        let root = super::identity::local_root(tonk).await?;
        tonk_identity::revocation::mint_delegated_revocation_with_witness(
            signer,
            path,
            target,
            &root.delegation,
        )
        .await
        .map_err(failure)?
    };
    tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(failure)?;
    let response =
        super::http::post_cbor(&group.remote.parse().map_err(failure)?, &artifact).await?;
    let receipt: tonk_account::customer::RevokeReceipt =
        serde_json::from_slice(&response.body).map_err(failure)?;
    let expected = tonk_identity::revocation::verify(&artifact)
        .await
        .map_err(failure)?;
    if receipt.revoked != *target || receipt.subject != expected.subject {
        return Err(failure(
            "access service acknowledged a different revocation",
        ));
    }
    Ok(receipt)
}

#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AgentConnectionSummary>, TonkWorkerError> {
    enabled()?;
    let tonk = state.read().await;
    let group = groups(&tonk)
        .await?
        .into_iter()
        .find(|group| group.id == id)
        .ok_or_else(|| {
            TonkWorkerError::NotFound("agent invitation not found in this account".into())
        })?;
    let repository = tonk
        .profile
        .repository(&group.repo)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    if repository.did().as_str() != group.subject {
        return Err(failure("space routing changed"));
    }
    let result = revoke_group(&tonk, &group, |chain, cid| {
        let tonk = &*tonk;
        let group = &group;
        async move { publish_target(tonk, group, &chain, &cid).await }
    })
    .await?;
    Ok(Json(result))
}

async fn revoke_group<F, Fut>(
    tonk: &TonkState,
    group: &PublicGroup,
    mut publish: F,
) -> Result<AgentConnectionSummary, TonkWorkerError>
where
    F: FnMut(DelegationChain, ipld_core::cid::Cid) -> Fut,
    Fut: std::future::Future<Output = Result<tonk_account::customer::RevokeReceipt, TonkWorkerError>>,
{
    let bundle = validate(group).await?;
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction()
        .assert(fields::AgentGrantRevocationIntent {
            this: group_entity(&group.id)?,
            requested_at: fields::RequestedAt(Timestamp::now().to_unix()),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;

    let mut result = summarize(tonk, group).await?;
    for target in &mut result.targets {
        if target.acknowledged {
            continue;
        }
        let cid = target.cid.parse().map_err(failure)?;
        let chain = bundle
            .chains()
            .iter()
            .find(|chain| chain.proof_cids().last() == Some(&cid))
            .unwrap();
        match publish(chain.clone(), cid).await {
            Ok(receipt) => {
                // Save each acknowledgement before sending the next revocation.
                tonk.reactor
                    .profile_repository()
                    .branch(&tonk.active_branch)
                    .transaction()
                    .assert(AgentGrantRevocation {
                        this: receipt_entity(&group.id, &target.cid)?,
                        target: fields::Target(target.cid.clone()),
                        receipt: fields::Receipt(serde_json::to_string(&receipt).map_err(failure)?),
                    })
                    .commit()
                    .perform(&tonk.operator)
                    .await
                    .map_err(failure)?;
                target.acknowledged = true;
            }
            Err(_) => {
                target.error = Some("Revocation delivery failed; retry this invitation.".into())
            }
        }
    }
    result.status = if result.targets.iter().all(|target| target.acknowledged) {
        "revoked"
    } else {
        "partial"
    }
    .into();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;

    async fn fixture(seconds: u64) -> (Signer, Vec<DelegationChain>, Vec<Scope>, Timestamp) {
        let owner = Ed25519Signer::import(&[51; 32]).await.unwrap();
        let issuer = Ed25519Signer::import(&[52; 32]).await.unwrap();
        let now = Timestamp::now();
        let expires = Timestamp::try_from((now.to_unix() + seconds) as i128).unwrap();
        let ancestor = DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(&issuer.did())
            .subject(dialog_ucan_core::subject::Subject::Specific(owner.did()))
            .command(vec!["use".into()])
            .expiration(expires)
            .try_build()
            .await
            .unwrap();
        (
            Signer::from(issuer),
            vec![DelegationChain::new(ancestor); 6],
            candidate_build_scopes(&owner.did()),
            now,
        )
    }
    #[test]
    fn connection_issuer_classifies_permission_refusal_without_hiding_retry() {
        use dialog_capability::access::{AuthorizeError, Recourse};
        for error in [
            AuthorizeError::UnavailableProof {
                link: "missing-proof".into(),
            },
            AuthorizeError::Unavailable {
                detail: "storage unavailable".into(),
            },
            AuthorizeError::Malformed {
                detail: "undecodable proof".into(),
            },
            AuthorizeError::Declined {
                recourse: Recourse::Retry,
                reason: "activation pending".into(),
            },
        ] {
            assert!(matches!(
                issuer_proof_failure(error),
                TonkWorkerError::Internal(_)
            ));
        }
        let did: Did = "did:key:z6MkrCD1csqtgdj8sRHYRPGLYcMFXAoDhkgvHNq2FML2xqCX"
            .parse()
            .unwrap();
        for error in [
            AuthorizeError::UnprovenSubject {
                claimed: did.clone(),
                authorized: did.clone(),
            },
            AuthorizeError::CommandEscalation {
                claimed: "/use".into(),
                authorized: "/use/get".into(),
            },
            AuthorizeError::PolicyViolation {
                predicate: "main only".into(),
            },
            AuthorizeError::InvalidAudience {
                claimed: did.clone(),
                authorized: did.clone(),
            },
            AuthorizeError::NotValidBefore {
                not_before: 20,
                at: 10,
            },
            AuthorizeError::Expired {
                expiration: 10,
                at: 20,
            },
            AuthorizeError::Revoked {
                subject: did.clone(),
            },
            AuthorizeError::InvalidSignature { issuer: did },
            AuthorizeError::Declined {
                recourse: Recourse::None,
                reason: "permission refused".into(),
            },
        ] {
            assert!(matches!(
                issuer_proof_failure(error),
                TonkWorkerError::Forbidden(_)
            ));
        }
    }

    #[dialog_common::test]
    async fn connection_issuer_mints_independent_bounded_six_leaf_bearers() {
        let (signer, ancestors, scopes, now) = fixture(DEFAULT_GRANT_TTL_SECONDS + 60).await;
        let deadline =
            Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128).unwrap();
        let remote = "https://sync.example.test/ucan/".parse().unwrap();
        let first = issue(
            [53; 32],
            signer.clone(),
            ancestors.clone(),
            &scopes,
            &remote,
            now,
            deadline,
        )
        .await
        .unwrap();
        let second = issue([54; 32], signer, ancestors, &scopes, &remote, now, deadline)
            .await
            .unwrap();
        assert_eq!(first.grants().chains().len(), 6);
        assert_eq!(first.grants().expires_at(), deadline);
        assert_ne!(first.grants().recipient(), second.grants().recipient());
        let url = first.to_url("https://tonk.example/join").unwrap();
        let parsed = AgentInvite::parse_url(&url, &scopes, &remote, now)
            .await
            .unwrap();
        assert_eq!(cids(parsed.grants()), cids(first.grants()));
        assert!(!format!("{first:?}").contains(&url));
    }
    #[dialog_common::test]
    async fn connection_issuer_reports_real_ancestor_deadline_without_shortening() {
        let (signer, ancestors, scopes, now) = fixture(3600).await;
        let limit = ancestors[0].expiration().unwrap().to_unix().to_string();
        let deadline =
            Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128).unwrap();
        let error = issue(
            [53; 32],
            signer,
            ancestors,
            &scopes,
            &"https://sync.example.test/ucan/".parse().unwrap(),
            now,
            deadline,
        )
        .await
        .unwrap_err();
        assert!(matches!(&error, TonkWorkerError::Forbidden(_)), "{error}");
        assert!(error.to_string().contains(&limit), "{error}");
    }
    /// The invitation ledger lives on the branch the profile is on. A
    /// profile that added a second account is on `main-N`, and a ledger
    /// pinned to `main` would retain the new account's grants into the
    /// first account's branch and commit through an upstream its device
    /// holds no authority for.
    #[cfg(not(target_arch = "wasm32"))]
    #[dialog_common::test]
    async fn it_keeps_the_invitation_ledger_on_the_branch_the_profile_is_on() -> anyhow::Result<()>
    {
        use dialog_effects::storage::Directory;
        use dialog_operator::Profile;
        use dialog_storage::provider::storage::Storage;
        let directory =
            std::env::temp_dir().join(format!("tonk-connection-branch-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let storage = Storage::default();
        let profile = Profile::open("ledger-branch")
            .at(location.clone())
            .perform(&storage)
            .await?;
        let registry = crate::device::Registry {
            profile: "ledger-branch".into(),
            directory: location.clone(),
        };
        let first =
            crate::worker::boot_state(storage, "ledger-branch".into(), profile, registry).await?;
        // Onto a fresh branch, the way add-account lands, and booted the
        // way the worker lands there.
        super::super::profile::leave_account(&first).await;
        let tonk = crate::worker::boot_state_with_profile_library(
            first.storage.clone(),
            first.profile_name.clone(),
            first.profile.clone(),
            first.registry.clone(),
            first.profile_library.clone(),
        )
        .await?;
        drop(first);
        assert_ne!(
            tonk.active_branch, "main",
            "the profile moved onto a branch"
        );

        let root = Ed25519Signer::import(&[57; 32]).await?;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await?;
        super::super::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "ledger-branch-fixture".into(),
                delegation_hex: hex::encode(grant.to_bytes()?),
                passkey: None,
                encryption_key: None,
            },
        )
        .await?;
        let (signer, ancestors, scopes, now) = fixture(DEFAULT_GRANT_TTL_SECONDS + 60).await;
        let deadline = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
        let remote = "https://sync.example.test/ucan/".parse()?;
        let invite = issue([58; 32], signer, ancestors, &scopes, &remote, now, deadline).await?;
        let group = PublicGroup {
            terminal_request: None,
            version: 1,
            id: grant_set_id(
                invite.grants().subject().as_str(),
                invite.grants().recipient().as_str(),
                &cids(invite.grants()),
            ),
            account: root.did().to_string(),
            repo: "unmounted".into(),
            subject: invite.grants().subject().to_string(),
            recipient: invite.grants().recipient().to_string(),
            label: "branch fixture".into(),
            remote: remote.to_string(),
            issued_at: now.to_unix(),
            chains: invite
                .grants()
                .chains()
                .iter()
                .map(|chain| chain.to_bytes().map(hex::encode))
                .collect::<Result<_, _>>()?,
        };
        tonk.reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .transaction()
            .assert(AgentGrantGroup {
                this: group_entity(&group.id)?,
                account: fields::Account(group.account.clone()),
                subject: fields::Subject(group.subject.clone()),
                public_record: fields::PublicRecord(serde_json::to_string(&group)?),
            })
            .commit()
            .perform(&tonk.operator)
            .await?;
        assert_eq!(
            groups(&tonk).await?.len(),
            1,
            "the ledger reads the branch it is on"
        );
        assert!(has_issued_for_subject(&tonk, invite.grants().subject()).await?);
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn connection_management_partial_receipts_survive_restart_and_retry_only_missing()
    -> anyhow::Result<()> {
        use dialog_effects::storage::Directory;
        use dialog_operator::Profile;
        use dialog_storage::provider::storage::Storage;
        let directory =
            std::env::temp_dir().join(format!("tonk-connection-ledger-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let storage = Storage::default();
        let profile = Profile::open("ledger")
            .at(location.clone())
            .perform(&storage)
            .await?;
        let registry = crate::device::Registry {
            profile: "ledger".into(),
            directory: location.clone(),
        };
        let tonk = crate::worker::boot_state(storage, "ledger".into(), profile, registry).await?;
        let root = Ed25519Signer::import(&[52; 32]).await?;
        let grant =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await?;
        super::super::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "ledger-fixture".into(),
                delegation_hex: hex::encode(grant.to_bytes()?),
                passkey: None,
                encryption_key: None,
            },
        )
        .await?;
        let (signer, ancestors, scopes, now) = fixture(DEFAULT_GRANT_TTL_SECONDS + 60).await;
        let deadline = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
        let remote = "https://sync.example.test/ucan/".parse()?;
        let invite = issue([53; 32], signer, ancestors, &scopes, &remote, now, deadline).await?;
        let group = PublicGroup {
            terminal_request: None,
            version: 1,
            id: grant_set_id(
                invite.grants().subject().as_str(),
                invite.grants().recipient().as_str(),
                &cids(invite.grants()),
            ),
            account: root.did().to_string(),
            repo: "unmounted".into(),
            subject: invite.grants().subject().to_string(),
            recipient: invite.grants().recipient().to_string(),
            label: "ledger fixture".into(),
            remote: remote.to_string(),
            issued_at: now.to_unix(),
            chains: invite
                .grants()
                .chains()
                .iter()
                .map(|chain| chain.to_bytes().map(hex::encode))
                .collect::<Result<_, _>>()?,
        };
        let public = serde_json::to_string(&group)?;
        assert!(!public.contains("tonk-agent-v"));
        assert!(!public.contains(&hex::encode(invite.secret_seed())));
        tonk.reactor
            .profile_repository()
            .branch("main")
            .transaction()
            .assert(AgentGrantGroup {
                this: group_entity(&group.id)?,
                account: fields::Account(group.account.clone()),
                subject: fields::Subject(group.subject.clone()),
                public_record: fields::PublicRecord(public),
            })
            .commit()
            .perform(&tonk.operator)
            .await?;
        assert_eq!(groups(&tonk).await?.len(), 1);
        assert!(has_issued_for_subject(&tonk, invite.grants().subject()).await?);
        let other_profile = Profile::open("other-account")
            .at(location.clone())
            .perform(&tonk.storage)
            .await?;
        let other_registry = crate::device::Registry {
            profile: "other-account".into(),
            directory: location.clone(),
        };
        let other = crate::worker::boot_state(
            tonk.storage.clone(),
            "other-account".into(),
            other_profile,
            other_registry,
        )
        .await?;
        let other_root = Ed25519Signer::import(&[56; 32]).await?;
        let other_grant =
            tonk_identity::delegation::mint_device_delegation(other_root, &other.profile.did())
                .await?;
        super::super::identity::persist_root(
            &other,
            tonk_worker_api::SaveRootRequest {
                credential_id: "other-ledger-fixture".into(),
                delegation_hex: hex::encode(other_grant.to_bytes()?),
                passkey: None,
                encryption_key: None,
            },
        )
        .await?;
        // Even a copied public row is not selected under an unrelated account.
        other
            .reactor
            .profile_repository()
            .branch("main")
            .transaction()
            .assert(AgentGrantGroup {
                this: group_entity(&group.id)?,
                account: fields::Account(group.account.clone()),
                subject: fields::Subject(group.subject.clone()),
                public_record: fields::PublicRecord(serde_json::to_string(&group)?),
            })
            .commit()
            .perform(&other.operator)
            .await?;
        assert!(groups(&other).await?.is_empty());
        assert!(!has_issued_for_subject(&other, invite.grants().subject()).await?);
        drop(other);
        let all_failed =
            revoke_group(&tonk, &group, |_, _| async { Err(failure("offline")) }).await?;
        assert_eq!(all_failed.status, "partial");
        assert!(all_failed.targets.iter().all(|target| !target.acknowledged));
        assert_eq!(
            summarize(&tonk, &group).await?.status,
            "partial",
            "zero acknowledgements must retain the withdrawal intent"
        );
        let missed = cids(invite.grants())[2].clone();
        let mut attempts = Vec::new();
        let first = revoke_group(&tonk, &group, |_, cid| {
            attempts.push(cid.to_string());
            let fail = cid.to_string() == missed;
            let subject = tonk.profile.did();
            async move {
                if fail {
                    Err(failure("injected delivery interruption"))
                } else {
                    Ok(tonk_account::customer::RevokeReceipt {
                        revoked: cid,
                        subject,
                        recorded: true,
                    })
                }
            }
        })
        .await?;
        assert_eq!(attempts.len(), 6);
        assert_eq!(first.status, "partial");
        assert_eq!(
            first
                .targets
                .iter()
                .filter(|target| target.acknowledged)
                .count(),
            5
        );
        drop(tonk);
        let storage = Storage::default();
        let profile = Profile::load("ledger")
            .at(location.clone())
            .perform(&storage)
            .await?;
        let registry = crate::device::Registry {
            profile: "ledger".into(),
            directory: location,
        };
        let reopened =
            crate::worker::boot_state(storage, "ledger".into(), profile, registry).await?;
        assert!(has_issued_for_subject(&reopened, invite.grants().subject()).await?);
        assert_eq!(
            summarize(&reopened, &group)
                .await?
                .targets
                .iter()
                .filter(|target| target.acknowledged)
                .count(),
            5
        );
        let mut retried = Vec::new();
        let final_result = revoke_group(&reopened, &group, |_, cid| {
            retried.push(cid.to_string());
            let subject = reopened.profile.did();
            async move {
                Ok(tonk_account::customer::RevokeReceipt {
                    revoked: cid,
                    subject,
                    recorded: false,
                })
            }
        })
        .await?;
        assert_eq!(retried, vec![missed]);
        assert_eq!(final_result.status, "revoked");
        let mut changed = group.clone();
        changed.subject = reopened.profile.did().to_string();
        assert!(validate(&changed).await.is_err());
        changed = group.clone();
        changed.id = "0".repeat(64);
        assert!(validate(&changed).await.is_err());
        drop(reopened);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }
}
