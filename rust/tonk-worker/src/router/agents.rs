//! The account's agents, for the hub's "your agents" tab.
//!
//! An agent is a space marked [`AgentSpace`]; what it can reach is the
//! grants this profile issued to that space's DID, as retained in the
//! profile or in the agent's own space. Nothing else is recorded: the
//! list reads the delegations themselves, not a summary of them.
use axum::{
    Json,
    extract::{Path, State},
};
use dialog_query::{Output as _, Query, Term};
use dialog_repository::RepositoryExt as _;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, time::Timestamp};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_handler_macros::wasm_compat;
use tonk_schema::{
    Replica, agent::AgentSpace, domain::replica::Profile as ProfileEntity, prelude::DidExt as _,
};

use super::{
    AppState,
    agent_connections::{failure, publish_target},
    create_invite::{RemoteRequirement, resolve_remote_url},
    repository::{CONTENT_BRANCH, repository_display_name},
};
use crate::{TonkState, TonkWorkerError};

pub(crate) mod withdraw;

/// One agent and the spaces it can reach.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    /// The agent space's DID: the agent's identity.
    did: Did,
    /// The agent space's routing key.
    key: String,
    /// The agent space's name, as the approval page set it.
    name: Option<String>,
    /// What it can reach, space by space.
    spaces: Vec<AgentSpaceAccess>,
}

/// One space an agent can reach, and how.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpaceAccess {
    subject: Did,
    key: String,
    name: Option<String>,
    /// `read`, or `write` for read and write.
    access: &'static str,
    /// When the earliest of its grants lapses, unix seconds.
    expires_at: Option<u64>,
    /// Whether a share link issued any of its grants. Removing those asks
    /// for the passkey, and does not stop the link being used again.
    via_link: bool,
    /// Where a removal the person started stands, while it is under way
    /// or after it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    removal: Option<withdraw::Removal>,
}

/// Which of an agent's spaces to revoke.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevokeRequest {
    /// The space's subject DID.
    space: Did,
}

/// The profile's spaces, as `GET /api/profile` lists them.
pub(super) async fn spaces(tonk: &TonkState) -> Result<Vec<Did>, TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows: Vec<Replica> = session
        .handle()
        .query()
        .select(Query::<Replica> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            profile: Term::from(ProfileEntity(tonk.profile.did().this())),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    Ok(rows
        .into_iter()
        .filter(|row| row.kind == Replica::repository_kind())
        .filter_map(|row| row.subject.0.to_string().parse().ok())
        .collect())
}

