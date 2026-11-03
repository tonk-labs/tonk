//! The account's agents, for the hub's "your agents" tab.
//!
//! An agent is a space marked [`AgentSpace`]; what it can reach is the
//! grants this profile issued to that space's DID, as retained in the
//! profile or in the agent's own space. The list reads the delegations
//! themselves, not a summary of them.
//!
//! The same mark also sits in the account directory, beside the space's
//! directory row, so that every device of the account knows which spaces
//! are agents. A device mounts a marked agent space it does not have yet,
//! and that pulls the grants the space holds.
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

/// The agent spaces the account directory marks.
async fn directory_agents(tonk: &TonkState) -> Result<Vec<Did>, TonkWorkerError> {
    let session = tonk
        .reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .acquire(&tonk.operator)
        .await
        .map_err(failure)?;
    let rows = session
        .handle()
        .query()
        .select(Query::<AgentSpace> {
            this: Term::var("this"),
            kind: Term::from(tonk_schema::agent::agent_space_kind()),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(failure)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.this.to_string().parse().ok())
        .collect())
}

/// Mark `agent` as an agent space in the account directory, which every
/// device of the account reads.
pub(super) async fn mark_in_directory(
    tonk: &TonkState,
    agent: &Did,
) -> Result<(), TonkWorkerError> {
    tonk.reactor
        .profile_repository()
        .branch(&tonk.active_branch)
        .transaction()
        .assert(AgentSpace::new(agent.to_string().parse().map_err(failure)?))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    Ok(())
}

