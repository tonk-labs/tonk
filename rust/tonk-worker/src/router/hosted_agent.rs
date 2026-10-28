//! Hosted agents: one persistent identity per agent a person sponsors.
//!
//! A hosted MCP server keeps each agent in its own private directory under
//! one root. Each directory is a [`LocalPeer`]: its key is generated there on
//! first use and never comes from a link. The person's sponsorship (see
//! [`tonk_invite::sponsorship`]) is verified against that key and retained in
//! the peer's delegation store, so the agent can later present it to
//! provision spaces billed to the sponsor.
//!
//! Not a public route: the host invokes it once per process with
//! `tonk-agent-worker`.
use std::path::{Path, PathBuf};

use dialog_ucan_core::time::Timestamp;
use dialog_varsig::Did;
use serde_json::{Value, json};
use tonk_invite::sponsorship::AgentSponsorship;

use super::local_peer::LocalPeer;
use tonk_schema::prelude::DidExt as _;

/// The non-authority facts about a sponsorship, kept beside the agent. The
/// chain itself is retained in the peer's delegation store.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SponsorRecord {
    sponsor: String,
    recipient: String,
    expires_at: u64,
    /// The verified chain, presented as proof when provisioning. Public.
    sponsorship_hex: String,
}

const SPONSOR_FILE: &str = "sponsor.json";

/// How long an agent may wait for its sponsor. OAuth gives an attempt five
/// minutes, so an agent still unsponsored after this can never be sponsored.
const ABANDONED_AFTER: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// How many agents may wait for a sponsor at once. Each one is checkpointed,
/// so unapproved attempts must not grow storage without bound.
const MAX_WAITING: usize = 32;

/// The agents one hosted service keeps, under one private root.
pub struct HostedAgents {
    root: PathBuf,
    remote: url::Url,
    max_waiting: usize,
}

impl HostedAgents {
    /// Agents under `root`, syncing with `remote` (the provider's UCAN
    /// endpoint). The caller serializes invocations.
    pub fn new(root: PathBuf, remote: url::Url) -> Self {
        Self {
            root,
            remote,
            max_waiting: MAX_WAITING,
        }
    }

    /// Run one agent tool. Errors are fixed strings that never echo input.
    pub async fn call(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
        let text = |name: &str| arguments.get(name).and_then(Value::as_str);
        match tool {
            "agent_create" => self.create().await,
            "agent_identity" => self.identity(text("agent").ok_or(INVALID)?).await,
            "agent_sponsor" => {
                self.sponsor(
                    text("agent").ok_or(INVALID)?,
                    text("sponsorship_hex").ok_or(INVALID)?,
                    text("recipient").ok_or(INVALID)?,
                    Timestamp::now(),
                )
                .await
            }
            "agent_call" => {
                let call = text("call").ok_or(INVALID)?;
                let mut forwarded = serde_json::Map::new();
                for field in ["space", "document"] {
                    if let Some(value) = text(field) {
                        forwarded.insert(field.to_owned(), Value::String(value.to_owned()));
                    }
                }
                self.call_peer(
                    text("agent").ok_or(INVALID)?,
                    call,
                    &Value::Object(forwarded),
                )
                .await
            }
            "agent_create_space" => {
                self.create_space(
                    text("agent").ok_or(INVALID)?,
                    text("name").ok_or(INVALID)?,
                    text("agent_name").ok_or(INVALID)?,
                )
                .await
            }
            _ => Err("Tool unavailable.".into()),
        }
    }

    /// Make a new agent with a fresh key. Abandoned agents are reclaimed
    /// first, and a new one is refused while too many wait for a sponsor.
    pub async fn create(&self) -> Result<Value, String> {
        if self.reclaim(std::time::SystemTime::now())? >= self.max_waiting {
            return Err("Too many agents are waiting for approval. Try again later.".into());
        }
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(|_| UNAVAILABLE)?;
        let id = hex::encode(id);
        let directory = self.directory(&id)?;
        let _lock = lock(&directory)?;
        let peer = LocalPeer::open(directory).await?;
        let did = peer.did().to_owned();
        peer.close().await;
        Ok(json!({ "agent": id, "did": did }))
    }

    /// The agent's DID, and its sponsor if it has one.
    pub async fn identity(&self, id: &str) -> Result<Value, String> {
        let directory = self.existing(id)?;
        let _lock = lock(&directory)?;
        let peer = LocalPeer::open(directory.clone()).await?;
        let did = peer.did().to_owned();
        peer.close().await;
        let sponsor = read_record(&directory)?;
        Ok(json!({
            "agent": id,
            "did": did,
            "sponsor": sponsor.as_ref().map(|record| &record.sponsor),
            "recipient": sponsor.as_ref().map(|record| &record.recipient),
            "expiresAt": sponsor.as_ref().map(|record| record.expires_at),
        }))
    }