/// Whether the space at `did` carries the agent-space mark.
pub(super) async fn is_agent(tonk: &TonkState, did: &Did) -> bool {
    let Ok(session) = tonk
        .reactor
        .repository(did.repo_key())
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        return false;
    };
    session
        .handle()
        .query()
        .select(Query::<AgentSpace> {
            this: Term::var("this"),
            kind: Term::from(tonk_schema::agent::agent_space_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .is_ok_and(|rows| rows.iter().any(|row| row.this.to_string() == did.as_str()))
}

/// The agent space at routing key `key`, when it is one of this
/// account's spaces and carries the agent-space mark.
pub(super) async fn find(tonk: &TonkState, key: &str) -> Result<Did, TonkWorkerError> {
    let agent = spaces(tonk)
        .await?
        .into_iter()
        .find(|did| did.repo_key() == key)
        .ok_or_else(|| TonkWorkerError::NotFound(format!("no agent {key}")))?;
    if !is_agent(tonk, &agent).await {
        return Err(TonkWorkerError::NotFound(format!("no agent {key}")));
    }
    Ok(agent)
}

/// The name the agent space `agent` gives `space` in its directory.
async fn directory_name(tonk: &TonkState, agent: &Did, space: &Did) -> Option<String> {
    let session = tonk
        .reactor
        .repository(agent.repo_key())
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .ok()?;
    let rows: Vec<tonk_schema::SpaceName> = session
        .handle()
        .query()
        .select(Query::<tonk_schema::SpaceName> {
            this: Term::from(space.this()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .ok()?;
    rows.into_iter()
        .next()
        .map(|row| row.name.0)
        .filter(|name| !name.trim().is_empty())
}

pub(super) async fn name(tonk: &TonkState, did: &Did) -> Option<String> {
    let key = did.repo_key();
    let repository = tonk
        .profile
        .space(key)
        .load()
        .perform(&tonk.operator)
        .await
        .ok()?;
    repository_display_name(tonk, &repository, key)
        .await
        .filter(|name| !name.trim().is_empty())
}

/// Where a grant is retained: the profile, which is where the approval
/// page keeps them today, or the agent's own space, where the agreed
/// design keeps them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Held {
    Profile,
    AgentSpace,
}

async fn retained(
    tonk: &TonkState,
    agent: &Did,
    held: Held,
) -> Result<Vec<UcanDelegation>, TonkWorkerError> {
    let session = match held {
        Held::Profile => tonk
            .reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .map_err(failure)?,
        Held::AgentSpace => tonk
            .reactor
            .repository(agent.repo_key())
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .map_err(failure)?,
    };
    session
        .handle()
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(failure)
}

/// Who issued a grant: this profile or its account root, which can
/// revoke it at the space's access service, or the key of a share link
/// the agent joined from the chat (`hosted::join`), which the person may
/// hold no proof to revoke. The agent, as the grant's audience, can: see
/// [`withdraw`], which opens its key with the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Issued {
    Here,
    Link,
}

/// The live grants to `agent`, each a one-certificate chain as retained,
/// read from both places a grant may be held: those this profile issued,
/// and those a share link issued into the agent space.
pub(super) async fn grants(
    tonk: &TonkState,
    agent: &Did,
) -> Result<Vec<(DelegationChain, Held, Issued)>, TonkWorkerError> {
    let now = Timestamp::now().to_unix();
    // The approval page issues from the profile today; the agreed design
    // issues from the account root, behind a passkey.
    let mut issuers = vec![tonk.profile.did()];
    if let Ok(root) = super::identity::local_root(tonk).await {
        issuers.push(root.root_did.clone());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut found = Vec::new();
    for held in [Held::AgentSpace, Held::Profile] {
        for delegation in retained(tonk, agent, held).await? {
            let chain = delegation.0;
            let issued = if issuers.contains(chain.issuer()) {
                Issued::Here
            } else if held == Held::AgentSpace {
                Issued::Link
            } else {
                continue;
            };
            if chain.subject().is_none_or(|subject| subject == agent)
                || chain.expiration().is_some_and(|at| at.to_unix() <= now)
                || !seen.insert(*chain.proof_cids().last().unwrap())
            {
                continue;
            }
            found.push((chain, held, issued));
        }
    }
    Ok(found)
}

/// `GET /api/agents`: every agent space and what it can reach.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentSummary>>, TonkWorkerError> {
    let tonk = state.read().await;
    let mut agents = Vec::new();
    for did in spaces(&tonk).await? {
        if !is_agent(&tonk, &did).await {
            continue;
        }
        let mut by_space =
            std::collections::BTreeMap::<String, (Did, bool, Option<u64>, bool)>::new();
        for (chain, _, issued) in grants(&tonk, &did).await? {
            let subject = chain.subject().unwrap().clone();
            let write = chain.ability().starts_with("/use/put");
            let expires = chain.expiration().map(|at| at.to_unix());
            let entry = by_space
                .entry(subject.to_string())
                .or_insert((subject, false, expires, false));
            entry.1 |= write;
            entry.2 = match (entry.2, expires) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            entry.3 |= issued == Issued::Link;
        }
        let key = did.repo_key().to_owned();
        let mut spaces = Vec::new();
        for (_, (subject, write, expires_at, via_link)) in by_space {
            // A space joined from a link may not be one of the person's:
            // the agent space's directory names it then.
            let name = match name(&tonk, &subject).await {
                Some(name) => Some(name),
                None => directory_name(&tonk, &did, &subject).await,
            };
            let removal = withdraw::status(&key, subject.as_str());
            spaces.push(AgentSpaceAccess {
                key: subject.repo_key().to_owned(),
                name,
                subject,
                access: if write { "write" } else { "read" },
                expires_at,
                via_link,
                removal,
            });
        }
        agents.push(AgentSummary {
            key,
            name: name(&tonk, &did).await,
            did,
            spaces,
        });
    }
    Ok(Json(agents))
}

/// What [`revoke`] answers: whether a passkey removal is under way.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeAnswer {
    /// The page was asked for the passkey; the list reports the removal
    /// (see [`withdraw::status`]) and drops the space when it is done.
    pending: bool,
}

/// `POST /api/agents/{key}/revoke`: withdraw every grant the agent holds
/// to one space. Each grant is revoked at the space's access service,
/// which checks every request the agent makes, and only then dropped, so
/// a failure leaves it listed to retry.
///
/// Grants the profile or the account issued are revoked here. Grants a
/// share link issued the agent can be revoked only by the agent, whose
/// key the account opens: for those the page that asked is asked for the
/// passkey, and [`withdraw::authorize`] finishes the removal.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path(key): Path<String>,
    client: Option<axum::Extension<super::ClientId>>,
    Json(request): Json<RevokeRequest>,
) -> Result<Json<RevokeAnswer>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    let agent = find(&tonk, &key).await?;
    let (leaves, links): (Vec<_>, Vec<_>) = grants(&tonk, &agent)
        .await?
        .into_iter()
        .filter(|(chain, _, _)| chain.subject() == Some(&request.space))
        .partition(|(_, _, issued)| *issued == Issued::Here);
    if leaves.is_empty() && links.is_empty() {
        return Err(TonkWorkerError::NotFound(
            "the agent holds no access to that space".into(),
        ));
    }
    if !leaves.is_empty() {
        revoke_issued(&tonk, &agent, &request.space, leaves).await?;
    }
    if links.is_empty() {
        leave_roster(&tonk, &agent, &request.space).await;
        return Ok(Json(RevokeAnswer { pending: false }));
    }
    drop(tonk);
    let Some(axum::Extension(client)) = client else {
        return Err(TonkWorkerError::Conflict(
            "removing access a share link gave the agent needs your passkey; remove it from Tonk in your browser".into(),
        ));
    };
    let space = request.space.to_string();
    withdraw::note(&key, &space, Some(withdraw::Removal::Pending));
    let asked = super::navigate::request_webauthn_with(
        &client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(tonk_worker_api::CustodyIntent::RevokeAgent(
            tonk_worker_api::AgentRevocation {
                agent: key.clone(),
                space: space.clone(),
            },
        )),
        None,
    )
    .await;
    if let Err(error) = asked {
        withdraw::note(&key, &space, None);
        return Err(error);
    }
    Ok(Json(RevokeAnswer { pending: true }))
}