/// The account's agent spaces. An agent approved on another device is
/// known here only by its directory mark, so it is mounted now. An agent
/// space on this device without the directory mark gets it, so the other
/// devices find it too.
async fn agent_spaces(tonk: &TonkState) -> Result<Vec<Did>, TonkWorkerError> {
    let marked = directory_agents(tonk).await?;
    let mut mounted = spaces(tonk).await?;
    for did in &marked {
        if mounted.contains(did) {
            continue;
        }
        match super::adopt::ensure_space_mounted(tonk, did.as_str()).await {
            Ok(true) => mounted.push(did.clone()),
            Ok(false) => tonk_common::log!("agent {did} is in the directory but cannot be mounted"),
            Err(error) => tonk_common::log!("agent {did} could not be mounted: {error}"),
        }
    }
    let mut agents = Vec::new();
    for did in mounted {
        if marked.contains(&did) {
            agents.push(did);
        } else if is_agent(tonk, &did).await {
            if let Err(error) = mark_in_directory(tonk, &did).await {
                tonk_common::log!("agent {did} was not marked in the directory: {error}");
            }
            agents.push(did);
        }
    }
    Ok(agents)
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
enum Held {
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
/// hold no proof to revoke. A link's grant is removed by dropping it from
/// the agent space, the only place the hosted agent finds it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Issued {
    Here,
    Link,
}

/// The live grants to `agent`, each a one-certificate chain as retained,
/// read from both places a grant may be held: those this profile issued,
/// and those a share link issued into the agent space.
async fn grants(
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
    for did in agent_spaces(&tonk).await? {
        let mut by_space = std::collections::BTreeMap::<String, (Did, bool, Option<u64>)>::new();
        for (chain, _, _) in grants(&tonk, &did).await? {
            let subject = chain.subject().unwrap().clone();
            let write = chain.ability().starts_with("/use/put");
            let expires = chain.expiration().map(|at| at.to_unix());
            let entry = by_space
                .entry(subject.to_string())
                .or_insert((subject, false, expires));
            entry.1 |= write;
            entry.2 = match (entry.2, expires) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        let mut spaces = Vec::new();
        for (_, (subject, write, expires_at)) in by_space {
            // A space joined from a link may not be one of the person's:
            // the agent space's directory names it then.
            let name = match name(&tonk, &subject).await {
                Some(name) => Some(name),
                None => directory_name(&tonk, &did, &subject).await,
            };
            spaces.push(AgentSpaceAccess {
                key: subject.repo_key().to_owned(),
                name,
                subject,
                access: if write { "write" } else { "read" },
                expires_at,
            });
        }
        agents.push(AgentSummary {
            key: did.repo_key().to_owned(),
            name: name(&tonk, &did).await,
            did,
            spaces,
        });
    }
    Ok(Json(agents))
}

/// `POST /api/agents/{key}/revoke`: withdraw every grant the agent holds
/// to one space. Each grant is revoked at the space's access service,
/// which checks every request the agent makes, and only then dropped
/// from the profile, so a failure leaves it listed to retry.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(request): Json<RevokeRequest>,
) -> Result<Json<()>, TonkWorkerError> {
    let tonk = state.read().await;
    let agent = agent_spaces(&tonk)
        .await?
        .into_iter()
        .find(|did| did.repo_key() == key)
        .ok_or_else(|| TonkWorkerError::NotFound(format!("no agent {key}")))?;
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
    if !links.is_empty() {
        // Dropped from the agent space, which the hosted agent reads its
        // access from on every call, then sent there by the background
        // sync, which retries.
        let session = tonk
            .reactor
            .repository(agent.repo_key())
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .map_err(failure)?;
        for (leaf, _, _) in links {
            session
                .handle()
                .delegations()
                .retract(UcanDelegation(leaf))
                .perform(&tonk.operator)
                .await
                .map_err(failure)?;
        }
        tonk.sync_queue
            .mark_dirty(agent.repo_key(), super::sync::now_millis());
    }
    if leaves.is_empty() {
        tonk_common::log!("agent {} link access to {} removed", agent, request.space);
        return Ok(Json(()));
    }
    let repository = tonk
        .profile
        .space(request.space.repo_key())
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(|_| TonkWorkerError::NotFound(format!("no space {}", request.space)))?;
    let remote = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => {
            return Err(TonkWorkerError::Conflict(
                "the space is not synced, so there is no access service to tell".into(),
            ));
        }
    };
    let profile = tonk.profile.did();
    for (leaf, held, _) in leaves {
        let target = *leaf
            .proof_cids()
            .last()
            .ok_or_else(|| failure("empty grant"))?;
        let certificate = leaf.proofs().last().unwrap().clone();
        let path = ancestor(&tonk, &profile, &request.space, &leaf)
            .await?
            .push(certificate)
            .map_err(failure)?;
        publish_target(&tonk, &remote, &path, &target).await?;
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
    tonk_common::log!("agent {} access to {} revoked", agent, request.space);
    Ok(Json(()))
}

/// The profile's own proof for the scope `leaf` was issued under: the
/// path a revocation of `leaf` is witnessed by.
async fn ancestor(
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

        // Listing marks the agent in the directory for the other devices.
        assert_eq!(
            directory_agents(&*state.read().await).await?,
            vec![agent.clone()]
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

        // A space the agent joined from a share link in the chat: its
        // grant was issued by the link's key into the agent space, for a
        // space this person may not belong to. It lists under the agent
        // space's directory name, and revoking drops it from there.
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
        let invite = tonk_invite::Invite::new(
            DelegationChain::new(root),
            tonk_invite::InviteAudience::Open { seed },
            Some(remote.clone()),
        )
        .await?;
        let offer = super::super::hosted::join::Offer::of(invite).map_err(anyhow::Error::msg)?;
        let chains = offer
            .grant(&agent, AgentAccess::Read, now)
            .await
            .map_err(anyhow::Error::msg)?;
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

        let Json(()) = revoke(
            State(state.clone()),
            Path(agent_key.clone()),
            Json(RevokeRequest {
                space: shared.did(),
            }),
        )
        .await?;
        let Json(agents) = list(State(state.clone())).await?;
        assert!(
            agents[0]
                .spaces
                .iter()
                .all(|space| space.subject != shared.did()),
            "the link's grant is gone"
        );
        assert_eq!(agents[0].spaces.len(), 2, "the person's own grants stay");
        Ok(())
    }

    /// An agent approved on another device is known here only by its
    /// directory row and mark. Listing mounts it and lists it.
    #[dialog_common::test]
    async fn it_lists_an_agent_another_device_approved() -> anyhow::Result<()> {
        use dialog_repository::SiteAddress;

        let state = crate::router::command::tests::native::test_state().await;
        let agent = Ed25519Signer::generate().await?.did();
        let address = SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
            "https://sync.example.test/ucan/",
        ));
        let configuration = super::super::repository::RepositoryConfiguration::default()
            .remote(
                "origin",
                super::super::repository::RemoteConfiguration::new(address).subject(agent.clone()),
            )
            .branch(
                "main",
                super::super::repository::BranchConfiguration::default().upstream("origin", "main"),
            );
        {
            let tonk = state.read().await;
            super::super::repository::record_space_mount(
                &tonk,
                &agent,
                &configuration,
                Some("Claude"),
            )
            .await;
            mark_in_directory(&tonk, &agent).await?;
            assert!(!super::super::join::find_replica_for_subject(&tonk, &agent).await?);
        }

        let Json(agents) = list(State(state.clone())).await?;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].did, agent);
        assert!(
            super::super::join::find_replica_for_subject(&*state.read().await, &agent).await?,
            "listing mounts the agent space, which pulls its grants"
        );
        Ok(())
    }
}