    /// Verify a sponsorship of this agent and retain it.
    pub async fn sponsor(
        &self,
        id: &str,
        sponsorship_hex: &str,
        recipient: &str,
        now: Timestamp,
    ) -> Result<Value, String> {
        let directory = self.existing(id)?;
        let recipient: Did = recipient.parse().map_err(|_| INVALID)?;
        tonk_identity::sealed::RecipientKey::try_from(&recipient).map_err(|_| INVALID)?;
        let bytes = hex::decode(sponsorship_hex).map_err(|_| INVALID)?;
        let sponsorship = AgentSponsorship::from_bytes(&bytes).map_err(|_| INVALID)?;
        let _lock = lock(&directory)?;
        let peer = LocalPeer::open(directory.clone()).await?;
        let agent: Did = peer.did().parse().map_err(|_| UNAVAILABLE)?;
        let verified = match sponsorship.verify(&agent, now).await {
            Ok(verified) => verified,
            Err(_) => {
                peer.close().await;
                return Err("The sponsorship is not valid for this agent.".into());
            }
        };
        let retained = peer.retain(sponsorship.chain().clone()).await;
        peer.close().await;
        retained?;
        let record = SponsorRecord {
            sponsor: verified.sponsor.to_string(),
            recipient: recipient.to_string(),
            expires_at: verified.expires_at.to_unix(),
            sponsorship_hex: sponsorship_hex.to_owned(),
        };
        write_record(&directory, &record)?;
        Ok(json!({
            "agent": id,
            "did": agent.to_string(),
            "sponsor": record.sponsor,
            "expiresAt": record.expires_at,
        }))
    }

    /// Create a space the agent's sponsor owns, provision it under the
    /// sponsor, push it, and answer the link the sponsor opens to adopt it.
    pub async fn create_space(
        &self,
        id: &str,
        name: &str,
        agent_name: &str,
    ) -> Result<Value, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err("Give the space a name of up to 120 characters.".into());
        }
        let agent_name = agent_name.trim();
        if agent_name.is_empty() || agent_name.chars().count() > 120 {
            return Err(INVALID.into());
        }
        let directory = self.existing(id)?;
        let record =
            read_record(&directory)?.ok_or_else(|| "This agent has no sponsor.".to_owned())?;
        let now = Timestamp::now();
        if record.expires_at <= now.to_unix() {
            return Err("This agent's sponsorship has expired; reconnect to renew it.".into());
        }
        let sponsor: Did = record.sponsor.parse().map_err(|_| UNAVAILABLE)?;
        let recipient: Did = record.recipient.parse().map_err(|_| UNAVAILABLE)?;
        let recipient =
            tonk_identity::sealed::RecipientKey::try_from(&recipient).map_err(|_| UNAVAILABLE)?;
        let sponsorship = AgentSponsorship::from_bytes(
            &hex::decode(&record.sponsorship_hex).map_err(|_| UNAVAILABLE)?,
        )
        .map_err(|_| UNAVAILABLE)?;
        let grants_expire =
            Timestamp::try_from(record.expires_at as i128).map_err(|_| UNAVAILABLE)?;

        let _lock = lock(&directory)?;
        let peer = LocalPeer::open(directory.clone()).await?;
        let outcome = self
            .create_with(
                &peer,
                &sponsor,
                &recipient,
                &sponsorship,
                name,
                agent_name,
                grants_expire,
            )
            .await;
        peer.close().await;
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_with(
        &self,
        peer: &LocalPeer,
        sponsor: &Did,
        recipient: &tonk_identity::sealed::RecipientKey,
        sponsorship: &AgentSponsorship,
        name: &str,
        agent_name: &str,
        grants_expire: Timestamp,
    ) -> Result<Value, String> {
        let state = peer.sync_state();
        let created = {
            let tonk = state.read().await;
            super::repository::create_sponsored_space(
                &tonk,
                sponsor,
                recipient,
                name,
                agent_name,
                &self.remote,
                grants_expire,
            )
            .await
            .map_err(|_| "The space could not be created.".to_owned())?
        };

        // Provision under the sponsor before anything is pushed: the service
        // serves only provisioned spaces.
        let invocation = {
            let tonk = state.read().await;
            tonk_identity::request::build_provider_add_invocation(
                tonk.profile.credential().signer().clone(),
                sponsorship.chain(),
                &created.did,
                &created.founder,
                None,
            )
            .await
            .map_err(|_| "The space could not be registered.".to_owned())?
        };
        super::http::post_cbor(&self.remote, &invocation)
            .await
            .map_err(|_| {
                "The space could not be registered under the sponsor's account.".to_owned()
            })?;

        let mut router = peer.router();
        let key = created.did.repo_key().to_owned();
        super::agent_link::request_detailed(
            &mut router,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            "",
            "POST",
        )
        .await
        .map_err(|_| "The space was created but could not be published yet.".to_owned())?;

        let claim = claim_url(&self.remote, created.founder, &created.sealed_seed).await?;
        Ok(json!({
            "space": created.did.to_string(),
            "name": name,
            "claim_url": claim,
        }))
    }

    /// Run one of the agent's own space tools on its peer: list its spaces,
    /// or query or evaluate one of them. The peer pulls before it reads,
    /// journals a write, and pushes after it.
    pub async fn call_peer(
        &self,
        id: &str,
        call: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        if !["tonk_list_spaces", "tonk_query", "tonk_evaluate"].contains(&call) {
            return Err("Tool unavailable.".into());
        }
        let directory = self.existing(id)?;
        if read_record(&directory)?.is_none() {
            return Err("This agent has no sponsor.".into());
        }
        let _lock = lock(&directory)?;
        let mut peer = LocalPeer::open(directory).await?;
        let outcome = peer.call(call, arguments).await;
        peer.close().await;
        outcome
    }

    /// Remove agents that were never sponsored and are older than
    /// [`ABANDONED_AFTER`]. Returns how many unsponsored agents remain.
    fn reclaim(&self, now: std::time::SystemTime) -> Result<usize, String> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        let mut waiting = 0;
        for entry in entries {
            let entry = entry.map_err(|_| UNAVAILABLE)?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(directory) = self.directory(&id) else {
                continue;
            };
            if !directory.is_dir() || directory.join(SPONSOR_FILE).exists() {
                continue;
            }
            let age = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok());
            if age.is_some_and(|age| age >= ABANDONED_AFTER) {
                // A busy agent is in use: leave it for the next sweep.
                if let Ok(Some(_lock)) = super::local_peer::try_lock(&directory)
                    && std::fs::remove_dir_all(&directory).is_ok()
                {
                    continue;
                }
            }
            waiting += 1;
        }
        Ok(waiting)
    }

    fn directory(&self, id: &str) -> Result<PathBuf, String> {
        // 32 lowercase hex characters, as `create` makes them: never a path.
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(INVALID.into());
        }
        Ok(self.root.join(id))
    }

    fn existing(&self, id: &str) -> Result<PathBuf, String> {
        let directory = self.directory(id)?;
        if !directory.is_dir() {
            return Err("Unknown agent.".into());
        }
        Ok(directory)
    }
}