/// Whether `member` is one of this profile's agents: a space the profile
/// holds that carries the agent-space mark. Its grants live in its own
/// space, not in the spaces it reaches, so removing it is a revocation
/// here ([`revoke_space`]), not an expulsion.
pub(super) async fn held_agent(tonk: &TonkState, member: &Did) -> Result<bool, TonkWorkerError> {
    Ok(spaces(tonk).await?.contains(member) && is_agent(tonk, member).await)
}

/// Withdraw every grant to `space` that the profile or the account issued
/// the agent `agent`, then, when no share-link grant is left, take it off
/// that space's roster. Answers the last revocation recorded.
///
/// Grants a share link issued need the account's passkey: remove those
/// from the agent list in Tonk ([`revoke`]).
pub(super) async fn revoke_space(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
) -> Result<tonk_account::customer::RevokeReceipt, TonkWorkerError> {
    let (leaves, links): (Vec<_>, Vec<_>) = grants(tonk, agent)
        .await?
        .into_iter()
        .filter(|(chain, _, _)| chain.subject() == Some(space))
        .partition(|(_, _, issued)| *issued == Issued::Here);
    if leaves.is_empty() {
        return Err(TonkWorkerError::NotFound(if links.is_empty() {
            "the agent holds no access to that space".into()
        } else {
            "the agent holds this space through a share link; remove it from the agent list in Tonk, with your passkey".into()
        }));
    }
    let receipt = revoke_issued(tonk, agent, space, leaves).await?;
    if links.is_empty() {
        leave_roster(tonk, agent, space).await;
    }
    Ok(receipt)
}

/// Take `agent` off the roster of `space` once it holds no access there.
/// A row left behind lists an agent that cannot reach the space, so a
/// failure is logged, not answered.
async fn leave_roster(tonk: &TonkState, agent: &Did, space: &Did) {
    if let Err(error) = take_off_roster(tonk, space.repo_key(), space, agent).await {
        tonk_common::log!("agent {agent} was not taken off the roster: {error}");
    }
}

