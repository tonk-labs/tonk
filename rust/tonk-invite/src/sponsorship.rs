//! A human account sponsoring an agent.
//!
//! The sponsor's signed-in device extends its `account -> device` grant to the
//! agent's key with exactly `/provider/add` on the account. With it the agent
//! can provision spaces billed to the sponsor, and nothing else: it holds no
//! other authority over the account, and the sponsor revokes it like any grant.
//!
//! The chain is public. It carries no key material.

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Signer};
use dialog_ucan_core::{
    DelegationBuilder, DelegationChain,
    command::Command,
    delegation::chain::check_chain,
    subject::Subject,
    time::{
        Timestamp,
        timestamp::{Duration, UNIX_EPOCH},
    },
};
use dialog_varsig::{Did, Principal};

use crate::connection::DEFAULT_GRANT_TTL_SECONDS;

/// The one command a sponsorship grants: provisioning a space under the
/// sponsor's account with the access service.
pub const SPONSOR_COMMAND: &[&str] = &["provider", "add"];
/// Upper bound for an encoded sponsorship.
pub const MAX_SPONSORSHIP_BYTES: usize = 64 * 1024;

fn sponsor_command() -> Command {
    Command(
        SPONSOR_COMMAND
            .iter()
            .map(|part| (*part).to_owned())
            .collect(),
    )
}

/// A signed `sponsor -> ... -> agent` grant of `/provider/add` on the sponsor.
#[derive(Clone, Debug)]
pub struct AgentSponsorship {
    chain: DelegationChain,
}

/// What a valid sponsorship establishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSponsorship {
    /// The sponsoring account: the chain's root issuer and subject.
    pub sponsor: Did,
    /// The sponsored agent: the chain's final audience.
    pub agent: Did,
    /// When the grant lapses.
    pub expires_at: Timestamp,
}