/// The link the sponsor opens to adopt a space: an ordinary Scoped invite
/// carrying `space -> sponsor`, which the sponsor's join redeems as is, and
/// the sealed seed in the fragment, which never reaches a server.
async fn claim_url(
    remote: &url::Url,
    founder: dialog_ucan_core::DelegationChain,
    sealed_seed: &[u8],
) -> Result<String, String> {
    let join = remote.join("/join").map_err(|_| UNAVAILABLE)?;
    let invite = tonk_invite::Invite::new(
        founder,
        tonk_invite::InviteAudience::Scoped,
        Some(remote.clone()),
    )
    .await
    .map_err(|_| UNAVAILABLE)?;
    let url = invite.to_url(join.as_str()).map_err(|_| UNAVAILABLE)?;
    Ok(format!(
        "{url}#seed={}",
        bs58::encode(sealed_seed).into_string()
    ))
}

/// Hold the agent's directory for this invocation, as [`LocalPeer`] requires.
fn lock(directory: &Path) -> Result<std::fs::File, String> {
    super::local_peer::try_lock(directory)
        .map_err(|_| "The agent's storage is not private or cannot be locked.".to_owned())?
        .ok_or_else(|| "The agent is busy.".to_owned())
}

const INVALID: &str = "Invalid agent invocation.";
const UNAVAILABLE: &str = "Agent storage is unavailable.";