/// Revoke `leaves`, grants to `agent` for `space` that the profile or the
/// account root issued, with the profile's own proof, and drop each once
/// the access service has acknowledged it. Answers the last revocation.
async fn revoke_issued(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
    leaves: Vec<(DelegationChain, Held, Issued)>,
) -> Result<tonk_account::customer::RevokeReceipt, TonkWorkerError> {
    let repository = tonk
        .profile
        .space(space.repo_key())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|_| TonkWorkerError::NotFound(format!("no space {space}")))?;
    let remote = match resolve_remote_url(tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => {
            return Err(TonkWorkerError::Conflict(
                "the space is not synced, so there is no access service to tell".into(),
            ));
        }
    };
    let profile = tonk.profile.did();
    let from_agent_space = leaves.iter().any(|(_, held, _)| *held == Held::AgentSpace);
    let mut last = None;
    for (leaf, held, _) in leaves {
        let target = *leaf
            .proof_cids()
            .last()
            .ok_or_else(|| failure("empty grant"))?;
        let certificate = leaf.proofs().last().unwrap().clone();
        let path = ancestor(tonk, &profile, space, &leaf)
            .await?
            .push(certificate)
            .map_err(failure)?;
        last = Some(publish_target(tonk, &remote, &path, &target).await?);
        let session = match held {
            Held::Profile => tonk
                .reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await
                .map_err(failure)?,
            Held::AgentSpace => tonk
                .reactor
                .repository(agent.repo_key())
                .branch(CONTENT_BRANCH)
                .acquire(&tonk.operator)
                .await
                .map_err(failure)?,
        };
        session
            .handle()
            .delegations()
            .retract(UcanDelegation(leaf))
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
    }
    // The hosted agent lists its spaces from the agent space: send the
    // removal there too. The access service already refuses the grant.
    if from_agent_space {
        tonk.sync_queue
            .mark_dirty(agent.repo_key(), super::sync::now_millis());
    }
    tonk_common::log!("agent {agent} access to {space} revoked");
    last.ok_or_else(|| failure("no grant was revoked"))
}

