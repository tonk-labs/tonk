//! An agent asking a person for access to their spaces.
//!
//! The agent holds its own key and signs a short-lived request naming
//! itself; the person opens the request where they are signed in, checks the
//! verification code the agent shows them against the one the page shows,
//! and approves the spaces to grant. Approval issues ordinary grants to the
//! agent's key (see [`crate::connection::AgentGrant`]); the request itself
//! authorizes nothing.
//!
//! Like the local-space-link consent, the request is a UCAN rather than a
//! second signature format: one self-issued delegation from the agent's key,
//! with the agent's name in its signed meta and a short expiry.

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Signer};
use dialog_ucan_core::{
    Delegation, DelegationBuilder,
    command::Command,
    subject::Subject,
    time::{
        Timestamp,
        timestamp::{Duration, UNIX_EPOCH},
    },
};
use dialog_varsig::{AnySignature, Did, Principal};
use ipld_core::ipld::Ipld;
use std::collections::BTreeMap;
use url::Url;

/// How long a request can be approved after the agent makes it.
pub const REQUEST_TTL_SECONDS: u64 = 60 * 60;
/// Longest display name a request may carry.
pub const MAX_NAME_LENGTH: usize = 80;

const PREFIX: &str = "tonk-agent-request=";
const REQUEST_COMMAND: &[&str] = &["agent", "access", "request"];
const META_NAME: &str = "tonk.agent.name";
const MAX_ENCODED_LENGTH: usize = 8 * 1024;

/// A signed request from an agent's key for access to a person's spaces.
#[derive(Clone, Debug)]
pub struct AgentRequest {
    delegation: Delegation<AnySignature>,
    name: String,
}

impl AgentRequest {
    /// Sign a request as `agent`, under the display `name`, valid from `now`.
    pub async fn new(agent: &Signer, name: &str, now: Timestamp) -> Result<Self> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= MAX_NAME_LENGTH,
            "agent_request_invalid_name"
        );
        let did = agent.did();
        let expires =
            Timestamp::new(UNIX_EPOCH + Duration::from_secs(now.to_unix() + REQUEST_TTL_SECONDS))
                .context("agent_request_invalid_time")?;
        let delegation = DelegationBuilder::new()
            .issuer(agent.clone())
            .audience(&did)
            .subject(Subject::Specific(did.clone()))
            .command(REQUEST_COMMAND.iter().map(|s| (*s).to_owned()).collect())
            .expiration(expires)
            .meta(BTreeMap::from([(
                META_NAME.to_owned(),
                Ipld::String(name.to_owned()),
            )]))
            .try_build()
            .await
            .context("agent_request_unsigned")?;
        Ok(Self {
            delegation,
            name: name.to_owned(),
        })
    }

    /// The key the agent asks to be granted access under.
    pub fn agent(&self) -> &Did {
        self.delegation.issuer()
    }

    /// The agent's display name, as it signed it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// When the request stops being approvable.
    pub fn expires_at(&self) -> Option<Timestamp> {
        self.delegation.expiration()
    }

    /// The code the agent shows its person, to compare with the page.
    pub fn verification_code(&self) -> String {
        verification_code(self.agent())
    }

    /// `base` with the signed request in its fragment, which stays in the
    /// browser: nothing in it is secret, but it need not reach a server.
    pub fn to_url(&self, base: &str) -> Result<String> {
        let mut url = Url::parse(base).context("agent_request_invalid_url")?;
        let encoded = bs58::encode(self.delegation.encoded()).into_string();
        ensure!(
            encoded.len() <= MAX_ENCODED_LENGTH,
            "agent_request_too_large"
        );
        url.set_fragment(Some(&format!("{PREFIX}{encoded}")));
        Ok(url.into())
    }

    /// Whether `value` is an access request link.
    pub fn is_url(value: &str) -> bool {
        Url::parse(value)
            .ok()
            .and_then(|url| url.fragment().map(|f| f.starts_with(PREFIX)))
            .unwrap_or(false)
    }

    /// Decode and verify a request link: signed by the key it names, for
    /// that key alone, asking only for access, and not yet expired at `now`.
    pub async fn from_url(value: &str, now: Timestamp) -> Result<Self> {
        let url = Url::parse(value).context("agent_request_invalid_url")?;
        let encoded = url
            .fragment()
            .and_then(|fragment| fragment.strip_prefix(PREFIX))
            .context("agent_request_missing")?;
        ensure!(
            encoded.len() <= MAX_ENCODED_LENGTH,
            "agent_request_too_large"
        );
        let bytes = bs58::decode(encoded)
            .into_vec()
            .context("agent_request_invalid_encoding")?;
        let delegation: Delegation<AnySignature> =
            serde_ipld_dagcbor::from_slice(&bytes).context("agent_request_invalid_encoding")?;
        Self::verify(delegation, now).await
    }

    async fn verify(delegation: Delegation<AnySignature>, now: Timestamp) -> Result<Self> {
        let agent = delegation.issuer().clone();
        ensure!(
            delegation.audience() == &agent
                && *delegation.subject() == Subject::Specific(agent.clone()),
            "agent_request_not_self_issued"
        );
        let expected = Command::new(REQUEST_COMMAND.iter().map(|s| (*s).to_owned()).collect());
        ensure!(
            delegation.command() == &expected && delegation.policy().is_empty(),
            "agent_request_wrong_command"
        );
        let expires = delegation
            .expiration()
            .context("agent_request_missing_expiry")?;
        ensure!(expires.to_unix() > now.to_unix(), "agent_request_expired");
        ensure!(
            expires.to_unix() <= now.to_unix() + REQUEST_TTL_SECONDS,
            "agent_request_too_long_lived"
        );
        let Some(Ipld::String(name)) = delegation.meta().get(META_NAME) else {
            anyhow::bail!("agent_request_invalid_name");
        };
        ensure!(
            !name.trim().is_empty() && name.chars().count() <= MAX_NAME_LENGTH,
            "agent_request_invalid_name"
        );
        let name = name.clone();
        delegation
            .verify_signature(&DidKeyResolver)
            .await
            .context("agent_request_invalid_signature")?;
        Ok(Self { delegation, name })
    }
}

