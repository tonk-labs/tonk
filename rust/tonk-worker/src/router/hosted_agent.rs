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

/// The non-authority facts about a sponsorship, kept beside the agent. The
/// chain itself is retained in the peer's delegation store.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SponsorRecord {
    sponsor: String,
    recipient: String,
    expires_at: u64,
}

const SPONSOR_FILE: &str = "sponsor.json";

/// The agents one hosted service keeps, under one private root.
pub struct HostedAgents {
    root: PathBuf,
}

impl HostedAgents {
    /// Agents under `root`. The caller serializes invocations.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
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
            _ => Err("Tool unavailable.".into()),
        }
    }

    /// Make a new agent with a fresh key.
    pub async fn create(&self) -> Result<Value, String> {
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
        };
        write_record(&directory, &record)?;
        Ok(json!({
            "agent": id,
            "did": agent.to_string(),
            "sponsor": record.sponsor,
            "expiresAt": record.expires_at,
        }))
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

/// Hold the agent's directory for this invocation, as [`LocalPeer`] requires.
fn lock(directory: &Path) -> Result<std::fs::File, String> {
    super::local_peer::try_lock(directory)
        .map_err(|_| UNAVAILABLE.to_owned())?
        .ok_or_else(|| "The agent is busy.".to_owned())
}

const INVALID: &str = "Invalid agent invocation.";
const UNAVAILABLE: &str = "Agent storage is unavailable.";

fn read_record(directory: &Path) -> Result<Option<SponsorRecord>, String> {
    match std::fs::read(directory.join(SPONSOR_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| UNAVAILABLE.into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(UNAVAILABLE.into()),
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
        let agents = HostedAgents::new(root());
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
        let agents = HostedAgents::new(root());
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
        let agents = HostedAgents::new(root());
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
        let agents = HostedAgents::new(root());
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
}
