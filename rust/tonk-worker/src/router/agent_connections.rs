//! Account-managed ordinary grants. Invitation secrets leave only through transient UI.
pub(super) mod terminal_management;
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
        .branch("main")
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
        .branch("main")
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
        .branch("main")
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
        .branch("main")
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

/// Minimal projection of an already recorded passkey custody identity.
#[derive(dialog_query::Concept, Clone, Debug)]
pub struct KnownCustody {
    this: dialog_artifacts::Entity,
    credential_id: tonk_schema::domain::recovery::CredentialId,
}

/// Refuse known internal identities even if a stale row labels one a user space.
async fn ensure_terminal_subject(tonk: &TonkState, subject: &Did) -> Result<(), TonkWorkerError> {
    use tonk_schema::{Replica, prelude::DidExt as _};
    let root = super::identity::local_root(tonk).await?;
    if subject == &root.root_did || subject == &tonk.profile.did() {
        return Err(TonkWorkerError::Forbidden(
            "account and profile data cannot be granted to a terminal".into(),
        ));
    }
    if root.encryption_key.as_ref() == Some(subject) {
        return Err(TonkWorkerError::Forbidden(
            "account custody data cannot be granted to a terminal".into(),
        ));
    }
    let branch = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<Replica> = branch
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::from(Replica::new(tonk.profile.did(), subject.clone()).this),
            subject: Term::var("subject"),
            profile: Term::var("profile"),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    if rows
        .iter()
        .any(|row| row.kind != Replica::repository_kind())
    {
        return Err(TonkWorkerError::Forbidden(
            "internal account and ledger replicas cannot be granted to a terminal".into(),
        ));
    }
    let custody: Vec<KnownCustody> = branch
        .handle()
        .query()
        .select(Query::<KnownCustody> {
            this: Term::from(subject.this()),
            credential_id: Term::var("credential"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    if !custody.is_empty() {
        return Err(TonkWorkerError::Forbidden(
            "passkey custody data cannot be granted to a terminal".into(),
        ));
    }
    Ok(())
}

/// List this profile's real spaces, including explicit inability to delegate.
/// A displayed all-current selection is an immutable list of these subjects.
#[wasm_compat]
pub async fn terminal_spaces(
    State(state): State<AppState>,
) -> Result<Json<tonk_worker_api::TerminalLinkSpaces>, TonkWorkerError> {
    enabled()?;
    let tonk = state.read().await;
    Ok(Json(selection_snapshot(&tonk).await?))
}

async fn selection_snapshot(
    tonk: &TonkState,
) -> Result<tonk_worker_api::TerminalLinkSpaces, TonkWorkerError> {
    use tonk_schema::domain::replica::Profile as ProfileEntity;
    use tonk_schema::{Replica, prelude::DidExt as _};
    let root = super::identity::local_root(tonk).await?;
    let meta = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let replicas: Vec<Replica> = meta
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(tonk.profile.did().this())),
            kind: Term::from(Replica::repository_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(failure)?;
    let mut spaces = Vec::new();
    for replica in replicas {
        let subject: Did = replica.subject.0.to_string().parse().map_err(failure)?;
        if replica.this != Replica::new(tonk.profile.did(), subject.clone()).this {
            return Err(failure("space replica identity mismatch"));
        }
        let repo = subject.repo_key().to_owned();
        let mut name = repo.clone();
        let result = async {
            ensure_terminal_subject(tonk, &subject).await?;
            let repository = tonk
                .profile
                .repository(&repo)
                .load()
                .perform(&tonk.operator)
                .await
                .map_err(failure)?;
            if repository.did() != subject {
                return Err(failure("space subject changed"));
            }
            name = super::repository::repository_display_name(tonk, &repository, &repo)
                .await
                .unwrap_or_else(|| repo.clone());
            match resolve_remote_url(tonk, &repository).await? {
                RemoteRequirement::Ready(_) => {}
                RemoteRequirement::Refused(_) => {
                    return Err(failure("space needs a configured sync remote"));
                }
            }
            issuer_ancestors(tonk, &candidate_build_scopes(&subject), now, expires).await?;
            Ok::<(), TonkWorkerError>(())
        }
        .await;
        spaces.push(tonk_worker_api::TerminalLinkSpace {
            repo,
            subject: subject.to_string(),
            name,
            can_delegate: result.is_ok(),
            reason: result.err().map(|error| error.to_string()),
        });
    }
    spaces.sort_by(|left, right| left.subject.cmp(&right.subject));
    spaces.dedup_by(|left, right| left.subject == right.subject);
    let snapshot = selection_fingerprint(&root.bytes, &spaces)?;
    Ok(tonk_worker_api::TerminalLinkSpaces {
        max_spaces: tonk_invite::terminal::MAX_SELECTED_SPACES,
        account: root.root_did.to_string(),
        snapshot,
        spaces,
        grant_lifetime_seconds: DEFAULT_GRANT_TTL_SECONDS,
    })
}

fn selection_fingerprint(
    root: &[u8],
    spaces: &[tonk_worker_api::TerminalLinkSpace],
) -> Result<String, TonkWorkerError> {
    // Review pins the verified account proof and the actual space identities.
    // Names and availability diagnostics may change while background sync runs;
    // the submitted explicit selection is checked against current authority by
    // prepare_terminal_selection before any grants are staged.
    let identities: Vec<_> = spaces
        .iter()
        .map(|space| (&space.repo, &space.subject))
        .collect();
    let mut fingerprint = blake3::Hasher::new();
    fingerprint.update(b"tonk-terminal-selection-v2\0");
    fingerprint.update(root);
    fingerprint.update(&serde_json::to_vec(&identities).map_err(failure)?);
    Ok(fingerprint.finalize().to_hex().to_string())
}

async fn prepare_terminal_selection(
    tonk: &TonkState,
    snapshot: &tonk_worker_api::TerminalLinkSpaces,
    subjects: &[String],
    request: &tonk_invite::terminal::LinkRequest,
    now: Timestamp,
) -> Result<(Vec<SpaceGrantBundle>, Vec<PublicGroup>), TonkWorkerError> {
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(failure)?;
    let mut bundles = Vec::new();
    let mut selected_groups = Vec::new();
    for subject in subjects {
        ensure_terminal_subject(tonk, &subject.parse().map_err(failure)?).await?;
        let selected = snapshot
            .spaces
            .iter()
            .find(|space| &space.subject == subject)
            .ok_or_else(|| failure("selected space is not in the current snapshot"))?;
        if !selected.can_delegate {
            return Err(TonkWorkerError::Forbidden(
                selected.reason.clone().unwrap_or_else(|| {
                    "selected space cannot delegate the requested build access".into()
                }),
            ));
        }
        let repository = tonk
            .profile
            .repository(&selected.repo)
            .load()
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        let remote = match resolve_remote_url(tonk, &repository).await? {
            RemoteRequirement::Ready(remote) => remote.access_url,
            RemoteRequirement::Refused(_) => {
                return Err(failure("selected space has no sync remote"));
            }
        };
        let scopes = candidate_build_scopes(&repository.did());
        let ancestors = issuer_ancestors(tonk, &scopes, now, expires).await?;
        let bundle = issue_to_recipient(
            request.recipient(),
            tonk.profile.signer().signer().clone(),
            ancestors,
            &scopes,
            &remote,
            now,
            expires,
        )
        .await?;
        selected_groups.push(PublicGroup {
            terminal_request: Some(request.id()),
            version: 1,
            id: grant_set_id(subject, request.recipient().as_str(), &cids(&bundle)),
            account: snapshot.account.clone(),
            repo: selected.repo.clone(),
            subject: subject.clone(),
            recipient: request.recipient().to_string(),
            label: request.label().to_owned(),
            remote: remote.to_string(),
            issued_at: now.to_unix(),
            chains: bundle
                .chains()
                .iter()
                .map(|chain| chain.to_bytes().map(hex::encode).map_err(failure))
                .collect::<Result<_, _>>()?,
        });
        bundles.push(bundle);
    }
    Ok((bundles, selected_groups))
}

async fn record_terminal_delivery(
    tonk: &TonkState,
    entity: dialog_artifacts::Entity,
    receipt: &[u8],
) -> Result<(), TonkWorkerError> {
    tonk.reactor
        .profile_repository()
        .branch("main")
        .transaction()
        .assert(fields::TerminalLinkDelivered {
            this: entity,
            delivery_receipt: fields::DeliveryReceipt(
                String::from_utf8(receipt.to_vec()).map_err(failure)?,
            ),
        })
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(())
}

async fn terminal_endpoint(
    tonk: &TonkState,
    request: &tonk_invite::terminal::LinkRequest,
) -> Result<url::Url, TonkWorkerError> {
    let provider = super::account::provider(tonk)
        .await
        .ok_or_else(|| failure("account has no attached delivery service"))?;
    let origin = url::Url::parse(&provider).map_err(failure)?;
    let loopback = origin
        .host_str()
        .is_some_and(|host| host == "localhost" || host == "127.0.0.1" || host == "[::1]");
    if !(origin.scheme() == "https" || origin.scheme() == "http" && loopback)
        || !origin.username().is_empty()
        || origin.password().is_some()
    {
        return Err(failure("invalid configured terminal delivery origin"));
    }
    let response =
        super::http::terminal_request(&origin.join("/.well-known/tonk").map_err(failure)?, None)
            .await?;
    let config: tonk_worker_api::DeploymentConfig =
        serde_json::from_slice(&response.body).map_err(failure)?;
    if config.service_did.as_deref() != Some(request.service().as_str()) {
        return Err(TonkWorkerError::Forbidden(
            "terminal request names a different delivery service".into(),
        ));
    }
    origin.join("/connection/delivery").map_err(failure)
}

#[wasm_compat]
pub async fn terminal_approve(
    State(state): State<AppState>,
    Json(input): Json<tonk_worker_api::TerminalLinkApproveRequest>,
) -> Result<Json<tonk_worker_api::TerminalLinkApprovalReceipt>, TonkWorkerError> {
    terminal_decide(state, input, false).await.map(Json)
}

#[wasm_compat]
pub async fn terminal_decline(
    State(state): State<AppState>,
    Json(input): Json<tonk_worker_api::TerminalLinkDeclineRequest>,
) -> Result<Json<tonk_worker_api::TerminalLinkApprovalReceipt>, TonkWorkerError> {
    terminal_decide(
        state,
        tonk_worker_api::TerminalLinkApproveRequest {
            request: input.request,
            snapshot: input.snapshot,
            subjects: Vec::new(),
        },
        true,
    )
    .await
    .map(Json)
}

static TERMINAL_APPROVAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn terminal_decide(
    state: AppState,
    input: tonk_worker_api::TerminalLinkApproveRequest,
    declined: bool,
) -> Result<tonk_worker_api::TerminalLinkApprovalReceipt, TonkWorkerError> {
    let _serial = TERMINAL_APPROVAL.lock().await;
    enabled()?;
    // One exclusive state guard pins the selected profile, account key and replica set
    // through preparation, durable public staging and mailbox acknowledgement.
    let tonk = state.write().await;
    let now = Timestamp::now();
    if input.request.len() > 32 * 1024
        || (!declined && input.subjects.is_empty())
        || input.subjects.len() > tonk_invite::terminal::MAX_SELECTED_SPACES
    {
        return Err(failure("invalid terminal approval size or selection"));
    }
    let bytes = hex::decode(&input.request).map_err(failure)?;
    let request = tonk_invite::terminal::LinkRequest::inspect(&bytes)
        .await
        .map_err(failure)?;
    let root = super::identity::local_root(&tonk).await?;
    let endpoint = terminal_endpoint(&tonk, &request).await?;
    if request
        .expected_account()
        .is_some_and(|expected| expected != &root.root_did)
    {
        return Err(TonkWorkerError::Forbidden(
            "terminal request names another account".into(),
        ));
    }
    let mut subjects = input.subjects;
    subjects.sort();
    if subjects.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(failure("duplicate selected space"));
    }
    let entity: dialog_artifacts::Entity = format!("id:tonk:terminal-approval:{}", request.id())
        .parse()
        .map_err(failure)?;
    let account = tonk
        .reactor
        .profile_repository()
        .branch("main")
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let cached: Vec<fields::TerminalLinkApproval> = account
        .handle()
        .query()
        .select(Query::<fields::TerminalLinkApproval> {
            this: Term::from(entity.clone()),
            account: Term::from(fields::Account(root.root_did.to_string())),
            approval: Term::var("approval"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    if cached.len() > 1 {
        return Err(failure("conflicting saved terminal approvals"));
    }
    let approval = if let Some(saved) = cached.first() {
        let approval = tonk_invite::terminal::Approval::validate(
            &hex::decode(&saved.approval.0).map_err(failure)?,
            now.to_unix(),
        )
        .await
        .map_err(failure)?;
        let saved_subjects: Vec<_> = approval
            .bundles()
            .iter()
            .map(|bundle| bundle.subject().to_string())
            .collect();
        if approval.request().bytes() != bytes
            || approval.account() != &root.root_did
            || saved_subjects != subjects
            || approval.is_declined() != declined
        {
            return Err(TonkWorkerError::Conflict(
                "a different complete selection was already approved".into(),
            ));
        }
        approval
    } else {
        tonk_invite::terminal::LinkRequest::validate(&bytes, now.to_unix())
            .await
            .map_err(failure)?;
        let snapshot = selection_snapshot(&tonk).await?;
        if snapshot.snapshot != input.snapshot || snapshot.account != root.root_did.as_str() {
            return Err(TonkWorkerError::Conflict(
                "account or current spaces changed; review the selection again".into(),
            ));
        }
        let (bundles, selected_groups) =
            prepare_terminal_selection(&tonk, &snapshot, &subjects, &request, now).await?;
        let approval = if declined {
            tonk_invite::terminal::Approval::sign_decline(
                tonk.profile.signer().signer(),
                &request,
                root.delegation,
                now.to_unix(),
            )
            .await
            .map_err(failure)?
        } else {
            tonk_invite::terminal::Approval::sign(
                tonk.profile.signer().signer(),
                &request,
                root.delegation,
                bundles,
                now.to_unix(),
            )
            .await
            .map_err(failure)?
        };
        // All bundles have been verified before any public group is published.
        account
            .handle()
            .delegations()
            .retain_all(
                approval
                    .bundles()
                    .iter()
                    .flat_map(|bundle| bundle.chains().iter().cloned().map(UcanDelegation))
                    .collect::<Vec<_>>(),
            )
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        let mut transaction = tonk
            .reactor
            .profile_repository()
            .branch("main")
            .transaction();
        for group in selected_groups {
            transaction = transaction.assert(AgentGrantGroup {
                this: group_entity(&group.id)?,
                account: fields::Account(group.account.clone()),
                subject: fields::Subject(group.subject.clone()),
                public_record: fields::PublicRecord(
                    serde_json::to_string(&group).map_err(failure)?,
                ),
            });
        }
        transaction
            .assert(fields::TerminalLinkApproval {
                this: entity.clone(),
                account: fields::Account(root.root_did.to_string()),
                approval: fields::Approval(hex::encode(approval.bytes())),
            })
            .commit()
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
        approval
    };
    publish_terminal_approval(&tonk, &approval, entity, &endpoint).await
}

async fn publish_terminal_approval(
    tonk: &TonkState,
    approval: &tonk_invite::terminal::Approval,
    entity: dialog_artifacts::Entity,
    endpoint: &url::Url,
) -> Result<tonk_worker_api::TerminalLinkApprovalReceipt, TonkWorkerError> {
    let response = super::http::terminal_request(endpoint, Some(approval.bytes())).await?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Receipt {
        request_id: String,
        recorded: bool,
    }
    let receipt: Receipt = serde_json::from_slice(&response.body).map_err(failure)?;
    if receipt.request_id != approval.request().id() {
        return Err(failure("service acknowledged another terminal request"));
    }
    record_terminal_delivery(tonk, entity, &response.body).await?;
    let ids: Vec<_> = approval
        .bundles()
        .iter()
        .map(|bundle| {
            grant_set_id(
                bundle.subject().as_str(),
                approval.request().recipient().as_str(),
                &cids(bundle),
            )
        })
        .collect();
    let mut connections = Vec::new();
    for group in groups(tonk).await?.into_iter().filter(|group| {
        group.terminal_request.as_deref() == Some(receipt.request_id.as_str())
            && ids.contains(&group.id)
    }) {
        connections.push(summarize(tonk, &group).await?);
    }
    if connections.len() != approval.bundles().len() {
        return Err(failure(
            "saved terminal groups do not match complete approval",
        ));
    }
    Ok(tonk_worker_api::TerminalLinkApprovalReceipt {
        request_id: receipt.request_id,
        recorded: receipt.recorded,
        connections,
    })
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
        .branch("main")
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
                    .branch("main")
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

    #[test]
    fn terminal_snapshot_preserves_reviewed_membership_across_presentation_changes() {
        let mut rows = vec![tonk_worker_api::TerminalLinkSpace {
            repo: "space-a".into(),
            subject: "did:key:space-a".into(),
            name: "space-a".into(),
            can_delegate: false,
            reason: Some("space needs a configured sync remote".into()),
        }];
        let reviewed = selection_fingerprint(b"verified account proof", &rows).unwrap();
        // A name/remote may arrive from background sync after rendering. The
        // exact selected subject must still pass current issuance validation.
        rows[0].name = "Garden".into();
        rows[0].can_delegate = true;
        rows[0].reason = None;
        assert_eq!(
            selection_fingerprint(b"verified account proof", &rows).unwrap(),
            reviewed,
            "unchanged account and subject membership acquired a stale snapshot"
        );
        assert_ne!(
            selection_fingerprint(b"another account proof", &rows).unwrap(),
            reviewed
        );
        let mut changed = rows.clone();
        changed[0].subject = "did:key:space-b".into();
        assert_ne!(
            selection_fingerprint(b"verified account proof", &changed).unwrap(),
            reviewed
        );
        changed = rows.clone();
        changed[0].repo = "space-b".into();
        assert_ne!(
            selection_fingerprint(b"verified account proof", &changed).unwrap(),
            reviewed
        );
        assert_ne!(
            selection_fingerprint(b"verified account proof", &[]).unwrap(),
            reviewed
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn terminal_selection_proves_owned_shared_and_reports_read_only() -> anyhow::Result<()> {
        use super::super::repository::{
            BranchConfiguration, RemoteConfiguration, RepositoryConfiguration,
        };
        use dialog_credentials::{Credential, Ed25519Verifier};
        use dialog_effects::{
            space::{Space, SpaceExt as _},
            storage::Directory,
        };
        use dialog_operator::Profile;
        use dialog_repository::{Repository, SiteAddress};
        use dialog_storage::provider::storage::Storage;
        use tonk_schema::prelude::DidExt as _;
        let directory =
            std::env::temp_dir().join(format!("tonk-terminal-selection-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let storage = Storage::default();
        let profile = Profile::open("selection")
            .at(location.clone())
            .perform(&storage)
            .await?;
        let registry = crate::device::Registry {
            profile: "selection".into(),
            directory: location,
        };
        let tonk =
            crate::worker::boot_state(storage, "selection".into(), profile, registry).await?;
        let root = Ed25519Signer::import(&[61; 32]).await?;
        let device =
            tonk_identity::delegation::mint_device_delegation(root.clone(), &tonk.profile.did())
                .await?;
        super::super::identity::persist_root(
            &tonk,
            tonk_worker_api::SaveRootRequest {
                credential_id: "selection".into(),
                delegation_hex: hex::encode(device.to_bytes()?),
                passkey: None,
                encryption_key: None,
            },
        )
        .await?;
        let now = Timestamp::now();
        let deadline =
            Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS + 3600) as i128)?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let remote: url::Url = format!("http://{address}/ucan/").parse()?;
        let config = RepositoryConfiguration::default()
            .remote(
                "origin",
                RemoteConfiguration::new(SiteAddress::from(
                    dialog_remote_ucan_s3::UcanAddress::new(remote.clone()),
                )),
            )
            .branch(
                "main",
                BranchConfiguration::default().upstream("origin", "main"),
            );
        let mut subjects = Vec::new();
        for command in [vec![], vec!["use".into()], vec!["use".into(), "get".into()]] {
            let owner = Ed25519Signer::generate().await?;
            let subject = owner.did();
            let grant = DelegationBuilder::new()
                .issuer(Signer::from(owner))
                .audience(&root.did())
                .subject(dialog_ucan_core::subject::Subject::Specific(
                    subject.clone(),
                ))
                .command(command)
                .expiration(deadline)
                .try_build()
                .await?;
            tonk.profile
                .access()
                .save(UcanDelegation(DelegationChain::new(grant)))
                .perform(&tonk.operator)
                .await?;
            let verifier: Ed25519Verifier = subject.as_str().parse()?;
            let credential = Subject::from(tonk.profile.did())
                .attenuate(Space::new(subject.repo_key()))
                .create(Credential::from(verifier))
                .perform(&tonk.operator)
                .await?;
            let repository = Repository::from(credential);
            super::super::repository::record_replica_meta(&tonk, &repository, "selection", &config)
                .await?;
            subjects.push(subject);
        }
        let snapshot = selection_snapshot(&tonk).await?;
        assert_eq!(snapshot.spaces.len(), 3);
        for subject in &subjects[..2] {
            let entry = snapshot
                .spaces
                .iter()
                .find(|entry| entry.subject == subject.as_str())
                .unwrap();
            assert!(entry.can_delegate, "{:?}", entry.reason);
            let scopes = candidate_build_scopes(subject);
            let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
            let ancestors = issuer_ancestors(&tonk, &scopes, now, expires).await?;
            let terminal = Ed25519Signer::import(&[65; 32]).await?.did();
            let bundle = issue_to_recipient(
                &terminal,
                tonk.profile.signer().signer().clone(),
                ancestors,
                &scopes,
                &remote,
                now,
                expires,
            )
            .await?;
            assert_eq!(bundle.recipient(), &terminal);
            assert_eq!(bundle.chains().len(), 6);
        }
        let readonly = snapshot
            .spaces
            .iter()
            .find(|entry| entry.subject == subjects[2].as_str())
            .unwrap();
        assert!(!readonly.can_delegate);
        assert!(
            readonly
                .reason
                .as_ref()
                .is_some_and(|reason| !reason.is_empty())
        );
        assert_eq!(selection_snapshot(&tonk).await?.snapshot, snapshot.snapshot);
        // The worker's actual publication boundary: the service fixture checks
        // complete signatures while the service crate tests customer/revocation
        // authorization and durable create-only semantics independently.
        let service = Ed25519Signer::import(&[66; 32]).await?.did();
        let terminal = Signer::from(Ed25519Signer::import(&[65; 32]).await?);
        let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let posted = seen.clone();
        let advertised = service.to_string();
        let additions = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let appended = additions.clone();
        let addition_fail_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reject_addition = addition_fail_once.clone();
        let revoked = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
        let revoked_targets = revoked.clone();
        let fail_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reject_next = fail_once.clone();
        let app = axum::Router::new()
            .route("/.well-known/tonk", axum::routing::get(move || {
                let service = advertised.clone();
                async move { Json(serde_json::json!({"serviceDid":service})) }
            }))
            .route("/connection/addition", axum::routing::post(move |body: axum::body::Bytes| {
                let appended = appended.clone();
                let reject_addition = reject_addition.clone();
                async move {
                    let addition = tonk_invite::terminal::Addition::validate(&body, Timestamp::now().to_unix()).await.unwrap();
                    appended.lock().await.push(body.to_vec());
                    if reject_addition.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"retry"})));
                    }
                    (axum::http::StatusCode::OK, Json(serde_json::json!({"deliveryId":addition.id(),"recorded":true})))
                }
            }))
            .route("/ucan/", axum::routing::post(move |body: axum::body::Bytes| {
                let revoked_targets = revoked_targets.clone();
                let reject_next = reject_next.clone();
                async move {
                    let checked = tonk_identity::revocation::verify(&body).await.unwrap();
                    revoked_targets.lock().await.push(checked.target_cid.clone());
                    if reject_next.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"retry"})));
                    }
                    (axum::http::StatusCode::OK, Json(serde_json::to_value(tonk_account::customer::RevokeReceipt { revoked: checked.target_cid.parse().unwrap(), subject: checked.subject, recorded: true }).unwrap()))
                }
            }))
            .route("/connection/delivery", axum::routing::post(move |body: axum::body::Bytes| {
                let posted = posted.clone();
                async move {
                    let approval = tonk_invite::terminal::Approval::validate(&body, Timestamp::now().to_unix()).await.unwrap();
                    posted.lock().await.push(body.to_vec());
                    Json(serde_json::json!({"requestId":approval.request().id(),"recorded":true}))
                }
            }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = tonk_account::AccountProviderRecord::attach(
            &format!("http://{address}/ucan/"),
            now.to_unix(),
        )?;
        tonk.profile
            .credential()
            .site(tonk_account::ACCOUNT_PROVIDER_CREDENTIAL_SITE)
            .save(provider.encode()?)
            .perform(&tonk.operator)
            .await?;
        let state = std::sync::Arc::new(tokio::sync::RwLock::new(tonk));
        let request = tonk_invite::terminal::LinkRequest::sign(
            &terminal,
            &service,
            [67; 32],
            now.to_unix(),
            now.to_unix() + 600,
            "Work terminal",
            Some(&root.did()),
        )
        .await?;
        let input = tonk_worker_api::TerminalLinkApproveRequest {
            request: hex::encode(request.bytes()),
            snapshot: snapshot.snapshot.clone(),
            subjects: vec![subjects[1].to_string()],
        };
        let receipt = terminal_decide(state.clone(), input.clone(), false).await?;
        assert_eq!(receipt.connections.len(), 1);
        assert_eq!(receipt.connections[0].subject, subjects[1].as_str());
        terminal_decide(state.clone(), input.clone(), false).await?;
        {
            let posted = seen.lock().await;
            assert_eq!(
                posted[0], posted[1],
                "retry publishes byte-identical complete approval"
            );
        }
        let mut changed = input;
        changed.subjects = vec![subjects[0].to_string()];
        assert!(
            terminal_decide(state.clone(), changed, false)
                .await
                .is_err()
        );
        assert_eq!(seen.lock().await.len(), 2);
        let rejected = tonk_invite::terminal::LinkRequest::sign(
            &terminal,
            &service,
            [68; 32],
            now.to_unix(),
            now.to_unix() + 600,
            "Rejected selection",
            Some(&root.did()),
        )
        .await?;
        let input = tonk_worker_api::TerminalLinkApproveRequest {
            request: hex::encode(rejected.bytes()),
            snapshot: snapshot.snapshot.clone(),
            subjects: vec![subjects[0].to_string(), subjects[2].to_string()],
        };
        assert!(terminal_decide(state.clone(), input, false).await.is_err());
        {
            let tonk = state.read().await;
            assert_eq!(
                groups(&tonk).await?.len(),
                1,
                "mixed selection failure publishes no partial groups"
            );
        }
        let both = tonk_invite::terminal::LinkRequest::sign(
            &terminal,
            &service,
            [69; 32],
            now.to_unix(),
            now.to_unix() + 600,
            "Two spaces",
            Some(&root.did()),
        )
        .await?;
        let input = tonk_worker_api::TerminalLinkApproveRequest {
            request: hex::encode(both.bytes()),
            snapshot: snapshot.snapshot.clone(),
            subjects: subjects[..2].iter().map(ToString::to_string).collect(),
        };
        assert_eq!(
            terminal_decide(state.clone(), input, false)
                .await?
                .connections
                .len(),
            2
        );
        let declined = tonk_invite::terminal::LinkRequest::sign(
            &terminal,
            &service,
            [70; 32],
            now.to_unix(),
            now.to_unix() + 600,
            "Declined",
            Some(&root.did()),
        )
        .await?;
        let input = tonk_worker_api::TerminalLinkApproveRequest {
            request: hex::encode(declined.bytes()),
            snapshot: snapshot.snapshot.clone(),
            subjects: Vec::new(),
        };
        assert!(
            terminal_decide(state.clone(), input.clone(), true)
                .await?
                .connections
                .is_empty()
        );
        assert!(terminal_decide(state.clone(), input, false).await.is_err());
        let posted = seen.lock().await;
        assert_eq!(posted.len(), 4);
        let last = tonk_invite::terminal::Approval::validate(
            posted.last().unwrap(),
            Timestamp::now().to_unix(),
        )
        .await?;
        assert!(last.is_declined());
        assert!(last.bundles().is_empty());
        drop(posted);
        let listed = terminal_management::list(State(state.clone())).await?.0;
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .all(|terminal| terminal.delivery_status == "delivered")
        );
        let add = tonk_worker_api::TerminalConnectionAddRequest {
            snapshot: snapshot.snapshot.clone(),
            subjects: vec![subjects[0].to_string()],
            operation_id: "first-add".into(),
        };
        addition_fail_once.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            terminal_management::add(State(state.clone()), Path(request.id()), Json(add.clone()))
                .await
                .is_err()
        );
        let pending = terminal_management::list(State(state.clone())).await?.0;
        assert_eq!(
            pending
                .iter()
                .find(|item| item.request_id == request.id())
                .unwrap()
                .delivery_status,
            "pending"
        );
        assert_eq!(
            pending
                .iter()
                .find(|item| item.request_id == request.id())
                .unwrap()
                .pending_additions[0]
                .operation_id,
            "first-add"
        );
        let added =
            terminal_management::add(State(state.clone()), Path(request.id()), Json(add.clone()))
                .await?
                .0;
        assert_eq!(added.connections.len(), 1);
        assert_eq!(added.connections[0].recipient, terminal.did().as_str());
        let delivered = terminal_management::list(State(state.clone())).await?.0;
        assert_eq!(
            delivered
                .iter()
                .find(|item| item.request_id == request.id())
                .unwrap()
                .delivery_status,
            "delivered"
        );
        {
            let rows = additions.lock().await;
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0], rows[1]);
        }
        let repeated = tonk_worker_api::TerminalConnectionAddRequest {
            snapshot: snapshot.snapshot.clone(),
            subjects: vec![subjects[0].to_string()],
            operation_id: "unwanted-duplicate".into(),
        };
        assert!(
            terminal_management::add(State(state.clone()), Path(request.id()), Json(repeated))
                .await
                .is_err()
        );
        fail_once.store(true, std::sync::atomic::Ordering::SeqCst);
        let partial = terminal_management::revoke(
            State(state.clone()),
            Path(request.id()),
            Json(tonk_worker_api::TerminalConnectionRevokeRequest { group_ids: None }),
        )
        .await?
        .0;
        assert_eq!(partial.len(), 2);
        assert!(partial.iter().any(|group| group.status == "partial"));
        assert_eq!(revoked.lock().await.len(), 12);
        let complete = terminal_management::revoke(
            State(state.clone()),
            Path(request.id()),
            Json(tonk_worker_api::TerminalConnectionRevokeRequest { group_ids: None }),
        )
        .await?
        .0;
        assert!(complete.iter().all(|group| group.status == "revoked"));
        assert_eq!(
            revoked.lock().await.len(),
            13,
            "retry sends only the missing standard revocation"
        );
        let readd = tonk_worker_api::TerminalConnectionAddRequest {
            snapshot: snapshot.snapshot,
            subjects: vec![subjects[0].to_string()],
            operation_id: "fresh-re-add".into(),
        };
        let renewed =
            terminal_management::add(State(state.clone()), Path(request.id()), Json(readd))
                .await?
                .0;
        assert_eq!(
            renewed.connections[0].recipient,
            added.connections[0].recipient
        );
        assert_ne!(renewed.connections[0].id, added.connections[0].id);
        assert!(renewed.connections[0].targets.iter().all(|new| {
            added.connections[0]
                .targets
                .iter()
                .all(|old| old.cid != new.cid)
        }));
        let initial_retry = terminal_management::retry(State(state.clone()), Path(request.id()))
            .await?
            .0;
        assert_eq!(
            initial_retry.connections.len(),
            1,
            "initial retry never absorbs later additions or mints replacements"
        );
        let listed = terminal_management::list(State(state.clone())).await?.0;
        let sibling = listed
            .iter()
            .find(|terminal| terminal.request_id == both.id())
            .unwrap();
        assert!(sibling.spaces.iter().all(|space| space.status == "active"));
        // Retained internal identities remain forbidden even when a stale
        // replica row is relabelled as an ordinary selectable space.
        {
            let tonk = state.read().await;
            let custody = Ed25519Signer::import(&[71; 32]).await?.did();
            let ledger = Ed25519Signer::import(&[72; 32]).await?.did();
            tonk.reactor
                .profile_repository()
                .branch("main")
                .transaction()
                .assert(tonk_schema::Replica::new(tonk.profile.did(), root.did()))
                .assert(tonk_schema::Replica::new(
                    tonk.profile.did(),
                    custody.clone(),
                ))
                .assert(tonk_schema::RecoveryPasskey::new(
                    &custody,
                    "custody-fixture",
                    now.to_unix(),
                    "test",
                ))
                .assert(
                    tonk_schema::Replica::with_kind(
                        tonk.profile.did(),
                        ledger.clone(),
                        tonk_schema::Replica::ledger_kind(),
                    )
                    .unwrap(),
                )
                .commit()
                .perform(&tonk.operator)
                .await?;
            let blocked = selection_snapshot(&tonk).await?;
            for subject in [root.did(), custody] {
                let entry = blocked
                    .spaces
                    .iter()
                    .find(|entry| entry.subject == subject.as_str())
                    .unwrap();
                assert!(!entry.can_delegate);
                assert!(
                    entry
                        .reason
                        .as_ref()
                        .is_some_and(|reason| reason.contains("cannot be granted"))
                );
                let mut forged = blocked.clone();
                forged
                    .spaces
                    .iter_mut()
                    .find(|entry| entry.subject == subject.as_str())
                    .unwrap()
                    .can_delegate = true;
                assert!(
                    prepare_terminal_selection(
                        &tonk,
                        &forged,
                        &[subject.to_string()],
                        &request,
                        Timestamp::now()
                    )
                    .await
                    .is_err()
                );
            }
            assert!(ensure_terminal_subject(&tonk, &ledger).await.is_err());
        }
        server.abort();
        drop(state);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    #[dialog_common::test]
    async fn terminal_issuer_uses_exact_public_recipient_and_rejects_missing_rights() {
        let (signer, ancestors, scopes, now) = fixture(DEFAULT_GRANT_TTL_SECONDS + 60).await;
        let terminal = Ed25519Signer::import(&[57; 32]).await.unwrap().did();
        let remote = "https://sync.example.test/ucan/".parse().unwrap();
        let deadline =
            Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128).unwrap();
        let bundle = issue_to_recipient(
            &terminal,
            signer.clone(),
            ancestors.clone(),
            &scopes,
            &remote,
            now,
            deadline,
        )
        .await
        .unwrap();
        assert_eq!(bundle.recipient(), &terminal);
        assert_eq!(bundle.chains().len(), 6);
        assert_eq!(bundle.expires_at(), deadline);
        assert!(
            bundle
                .chains()
                .iter()
                .all(|chain| chain.audience() == &terminal)
        );
        let mut missing = ancestors;
        missing.pop();
        assert!(
            issue_to_recipient(
                &terminal,
                signer.clone(),
                missing,
                &scopes,
                &remote,
                now,
                deadline,
            )
            .await
            .is_err()
        );
        let (_, limited, _, _) = fixture(3600).await;
        assert!(
            issue_to_recipient(&terminal, signer, limited, &scopes, &remote, now, deadline,)
                .await
                .is_err()
        );
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
        assert!(!public.contains("tonk-agent-v1"));
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