/// A short code derived from an agent's key, for a person to compare what
/// the agent shows with what the approval page shows. 64 bits in four
/// groups: easy to read aloud, and not practical to forge by generating keys
/// until one matches.
pub fn verification_code(agent: &Did) -> String {
    let digest = blake3::derive_key(
        "tonk agent access verification code",
        agent.as_str().as_bytes(),
    );
    digest[..8]
        .chunks(2)
        .map(|pair| format!("{:02x}{:02x}", pair[0], pair[1]))
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;

    fn at(seconds: u64) -> Timestamp {
        Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).unwrap()
    }

    async fn agent(seed: u8) -> Signer {
        Signer::from(Ed25519Signer::import(&[seed; 32]).await.unwrap())
    }

    #[dialog_common::test]
    async fn it_round_trips_a_signed_request_through_its_link() {
        let agent = agent(70).await;
        let now = at(2_000_000_000);
        let request = AgentRequest::new(&agent, "  Claude Code on Jack's MacBook ", now)
            .await
            .unwrap();
        let url = request
            .to_url("https://tonk.network/agent/connect")
            .unwrap();
        assert!(AgentRequest::is_url(&url));
        assert!(!AgentRequest::is_url("https://tonk.network/agent/connect"));

        let opened = AgentRequest::from_url(&url, at(2_000_000_060))
            .await
            .unwrap();
        assert_eq!(opened.agent(), &agent.did());
        assert_eq!(opened.name(), "Claude Code on Jack's MacBook");
        assert_eq!(opened.verification_code(), request.verification_code());
    }

    #[dialog_common::test]
    async fn it_refuses_an_expired_or_tampered_request() {
        let agent = agent(71).await;
        let now = at(2_000_000_000);
        let url = AgentRequest::new(&agent, "agent", now)
            .await
            .unwrap()
            .to_url("https://tonk.network/agent/connect")
            .unwrap();
        let late = at(2_000_000_000 + REQUEST_TTL_SECONDS + 1);
        let error = AgentRequest::from_url(&url, late).await.unwrap_err();
        assert!(error.to_string().contains("expired"), "{error}");

        // Flip a byte of the signed request.
        let (base, encoded) = url.split_once(PREFIX).unwrap();
        let mut bytes = bs58::decode(encoded).into_vec().unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        let tampered = format!("{base}{PREFIX}{}", bs58::encode(bytes).into_string());
        assert!(AgentRequest::from_url(&tampered, now).await.is_err());
    }

    #[dialog_common::test]
    async fn it_refuses_a_request_for_someone_else_or_for_more() {
        let agent = agent(72).await;
        let other = agent_did(73).await;
        let now = at(2_000_000_000);
        let expires = at(2_000_000_000 + 60);
        let build = |audience: Did, subject: Did, command: Vec<String>| {
            let agent = agent.clone();
            async move {
                DelegationBuilder::new()
                    .issuer(agent)
                    .audience(&audience)
                    .subject(Subject::Specific(subject))
                    .command(command)
                    .expiration(expires)
                    .meta(BTreeMap::from([(
                        META_NAME.to_owned(),
                        Ipld::String("agent".into()),
                    )]))
                    .try_build()
                    .await
                    .unwrap()
            }
        };
        let own = agent.did();
        let request_command: Vec<String> =
            REQUEST_COMMAND.iter().map(|s| (*s).to_owned()).collect();
        for delegation in [
            build(other.clone(), own.clone(), request_command.clone()).await,
            build(own.clone(), other.clone(), request_command.clone()).await,
            build(own.clone(), own.clone(), vec![]).await,
        ] {
            assert!(AgentRequest::verify(delegation, now).await.is_err());
        }
    }

    async fn agent_did(seed: u8) -> Did {
        agent(seed).await.did()
    }

    #[dialog_common::test]
    async fn it_derives_a_stable_code_that_differs_per_key() {
        let one = agent_did(74).await;
        let two = agent_did(75).await;
        let code = verification_code(&one);
        assert_eq!(code, verification_code(&one));
        assert_ne!(code, verification_code(&two));
        assert_eq!(code.len(), 19);
        assert_eq!(code.matches('-').count(), 3);
    }
}