fn read_record(directory: &Path) -> Result<Option<SponsorRecord>, String> {
    match std::fs::read(directory.join(SPONSOR_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "The agent's sponsor record is unreadable.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("The agent's sponsor record could not be read.".into()),
    }
}

fn write_record(directory: &Path, record: &SponsorRecord) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let staged = directory.join(format!("{SPONSOR_FILE}.tmp"));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&staged)
        .map_err(|_| UNAVAILABLE)?;
    file.write_all(&serde_json::to_vec(record).expect("record serializes"))
        .and_then(|()| file.sync_all())
        .map_err(|_| UNAVAILABLE)?;
    std::fs::rename(&staged, directory.join(SPONSOR_FILE)).map_err(|_| UNAVAILABLE.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject};
    use dialog_varsig::Principal as _;

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("hosted-agents-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// What the sponsor's device sends: `account -> device -> agent`.
    async fn sponsorship_for(agent: &Did) -> (Did, String) {
        let account = Ed25519Signer::generate().await.unwrap();
        let device = Ed25519Signer::generate().await.unwrap();
        let link = DelegationBuilder::new()
            .issuer(Signer::from(account.clone()))
            .audience(&device.did())
            .subject(Subject::Any)
            .command(Vec::<String>::new())
            .try_build()
            .await
            .unwrap();
        let sponsorship = AgentSponsorship::issue_from_device(
            DelegationChain::new(link),
            &Signer::from(device),
            agent,
            Timestamp::now(),
        )
        .await
        .unwrap();
        (account.did(), hex::encode(sponsorship.to_bytes().unwrap()))
    }

    fn recipient() -> String {
        tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([5u8; 32]))
            .secret()
            .did()
            .to_string()
    }

    #[dialog_common::test]
    async fn it_keeps_one_identity_per_agent_across_invocations() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        let created = agents.create().await.unwrap();
        let id = created["agent"].as_str().unwrap();
        let again = agents.identity(id).await.unwrap();
        assert_eq!(again["did"], created["did"]);
        assert_eq!(again["sponsor"], Value::Null);
        let other = agents.create().await.unwrap();
        assert_ne!(other["did"], created["did"]);
    }

    #[dialog_common::test]
    async fn it_retains_a_valid_sponsorship_of_the_agent() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        let created = agents.create().await.unwrap();
        let id = created["agent"].as_str().unwrap();
        let did: Did = created["did"].as_str().unwrap().parse().unwrap();
        let (account, hex) = sponsorship_for(&did).await;
        let sponsored = agents
            .sponsor(id, &hex, &recipient(), Timestamp::now())
            .await
            .unwrap();
        assert_eq!(sponsored["sponsor"], account.to_string());
        let identity = agents.identity(id).await.unwrap();
        assert_eq!(identity["sponsor"], account.to_string());
        assert_eq!(identity["recipient"], recipient());
    }

    #[dialog_common::test]
    async fn it_refuses_a_sponsorship_of_another_agent() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        let created = agents.create().await.unwrap();
        let id = created["agent"].as_str().unwrap();
        let stranger = Ed25519Signer::generate().await.unwrap().did();
        let (_, hex) = sponsorship_for(&stranger).await;
        let error = agents
            .sponsor(id, &hex, &recipient(), Timestamp::now())
            .await
            .unwrap_err();
        assert_eq!(error, "The sponsorship is not valid for this agent.");
        assert_eq!(agents.identity(id).await.unwrap()["sponsor"], Value::Null);
    }

    #[dialog_common::test]
    async fn it_refuses_ids_that_are_not_agents() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        for id in ["../escape", "", "ABCDEF", &"0".repeat(32)] {
            assert!(agents.identity(id).await.is_err(), "accepted {id:?}");
        }
        let (_, hex) = sponsorship_for(&Ed25519Signer::generate().await.unwrap().did()).await;
        assert!(
            agents
                .sponsor(&"0".repeat(32), &hex, "did:key:nope", Timestamp::now())
                .await
                .is_err()
        );
    }

    #[dialog_common::test]
    async fn it_reclaims_agents_that_were_never_sponsored() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        let abandoned = agents.create().await.unwrap();
        let sponsored = agents.create().await.unwrap();
        let id = sponsored["agent"].as_str().unwrap();
        let did: Did = sponsored["did"].as_str().unwrap().parse().unwrap();
        let (_, hex) = sponsorship_for(&did).await;
        agents
            .sponsor(id, &hex, &recipient(), Timestamp::now())
            .await
            .unwrap();

        let later = std::time::SystemTime::now() + ABANDONED_AFTER;
        assert_eq!(agents.reclaim(later).unwrap(), 0);
        let error = agents
            .identity(abandoned["agent"].as_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error, "Unknown agent.");
        assert_eq!(agents.identity(id).await.unwrap()["did"], sponsored["did"]);
    }

    #[dialog_common::test]
    async fn it_keeps_agents_still_waiting_for_a_sponsor() {
        let agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        let waiting = agents.create().await.unwrap();
        assert_eq!(agents.reclaim(std::time::SystemTime::now()).unwrap(), 1);
        assert!(
            agents
                .identity(waiting["agent"].as_str().unwrap())
                .await
                .is_ok()
        );
    }

    #[dialog_common::test]
    async fn it_refuses_new_agents_while_too_many_wait() {
        let mut agents = HostedAgents::new(root(), "http://127.0.0.1:9/ucan/".parse().unwrap());
        agents.max_waiting = 1;
        agents.create().await.unwrap();
        let error = agents.create().await.unwrap_err();
        assert_eq!(
            error,
            "Too many agents are waiting for approval. Try again later."
        );
    }
}
