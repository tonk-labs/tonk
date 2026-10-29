//! A new agent and its first session, for the OAuth approval page.
//!
//! The page asks for an agent named after the LLM host and the spaces it
//! may reach. The worker draws the agent space's key, has the agent
//! space delegate to a fresh session with it, and only then creates the
//! space with that seed, which seals it to the account like any other.
//! Each chosen space is delegated to the agent space DID, read or read and
//! write, with the same leaves an agent link carries. The token holds the
//! session and those grants; it goes back to the page and from there to
//! the authorization server, and is never stored here.
use axum::{Json, extract::State};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_repository::RepositoryExt as _;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::time::Timestamp;
use dialog_varsig::{Did, Principal as _};
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_handler_macros::wasm_compat;
use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS, agent_grant_scopes};
use tonk_schema::prelude::DidExt as _;
use zeroize::Zeroizing;

use super::{
    AppState,
    agent_connections::{failure, issue_to_recipient, issuer_ancestors},
    create_invite::{RemoteRequirement, resolve_remote_url},
};
use crate::TonkWorkerError;

/// Longest agent name the page may ask for.
const MAX_NAME: usize = 80;

/// What the approval page asks for.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionRequest {
    /// The agent's name: the agent space's label and the session title.
    name: String,
    /// The spaces the agent may reach, by routing key.
    #[serde(default)]
    spaces: Vec<AgentSpaceRequest>,
}

/// One space the agent may reach, and how.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSpaceRequest {
    /// The space's routing key, as `/api/profile` lists it.
    key: String,
    /// `read` or `write` (read and write).
    access: Access,
}

#[derive(Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Access {
    Read,
    Write,
}

impl From<Access> for AgentAccess {
    fn from(access: Access) -> Self {
        match access {
            Access::Read => AgentAccess::Read,
            Access::Write => AgentAccess::ReadWrite,
        }
    }
}

/// The new agent and its session token.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionResponse {
    /// The session token, for the authorization server. Holds the
    /// session key's seed: never log or store it.
    token: String,
    /// The agent space's DID.
    agent: Did,
    /// The agent space's routing key.
    key: String,
    /// What was granted, space by space.
    spaces: Vec<AgentSpaceGranted>,
    /// When the grants lapse, unix seconds.
    expires_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSpaceGranted {
    key: String,
    subject: Did,
    access: Access,
}

impl std::fmt::Debug for AgentSessionResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSessionResponse")
            .field("token", &"[REDACTED]")
            .field("agent", &self.agent)
            .finish()
    }
}

