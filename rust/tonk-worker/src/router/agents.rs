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
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::{DelegationChain, time::Timestamp};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use tonk_macros::handler;
use tonk_schema::{
    Replica,
    agent::{AgentProvider, AgentSpace},
    domain::replica::Profile as ProfileEntity,
    prelude::DidExt as _,
};

use super::{
    AppState,
    agent_connections::{failure, publish_target},
    agent_space::{self, AgentSpaceChange, AgentSpaceRecord},
    create_invite::{RemoteRequirement, resolve_remote_url},
    repository::{CONTENT_BRANCH, repository_display_name, space_named},
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

/// Whether the space at `did` carries the agent-space mark. Where the
/// space has a worker of its own, the profile's mark of it says so.
pub(super) async fn is_agent(tonk: &TonkState, did: &Did) -> bool {
    if tonk.spaces_elsewhere() {
        return profile_marks(tonk)
            .await
            .is_ok_and(|marked| marked.contains(did.as_str()));
    }
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

pub(super) async fn name(tonk: &TonkState, did: &Did) -> Option<String> {
    let key = did.repo_key();
    let repository = space_named(tonk, key).await.ok()?;
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

/// The delegations to `agent` this profile retains.
async fn retained(tonk: &TonkState, agent: &Did) -> Result<Vec<DelegationChain>, TonkWorkerError> {
    Ok(tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?
        .handle()
        .delegations()
        .issued_to(agent.clone())
        .perform(&tonk.operator)
        .await
        .map_err(failure)?
        .into_iter()
        .map(|UcanDelegation(chain)| chain)
        .collect())
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
/// and those a share link issued into the agent space, which `space`
/// records.
pub(super) async fn grants(
    tonk: &TonkState,
    agent: &Did,
    space: &AgentSpaceRecord,
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
    let in_space = space
        .grants
        .iter()
        .map(|grant| grant.chain())
        .collect::<Result<Vec<_>, _>>()?;
    for (held, chains) in [
        (Held::AgentSpace, in_space),
        (Held::Profile, retained(tonk, agent).await?),
    ] {
        for chain in chains {
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
#[handler]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AgentSummary>>, TonkWorkerError> {
    // A clone, not a guard: an agent's space may be asked of its own
    // worker.
    let tonk = state.read().await.clone();
    let mut agents = Vec::new();
    let mut marked = Vec::new();
    for did in spaces(&tonk).await? {
        if !is_agent(&tonk, &did).await {
            continue;
        }
        marked.push(did.clone());
        let record = agent_space::read(&tonk, did.repo_key()).await?;
        let mut by_space =
            std::collections::BTreeMap::<String, (Did, bool, Option<u64>, bool)>::new();
        for (chain, _, issued) in grants(&tonk, &did, &record).await? {
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
                None => record.names.get(subject.as_str()).cloned(),
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
    // An agent made before the profile held the mark gets it now, so the
    // palette can name it. The listing stands whether or not it lands.
    if let Err(error) = mark_on_profile(&tonk, &marked, None).await {
        tonk_common::log!("agents: the profile did not take the agent marks: {error}");
    }
    Ok(Json(agents))
}

/// The agents the profile marks as agents, by DID.
async fn profile_marks(
    tonk: &TonkState,
) -> Result<std::collections::BTreeSet<String>, TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(session
        .handle()
        .query()
        .select(Query::<AgentSpace> {
            this: Term::var("this"),
            kind: Term::from(tonk_schema::agent::agent_space_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?
        .into_iter()
        .map(|row| row.this.to_string())
        .collect())
}

/// Mark each of `agents` as an agent on the profile too, on its
/// directory entity, as its own space marks it: what the profile's
/// `agent` concept reads, and so what the palette's "invite" can name.
/// With `provider`, note which provider's apps they are for, which an app
/// is offered an agent again by. Writes only what the profile does not
/// hold yet.
pub(super) async fn mark_on_profile(
    tonk: &TonkState,
    agents: &[Did],
    provider: Option<&str>,
) -> Result<(), TonkWorkerError> {
    let held = profile_marks(tonk).await?;
    let missing: Vec<&Did> = agents
        .iter()
        .filter(|agent| !held.contains(agent.as_str()))
        .collect();
    if missing.is_empty() && provider.is_none() {
        return Ok(());
    }
    let mut transaction = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction();
    for agent in missing {
        transaction = transaction.assert(AgentSpace::new(agent.this()));
    }
    if let Some(provider) = provider {
        for agent in agents {
            transaction = transaction.assert(AgentProvider::new(agent.this(), provider));
        }
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(())
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
#[handler]
pub async fn revoke(
    State(state): State<AppState>,
    Path(key): Path<String>,
    client: Option<axum::Extension<super::ClientId>>,
    Json(request): Json<RevokeRequest>,
) -> Result<Json<RevokeAnswer>, TonkWorkerError> {
    let tonk = state.read().await.clone();
    let agent = find(&tonk, &key).await?;
    let record = agent_space::read(&tonk, &key).await?;
    let (leaves, links): (Vec<_>, Vec<_>) = grants(&tonk, &agent, &record)
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
    let asked = super::agent_session::ask_for_passkey(
        &client,
        tonk_schema::ceremony::REVOKE_AGENT,
        tonk_worker_api::CustodyIntent::RevokeAgent(tonk_worker_api::AgentRevocation {
            agent: key.clone(),
            space: space.clone(),
        }),
    )
    .await;
    if let Err(error) = asked {
        withdraw::note(&key, &space, None);
        return Err(error);
    }
    Ok(Json(RevokeAnswer { pending: true }))
}

/// Revoke `leaves`, grants to `agent` for `space` that the profile or the
/// account root issued, with the profile's own proof, and drop each once
/// the access service has acknowledged it.
async fn revoke_issued(
    tonk: &TonkState,
    agent: &Did,
    space: &Did,
    leaves: Vec<(DelegationChain, Held, Issued)>,
) -> Result<(), TonkWorkerError> {
    let repository = space_named(tonk, space.repo_key())
        .await
        .map_err(|_| TonkWorkerError::NotFound(format!("no space {}", space)))?;
    let remote = match resolve_remote_url(tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => {
            return Err(TonkWorkerError::Conflict(
                "the space is not synced, so there is no access service to tell".into(),
            ));
        }
    };
    let profile = tonk.profile.did();
    let mut forget = AgentSpaceChange::default();
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
        publish_target(tonk, &remote, &path, &target).await?;
        match held {
            Held::Profile => {
                tonk.reactor
                    .profile_repository()
                    .branch(&tonk.active_branch)
                    .acquire(&tonk.operator)
                    .await
                    .map_err(failure)?
                    .handle()
                    .delegations()
                    .retract(UcanDelegation(leaf))
                    .perform(&tonk.operator)
                    .await
                    .map_err(failure)?;
            }
            Held::AgentSpace => forget.forget.push(agent_space::encode(&leaf)?),
        }
    }
    // The hosted agent lists its spaces from the agent space: send the
    // removal there too. The access service already refuses the grant.
    if !forget.forget.is_empty() {
        agent_space::change(tonk, agent.repo_key(), &forget).await?;
        tonk.sync_queue
            .mark_dirty(agent.repo_key(), super::sync::now_millis());
    }
    tonk_common::log!("agent {} access to {} revoked", agent, space);
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
    use dialog_repository::RepositoryExt as _;
    use dialog_varsig::Principal as _;
    use std::sync::Arc;
    use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS};

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
        let record = agent_space::read(&tonk, agent.repo_key()).await?;
        for (leaf, _, _) in grants(&tonk, &agent, &record).await? {
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