impl AgentSponsorship {
    /// Extend the signed-in device's `account -> device` grant to `agent`.
    /// The leaf expires after the default grant lifetime, or with the device
    /// grant if that ends first.
    pub async fn issue_from_device(
        account_link: DelegationChain,
        device: &Signer,
        agent: &Did,
        now: Timestamp,
    ) -> Result<Self> {
        ensure!(
            account_link.audience() == &device.did(),
            "sponsorship_account_device_mismatch"
        );
        ensure!(agent != &device.did(), "sponsorship_agent_is_device");
        let sponsor = account_link.issuer().clone();
        ensure!(agent != &sponsor, "sponsorship_agent_is_sponsor");
        let seconds = now
            .to_unix()
            .checked_add(DEFAULT_GRANT_TTL_SECONDS)
            .context("sponsorship_time_overflow")?;
        let requested = Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds))
            .context("sponsorship_time_invalid")?;
        let expiration = account_link
            .expiration()
            .map_or(requested, |limit| requested.min(limit));
        let leaf = DelegationBuilder::new()
            .issuer(device.clone())
            .audience(agent)
            .subject(Subject::Specific(sponsor))
            .command(sponsor_command().0)
            .expiration(expiration)
            .try_build()
            .await?;
        Ok(Self {
            chain: account_link
                .push(leaf)
                .context("sponsorship_invalid_account_chain")?,
        })
    }

    /// Decode a bounded sponsorship.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_SPONSORSHIP_BYTES,
            "sponsorship_too_large"
        );
        Ok(Self {
            chain: DelegationChain::try_from(bytes).context("sponsorship_invalid_chain")?,
        })
    }

    /// Encode the public chain.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = self.chain.to_bytes()?;
        ensure!(
            bytes.len() <= MAX_SPONSORSHIP_BYTES,
            "sponsorship_too_large"
        );
        Ok(bytes)
    }

    /// The chain, for presenting as proof with an invocation.
    pub fn chain(&self) -> &DelegationChain {
        &self.chain
    }

    /// Check that this grants `agent` exactly `/provider/add` on the root
    /// account, with valid signatures on every hop, at `now`.
    ///
    /// Revocation is not checked here: the access service checks it on every
    /// use.
    pub async fn verify(&self, agent: &Did, now: Timestamp) -> Result<VerifiedSponsorship> {
        let chain = &self.chain;
        ensure!(chain.proofs().count() > 0, "sponsorship_invalid_chain");
        ensure!(chain.audience() == agent, "sponsorship_agent_mismatch");
        let sponsor = chain.issuer().clone();
        let root_subject = chain.subject().unwrap_or(&sponsor);
        ensure!(root_subject == &sponsor, "sponsorship_subject_mismatch");
        check_chain(chain.proofs(), &sponsor, Some(now)).context("sponsorship_invalid_chain")?;
        let leaf = chain.proofs().last().context("sponsorship_invalid_chain")?;
        ensure!(
            leaf.subject() == &Subject::Specific(sponsor.clone()),
            "sponsorship_subject_mismatch"
        );
        ensure!(
            leaf.command() == &sponsor_command() && leaf.policy().is_empty(),
            "sponsorship_scope_mismatch"
        );
        for hop in chain.proofs() {
            // Structural validation does not enforce attenuation. The leaf
            // carries no policy, so no ancestor may restrict it.
            ensure!(
                leaf.command().starts_with(hop.command()) && hop.policy().is_empty(),
                "sponsorship_scope_mismatch"
            );
            hop.verify_signature(&DidKeyResolver)
                .await
                .context("sponsorship_invalid_signature")?;
        }
        let expires_at = leaf.expiration().context("sponsorship_missing_expiry")?;
        Ok(VerifiedSponsorship {
            sponsor,
            agent: agent.clone(),
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::Ed25519Signer;

    fn at(seconds: u64) -> Timestamp {
        Timestamp::new(UNIX_EPOCH + Duration::from_secs(seconds)).unwrap()
    }

    /// `account -> device`, open in subject and command, as sign-in mints it.
    async fn account_link(
        account: &Ed25519Signer,
        device: &Ed25519Signer,
        expires: Option<Timestamp>,
    ) -> DelegationChain {
        let mut builder = DelegationBuilder::new()
            .issuer(Signer::from(account.clone()))
            .audience(&device.did())
            .subject(Subject::Any)
            .command(Vec::<String>::new());
        if let Some(expires) = expires {
            builder = builder.expiration(expires);
        }
        DelegationChain::new(builder.try_build().await.unwrap())
    }

    struct Fixture {
        account: Ed25519Signer,
        device: Ed25519Signer,
        agent: Did,
    }

    async fn fixture() -> Fixture {
        Fixture {
            account: Ed25519Signer::generate().await.unwrap(),
            device: Ed25519Signer::generate().await.unwrap(),
            agent: Ed25519Signer::generate().await.unwrap().did(),
        }
    }

    #[dialog_common::test]
    async fn it_sponsors_an_agent_from_the_signed_in_device() {
        let f = fixture().await;
        let link = account_link(&f.account, &f.device, None).await;
        let issued = AgentSponsorship::issue_from_device(
            link,
            &Signer::from(f.device.clone()),
            &f.agent,
            at(1_000_000),
        )
        .await
        .unwrap();
        let decoded = AgentSponsorship::from_bytes(&issued.to_bytes().unwrap()).unwrap();
        let verified = decoded.verify(&f.agent, at(1_000_001)).await.unwrap();
        assert_eq!(verified.sponsor, f.account.did());
        assert_eq!(verified.agent, f.agent);
        assert_eq!(
            verified.expires_at,
            at(1_000_000 + DEFAULT_GRANT_TTL_SECONDS)
        );
    }

    #[dialog_common::test]
    async fn it_ends_with_the_device_grant_when_that_ends_first() {
        let f = fixture().await;
        let link = account_link(&f.account, &f.device, Some(at(1_000_100))).await;
        let issued = AgentSponsorship::issue_from_device(
            link,
            &Signer::from(f.device.clone()),
            &f.agent,
            at(1_000_000),
        )
        .await
        .unwrap();
        let verified = issued.verify(&f.agent, at(1_000_001)).await.unwrap();
        assert_eq!(verified.expires_at, at(1_000_100));
        assert!(issued.verify(&f.agent, at(1_000_200)).await.is_err());
    }

    #[dialog_common::test]
    async fn it_refuses_a_sponsorship_for_another_agent() {
        let f = fixture().await;
        let link = account_link(&f.account, &f.device, None).await;
        let issued = AgentSponsorship::issue_from_device(
            link,
            &Signer::from(f.device.clone()),
            &f.agent,
            at(1_000_000),
        )
        .await
        .unwrap();
        let other = Ed25519Signer::generate().await.unwrap().did();
        let error = issued.verify(&other, at(1_000_001)).await.unwrap_err();
        assert_eq!(error.to_string(), "sponsorship_agent_mismatch");
    }

    #[dialog_common::test]
    async fn it_refuses_a_device_that_does_not_hold_the_account_grant() {
        let f = fixture().await;
        let link = account_link(&f.account, &f.device, None).await;
        let stranger = Ed25519Signer::generate().await.unwrap();
        let error = AgentSponsorship::issue_from_device(
            link,
            &Signer::from(stranger),
            &f.agent,
            at(1_000_000),
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "sponsorship_account_device_mismatch");
    }

    /// A grant of anything broader than `/provider/add`, or on another
    /// subject, is not a sponsorship.
    #[dialog_common::test]
    async fn it_refuses_a_broader_or_misdirected_grant() {
        let f = fixture().await;
        let device = Signer::from(f.device.clone());
        for (subject, command) in [
            (Subject::Specific(f.account.did()), vec![]),
            (
                Subject::Specific(f.account.did()),
                vec!["provider".to_owned()],
            ),
            (Subject::Any, sponsor_command().0),
            (
                Subject::Specific(Ed25519Signer::generate().await.unwrap().did()),
                sponsor_command().0,
            ),
        ] {
            let leaf = DelegationBuilder::new()
                .issuer(device.clone())
                .audience(&f.agent)
                .subject(subject)
                .command(command)
                .expiration(at(2_000_000))
                .try_build()
                .await
                .unwrap();
            let chain = account_link(&f.account, &f.device, None)
                .await
                .push(leaf)
                .unwrap();
            let bytes = chain.to_bytes().unwrap();
            let verdict = AgentSponsorship::from_bytes(&bytes)
                .unwrap()
                .verify(&f.agent, at(1_000_001))
                .await;
            assert!(verdict.is_err(), "accepted a non-sponsorship grant");
        }
    }
}