/// `POST /api/agent/session`: create an agent space, its first session,
/// and grants to the chosen spaces.
#[wasm_compat]
pub async fn create(
    State(state): State<AppState>,
    Json(request): Json<AgentSessionRequest>,
) -> Result<Json<AgentSessionResponse>, TonkWorkerError> {
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(TonkWorkerError::Router(format!(
            "an agent needs a name of 1 to {MAX_NAME} characters"
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !request.spaces.iter().all(|space| seen.insert(&space.key)) {
        return Err(TonkWorkerError::Router(
            "each space may be chosen once".into(),
        ));
    }

    // Check every chosen space before anything is created, so a refusal
    // leaves no agent space behind.
    let now = Timestamp::now();
    let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)
        .map_err(failure)?;
    let mut targets = Vec::new();
    {
        let tonk = state.read().await;
        if super::account::provider(&tonk).await.is_none() {
            return Err(TonkWorkerError::Forbidden(
                "sign in before connecting an agent".into(),
            ));
        }
        for space in &request.spaces {
            let repository = tonk
                .profile
                .space(&space.key)
                .load()
                .perform(&tonk.operator)
                .await
                .map_err(|_| TonkWorkerError::NotFound(format!("no space {}", space.key)))?;
            let remote = match resolve_remote_url(&tonk, &repository).await? {
                RemoteRequirement::Ready(remote) => remote.access_url,
                RemoteRequirement::Refused(_) => {
                    return Err(TonkWorkerError::Conflict(format!(
                        "space {} is not synced, so a hosted agent cannot reach it",
                        space.key
                    )));
                }
            };
            targets.push((space.key.clone(), repository.did(), space.access, remote));
        }
    }

    // The agent space's key: signs the session's powerline, then is
    // sealed to the account by creating the space with it.
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(failure)?;
    let agent = Ed25519Signer::import(&*seed).await.map_err(failure)?;
    let agent_did = agent.did();

    let key = super::repository::create_space_from_seed(
        &state,
        name,
        Some("An agent's own space."),
        seed,
    )
    .await
    .map_err(failure)?;
    let remote = {
        let tonk = state.read().await;
        super::repository::account_sync_remote(&tonk).await
    }
    .ok_or_else(|| TonkWorkerError::Conflict("this account has no sync endpoint".into()))?;
    super::repository::enable_sync_inner(&state, &key, &remote)
        .await
        .map_err(failure)?;

    let tonk = state.read().await;
    let repository = tonk
        .profile
        .space(&key)
        .load()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;
    let home = match resolve_remote_url(&tonk, &repository).await? {
        RemoteRequirement::Ready(remote) => remote.access_url,
        RemoteRequirement::Refused(_) => {
            return Err(TonkWorkerError::Conflict(
                "the agent space could not be synced".into(),
            ));
        }
    };

    // The mark is what tells an agent space from any other.
    tonk.reactor
        .repository(&key)
        .branch(super::repository::CONTENT_BRANCH)
        .transaction()
        .assert(tonk_schema::agent::AgentSpace::new(
            agent_did.to_string().parse().map_err(failure)?,
        ))
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(failure)?;

    let mut grants = Vec::new();
    let mut spaces = Vec::new();
    for (key, subject, access, remote) in targets {
        grants.extend(
            grant(
                &tonk,
                &agent_did,
                &subject,
                access.into(),
                &remote,
                now,
                expires,
            )
            .await?,
        );
        spaces.push(AgentSpaceGranted {
            key,
            subject,
            access,
        });
    }

    // Retained so the account can find and revoke them later, the way it
    // keeps the grants of an agent link.
    if !grants.is_empty() {
        tonk.reactor
            .profile_repository()
            .branch(&tonk.active_branch)
            .acquire(&tonk.operator)
            .await
            .map_err(failure)?
            .handle()
            .delegations()
            .retain_all(
                grants
                    .iter()
                    .cloned()
                    .map(UcanDelegation)
                    .collect::<Vec<_>>(),
            )
            .perform(&tonk.operator)
            .await
            .map_err(failure)?;
    }

    let issued =
        tonk_identity::agent::issue_agent_session_with_grants(Signer::from(agent), &home, &grants)
            .await
            .map_err(failure)?;
    tonk_common::log!(
        "agent {} created with {} space grant(s)",
        agent_did,
        spaces.len()
    );
    Ok(Json(AgentSessionResponse {
        token: issued.token,
        agent: agent_did.clone(),
        key: agent_did.repo_key().to_owned(),
        spaces,
        expires_at: expires.to_unix(),
    }))
}

/// The chains delegating `subject` to `agent` at `access`: one per scope,
/// each the account's own proof for the space with a leaf from the
/// profile to the agent space, the shape an agent link's grants have.
async fn grant(
    tonk: &crate::TonkState,
    agent: &Did,
    subject: &Did,
    access: AgentAccess,
    remote: &url::Url,
    now: Timestamp,
    expires: Timestamp,
) -> Result<Vec<dialog_ucan_core::DelegationChain>, TonkWorkerError> {
    let scopes = agent_grant_scopes(subject, access);
    let ancestors = issuer_ancestors(tonk, &scopes, now, expires).await?;
    let bundle = issue_to_recipient(
        agent,
        tonk.profile.credential().signer().clone(),
        ancestors,
        &scopes,
        remote,
        now,
        expires,
    )
    .await?;
    Ok(bundle.chains().to_vec())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_effects::storage::Directory;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// A token minted the way the approval page's request mints one is
    /// one the hosted front accepts, with the access that was chosen.
    #[dialog_common::test]
    async fn it_mints_grants_the_hosted_front_accepts() -> anyhow::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("tonk-agent-session-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&directory)?;
        let location = Directory::At(directory.to_string_lossy().into_owned());
        let (storage, profile) =
            crate::device::open_profile_at("agent-session", location.clone()).await?;
        let registry = crate::device::Registry::new("agent-session".into(), location);
        let tonk =
            crate::worker::boot_state(storage, "agent-session".into(), profile, registry).await?;
        let state: AppState = Arc::new(RwLock::new(tonk));

        let mut keys = Vec::new();
        for name in ["Notes", "Plans"] {
            keys.push(super::super::repository::create_space_inner(&state, name, None).await?);
        }
        let agent = Ed25519Signer::import(&[61; 32]).await?;
        let remote: url::Url = "https://sync.example.test/ucan/".parse()?;
        let now = Timestamp::now();
        let expires = Timestamp::try_from((now.to_unix() + DEFAULT_GRANT_TTL_SECONDS) as i128)?;

        let tonk = state.read().await;
        let mut subjects = Vec::new();
        let mut grants = Vec::new();
        for (key, access) in keys.iter().zip([AgentAccess::Read, AgentAccess::ReadWrite]) {
            let subject = tonk
                .profile
                .space(key)
                .load()
                .perform(&tonk.operator)
                .await?
                .did();
            grants
                .extend(grant(&tonk, &agent.did(), &subject, access, &remote, now, expires).await?);
            subjects.push(subject);
        }
        assert_eq!(grants.len(), 3 + 6);

        let issued = tonk_identity::agent::issue_agent_session_with_grants(
            Signer::from(agent.clone()),
            &remote,
            &grants,
        )
        .await?;
        let verified = super::super::hosted::mcp::verify(&issued.token)
            .await
            .map_err(anyhow::Error::msg)?;
        let read = &verified.spaces()[&subjects[0]];
        assert!(read.read && !read.write, "read only");
        let write = &verified.spaces()[&subjects[1]];
        assert!(write.read && write.write, "read and write");
        assert!(verified.spaces()[&agent.did()].agent);
        Ok(())
    }
}