/// Put the agent `agent` on the roster of the space at `repo`, whose
/// subject is `subject`, as approving it to reach the space does: a
/// membership keyed on the agent space's DID, marked an agent, a member,
/// named `name`, and stamped as invited by `inviter`, the approving
/// account. The approving worker writes it, so the row is the sponsor's
/// word, not the agent's.
///
/// First-wins, as for a claimed invite: approving the same agent again
/// re-asserts the membership, which changes nothing, and stamps only what
/// the row lacks, so a renamed or promoted agent keeps its name and role.
#[cfg_attr(
    not(all(target_arch = "wasm32", target_os = "unknown")),
    allow(dead_code)
)]
pub(super) async fn put_on_roster(
    tonk: &TonkState,
    repo: &str,
    subject: &Did,
    agent: &Did,
    name: &str,
    inviter: &Did,
) -> Result<(), TonkWorkerError> {
    use tonk_schema::{InvitedBy, MemberKind, MemberName, MemberRole, Membership};
    let membership = Membership::new(agent.clone(), subject.clone());
    let entity = membership.this().clone();
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let branch = session.handle();
    let roles: Vec<MemberRole> = branch
        .query()
        .select(Query::<MemberRole> {
            this: Term::from(entity.clone()),
            role: Term::var("role"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let names: Vec<MemberName> = branch
        .query()
        .select(Query::<MemberName> {
            this: Term::from(entity.clone()),
            name: Term::var("name"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let inviters: Vec<InvitedBy> = branch
        .query()
        .select(Query::<InvitedBy> {
            this: Term::from(entity.clone()),
            inviter: Term::var("inviter"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    let kinds: Vec<MemberKind> = branch
        .query()
        .select(Query::<MemberKind> {
            this: Term::from(entity.clone()),
            kind: Term::var("kind"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;

    let mut transaction = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .transaction()
        .assert(membership);
    if roles.is_empty() {
        transaction = transaction.assert(MemberRole::member(entity.clone()));
    }
    if names.is_empty() {
        transaction = transaction.assert(MemberName::new(entity.clone(), name.to_owned()));
    }
    if inviters.is_empty() {
        transaction = transaction.assert(InvitedBy::new(entity.clone(), inviter));
    }
    if !kinds.iter().any(MemberKind::is_agent) {
        transaction = transaction.assert(MemberKind::agent(entity));
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    Ok(())
}

/// Take the agent `agent` off the roster of the space at `repo`, once
/// its access to the space is revoked.
pub(super) async fn take_off_roster(
    tonk: &TonkState,
    repo: &str,
    subject: &Did,
    agent: &Did,
) -> Result<(), TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(repo)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    super::members::retract_member_rows(tonk, repo, session.handle(), subject, agent).await;
    tonk.sync_queue.mark_dirty(repo, super::sync::now_millis());
    Ok(())
}

/// The profile's own proof for the scope `leaf` was issued under: the
/// path a revocation of `leaf` is witnessed by.
pub(super) async fn ancestor(
    tonk: &TonkState,
    profile: &Did,
    subject: &Did,
    leaf: &DelegationChain,
) -> Result<DelegationChain, TonkWorkerError> {
    use dialog_capability::{
        Subject,
        access::{Access, Prove},
    };
    let ability = leaf.ability();
    let scope = tonk_invite::connection::candidate_build_scopes(subject)
        .into_iter()
        .find(|scope| format!("/{}", scope.command.0.join("/")) == ability)
        .ok_or_else(|| failure(format!("no known scope for {ability}")))?;
    let proof = Subject::from(profile.clone())
        .attenuate(Access)
        .invoke(Prove::<dialog_ucan::Ucan>::new(profile.clone(), scope))
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    // The profile's proof runs space → … → account root → profile. A leaf
    // the root issued hangs off the root, so the path stops there; the
    // profile then withdraws it under the root's delegation to it, the
    // way `publish_target` withdraws a sibling device's grant.
    let issuer = leaf.issuer();
    let mut certificates: Vec<_> = proof.proofs.into_iter().map(|proof| proof.0).collect();
    if let Some(at) = certificates
        .iter()
        .position(|certificate| certificate.audience() == issuer)
    {
        certificates.truncate(at + 1);
    } else {
        return Err(failure("no path from the space to the grant's issuer"));
    }
    let mut certificates = certificates.into_iter();
    let mut chain = DelegationChain::new(
        certificates
            .next()
            .ok_or_else(|| failure("issuer proof is empty"))?,
    );
    for certificate in certificates {
        chain = chain.push(certificate).map_err(failure)?;
    }
    Ok(chain)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;
    use dialog_effects::storage::Directory;
    use dialog_varsig::Principal as _;
    use std::sync::Arc;
    use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS};
    use tonk_schema::{InvitedBy, MemberKind, MemberName, MemberRole, Membership};

    async fn booted(name: &str) -> anyhow::Result<AppState> {
        let directory = std::env::temp_dir().join(format!("tonk-{name}-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) = crate::device::open_profile_at(name, location.clone()).await?;
        let registry = crate::device::Registry::new(name.into(), location);
        let tonk = crate::worker::boot_state(storage, name.into(), profile, registry).await?;
        Ok(Arc::new(crate::StateLock::new(tonk)))
    }

    /// The roster rows of `agent`'s membership of the space at `key`.
    async fn roster(
        tonk: &TonkState,
        key: &str,
        entity: &dialog_artifacts::Entity,
    ) -> anyhow::Result<(
        Vec<Membership>,
        Vec<MemberRole>,
        Vec<MemberName>,
        Vec<InvitedBy>,
        Vec<MemberKind>,
    )> {
        let session = tonk
            .reactor
            .repository(key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await?;
        let branch = session.handle();
        let memberships: Vec<Membership> = branch
            .query()
            .select(Query::<Membership> {
                this: Term::var("this"),
                member: Term::var("member"),
                subject: Term::var("subject"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?
            .into_iter()
            .filter(|membership: &Membership| membership.this() == entity)
            .collect();
        let roles = branch
            .query()
            .select(Query::<MemberRole> {
                this: Term::from(entity.clone()),
                role: Term::var("role"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        let names = branch
            .query()
            .select(Query::<MemberName> {
                this: Term::from(entity.clone()),
                name: Term::var("name"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        let inviters = branch
            .query()
            .select(Query::<InvitedBy> {
                this: Term::from(entity.clone()),
                inviter: Term::var("inviter"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        let kinds = branch
            .query()
            .select(Query::<MemberKind> {
                this: Term::from(entity.clone()),
                kind: Term::var("kind"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await?;
        Ok((memberships, roles, names, inviters, kinds))
    }

    /// An approved agent joins the space's roster keyed on its agent
    /// space, as an agent and a member, named and credited to the
    /// approving account; approving it again writes nothing new, and
    /// revoking its access takes it off.
    #[dialog_common::test]
    async fn it_puts_an_approved_agent_on_the_roster() -> anyhow::Result<()> {
        let state = booted("agent-roster").await?;
        let key = super::super::repository::create_space_inner(&state, "Notes", None).await?;
        let agent = Ed25519Signer::import(&[72u8; 32]).await?.did();
        let inviter = Ed25519Signer::import(&[73u8; 32]).await?.did();
        let tonk = state.read().await;
        let subject = tonk
            .profile
            .space(&key)
            .load()
            .perform(&tonk.operator)
            .await?
            .did();
        let entity = Membership::new(agent.clone(), subject.clone())
            .this()
            .clone();

        put_on_roster(&tonk, &key, &subject, &agent, "Claude", &inviter).await?;
        let (memberships, roles, names, inviters, kinds) = roster(&tonk, &key, &entity).await?;
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].member.0.to_string(), agent.to_string());
        assert_eq!(roles.len(), 1);
        assert_eq!(roles[0].role.0.to_string(), MemberRole::MEMBER);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name.0, "Claude");
        assert_eq!(inviters.len(), 1);
        assert_eq!(inviters[0].inviter.0, inviter.this());
        assert!(kinds.iter().any(MemberKind::is_agent));

        // Approving it again, under another name, changes nothing.
        put_on_roster(&tonk, &key, &subject, &agent, "Renamed", &inviter).await?;
        let (memberships, roles, names, inviters, kinds) = roster(&tonk, &key, &entity).await?;
        assert_eq!(memberships.len(), 1);
        assert_eq!(roles.len(), 1);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name.0, "Claude", "the first name stands");
        assert_eq!(inviters.len(), 1);
        assert_eq!(kinds.len(), 1);

        take_off_roster(&tonk, &key, &subject, &agent).await?;
        let (memberships, roles, names, inviters, kinds) = roster(&tonk, &key, &entity).await?;
        assert!(memberships.is_empty());
        assert!(roles.is_empty());
        assert!(names.is_empty());
        assert!(inviters.is_empty());
        assert!(kinds.is_empty());
        Ok(())
    }

    /// A marked space with grants retained to it lists as an agent, with
    /// each space at the access it was given; an unmarked space does not.
    #[dialog_common::test]
    async fn it_lists_agents_with_their_access() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!("tonk-agents-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) = crate::device::open_profile_at("agents", location.clone()).await?;
        let registry = crate::device::Registry::new("agents".into(), location);
        let tonk = crate::worker::boot_state(storage, "agents".into(), profile, registry).await?;
        let state: AppState = Arc::new(crate::StateLock::new(tonk));

        let mut keys = Vec::new();
        for name in ["Notes", "Plans"] {
            keys.push(super::super::repository::create_space_inner(&state, name, None).await?);
        }
        let seed = zeroize::Zeroizing::new([71u8; 32]);
        let agent = Ed25519Signer::import(&*seed).await?.did();
        let agent_key =
            super::super::repository::create_space_from_seed(&state, "Claude", None, seed).await?;
        let remote: url::Url = "https://sync.example.test/ucan/".parse()?;
        let now = Timestamp::now();
        let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;
        {
            let tonk = state.read().await;
            tonk.reactor
                .repository(&agent_key)
                .branch(CONTENT_BRANCH)
                .transaction()
                .assert(AgentSpace::new(agent.to_string().parse()?))
                .commit()
                .perform(&tonk.operator)
                .await?;
            let mut chains = Vec::new();
            for (key, access) in keys.iter().zip([AgentAccess::Read, AgentAccess::ReadWrite]) {
                let subject = tonk
                    .profile
                    .space(key)
                    .load()
                    .perform(&tonk.operator)
                    .await?
                    .did();
                chains.extend(
                    super::super::agent_session::grant(
                        &tonk,
                        &tonk.profile.credential().signer().clone(),
                        &agent,
                        &subject,
                        access,
                        &remote,
                        now,
                        expires,
                    )
                    .await?,
                );
            }
            tonk.reactor
                .profile_repository()
                .branch(&tonk.active_branch)
                .acquire(&tonk.operator)
                .await?
                .handle()
                .delegations()
                .retain_all(chains.into_iter().map(UcanDelegation).collect())
                .perform(&tonk.operator)
                .await?;
        }

        let Json(agents) = list(State(state.clone())).await?;
        assert_eq!(agents.len(), 1, "only the marked space is an agent");
        assert_eq!(agents[0].did, agent);
        assert_eq!(agents[0].name.as_deref(), Some("Claude"));
        let mut access: Vec<_> = agents[0]
            .spaces
            .iter()
            .map(|space| (space.name.clone().unwrap_or_default(), space.access))
            .collect();
        access.sort();
        assert_eq!(
            access,
            vec![("Notes".into(), "read"), ("Plans".into(), "write")]
        );

        // A revocation's witness path is the issuer's proof for the space
        // with the grant on its end.
        let tonk = state.read().await;
        let profile = tonk.profile.did();
        for (leaf, _, _) in grants(&tonk, &agent).await? {
            let subject = leaf.subject().unwrap().clone();
            let path = ancestor(&tonk, &profile, &subject, &leaf).await?;
            assert_eq!(path.audience(), leaf.issuer());
            let path = path.push(leaf.proofs().last().unwrap().clone())?;
            assert_eq!(path.subject(), Some(&subject));
            assert_eq!(path.audience(), &agent);
        }
        drop(tonk);

        // A space the agent joined from a share link in the chat: its grant
        // was issued by the link's key into the agent space, for a space
        // this person may not belong to. It lists under the agent space's
        // directory name.
        let shared = Ed25519Signer::generate().await?;
        let seed: [u8; 32] = rand::random();
        let link_key = Ed25519Signer::import(&seed).await?;
        let root = dialog_ucan_core::DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(shared.clone()))
            .audience(&link_key.did())
            .subject(dialog_ucan_core::subject::Subject::Specific(shared.did()))
            .command(vec![])
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        // The link's key delegates straight to the agent space.
        let scopes = tonk_invite::connection::agent_grant_scopes(&shared.did(), AgentAccess::Read);
        let chains = super::super::agent_connections::issue_to_recipient(
            &agent,
            dialog_credentials::Signer::from(link_key),
            vec![DelegationChain::new(root); scopes.len()],
            &scopes,
            &remote,
            now,
            expires,
        )
        .await?
        .chains()
        .to_vec();
        {
            let tonk = state.read().await;
            super::super::hosted::access::retain_agent_grants(
                &tonk,
                &agent,
                &chains,
                &[super::super::hosted::access::DirectoryEntry {
                    space: shared.did(),
                    name: Some("Family trip".into()),
                    address: Some(remote.to_string()),
                }],
            )
            .await?;
        }
        let Json(agents) = list(State(state.clone())).await?;
        let joined = agents[0]
            .spaces
            .iter()
            .find(|space| space.subject == shared.did())
            .expect("the space joined from a link is listed");
        assert_eq!(joined.name.as_deref(), Some("Family trip"));
        assert_eq!(joined.access, "read");
        assert!(joined.via_link, "{joined:?}");
        assert!(
            agents[0]
                .spaces
                .iter()
                .filter(|space| space.subject != shared.did())
                .all(|space| !space.via_link),
            "the person's own grants are not a link's"
        );

        // Only the agent can revoke a link's grant, and only the passkey
        // opens its key: with no page to ask, nothing is removed, not even
        // from the agent space.
        let refused = revoke(
            State(state.clone()),
            Path(agent_key.clone()),
            None,
            Json(RevokeRequest {
                space: shared.did(),
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(refused, TonkWorkerError::Conflict(_)), "{refused}");
        let Json(agents) = list(State(state.clone())).await?;
        assert!(
            agents[0]
                .spaces
                .iter()
                .any(|space| space.subject == shared.did() && space.removal.is_none()),
            "the link's grant stays listed"
        );
        assert_eq!(agents[0].spaces.len(), 3);
        Ok(())
    }
}
