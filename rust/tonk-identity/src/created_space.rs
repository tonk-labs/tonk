//! Space-signed handoff created in a hosted MCP confirmation. The only
//! surviving key material is Dialog's normal space seed sealed to the owner.

use anyhow::{Context as _, Result, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Signer, Ed25519Verifier, Signer, secret::Context};
use dialog_ucan_core::{DelegationBuilder, DelegationChain, subject::Subject, time::Timestamp};
use dialog_varsig::{Did, Principal as _};
use ipld_core::ipld::Ipld;
use tonk_invite::connection::{AgentAccess, DEFAULT_GRANT_TTL_SECONDS, agent_grant_scopes};
use url::Url;
use zeroize::Zeroizing;

/// Signed metadata on the full space-to-account delegation.
pub const SEALED: &str = "tonk.space.sealed";
/// The proposed display name.
pub const NAME: &str = "tonk.space.name";
/// The agent receiving the independent read/write grants.
pub const AGENT: &str = "tonk.space.agent";
/// The public agent certificates, retained by the owner for revocation.
pub const GRANTS: &str = "tonk.space.grants";
/// Dialog's context for a held space principal.
pub const SPACE_KEY: &str = "dialog.space/key";
/// Permission granted during OAuth setup, independent of account storage.
pub const CREATE: [&str; 2] = ["provider", "create-space"];
/// Account devices read their own handoffs with this permission.
pub const LIST: [&str; 2] = ["provider", "created-spaces"];

/// Mint the one create permission for this connection.
pub async fn permission(
    root: &Ed25519Signer,
    agent: &Did,
    home: &Url,
    expires: Timestamp,
) -> Result<DelegationChain> {
    Ok(DelegationChain::new(
        DelegationBuilder::new()
            .issuer(Signer::from(root.clone()))
            .audience(agent)
            .subject(Subject::Specific(root.did()))
            .command(CREATE.map(str::to_string).to_vec())
            .meta(tonk_invite::home_address_meta(home))
            .expiration(expires)
            .try_build()
            .await?,
    ))
}

/// Only the owner certificate and the agent's public delegations leave this function.
pub async fn mint(owner: &Did, agent: &Did, name: &str, home: &Url) -> Result<DelegationChain> {
    ensure!(
        !name.trim().is_empty() && name.chars().count() <= 100,
        "invalid space name"
    );
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut())
        .map_err(|e| anyhow::anyhow!("space entropy unavailable: {e}"))?;
    let space = Ed25519Signer::import(&*seed)
        .await
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let verifier: Ed25519Verifier = owner.as_str().parse()?;
    let sealed = verifier
        .secret(Context::new(SPACE_KEY))
        .conceal(seed.as_ref())
        .await?;
    drop(seed);
    let subject = space.did();
    let signer = Signer::from(space);
    let expires = Timestamp::try_from(i128::from(
        Timestamp::now().to_unix() + DEFAULT_GRANT_TTL_SECONDS,
    ))?;
    let mut grants = Vec::new();
    for scope in agent_grant_scopes(&subject, AgentAccess::ReadWrite) {
        let grant = DelegationBuilder::new()
            .issuer(signer.clone())
            .audience(agent)
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .meta(tonk_invite::home_address_meta(home))
            .expiration(expires)
            .try_build()
            .await?;
        grants.push(Ipld::Bytes(DelegationChain::new(grant).to_bytes()?));
    }
    let mut meta = tonk_invite::home_address_meta(home);
    meta.insert(SEALED.into(), Ipld::Bytes(sealed.to_bytes()));
    meta.insert(NAME.into(), Ipld::String(name.into()));
    meta.insert(AGENT.into(), Ipld::String(agent.to_string()));
    meta.insert(GRANTS.into(), Ipld::List(grants));
    Ok(DelegationChain::new(
        DelegationBuilder::new()
            .issuer(signer)
            .audience(owner)
            .subject(Subject::Specific(subject))
            .command(vec![])
            .meta(meta)
            .try_build()
            .await?,
    ))
}

/// Checked owner receipt. The grant is independently usable by the account.
pub struct Handoff {
    /// Full, nonexpiring space authority addressed to the owner.
    pub owner_grant: DelegationChain,
    /// New space DID.
    pub subject: Did,
    /// Agent that received the limited grants.
    pub agent: Did,
    /// Original display name.
    pub name: String,
    /// Space seed encrypted for the account root.
    pub sealed: Vec<u8>,
    /// Scoped agent certificates and their revocation targets.
    pub grants: Vec<DelegationChain>,
}

impl Handoff {
    /// Verify the owner, full authority, scope and signature before importing
    /// a service receipt. An expired agent grant does not expire ownership.
    pub async fn read(chain: DelegationChain, owner: &Did) -> Result<Self> {
        let cert = chain.proofs().next().context("missing owner grant")?;
        ensure!(
            chain.proof_cids().len() == 1
                && cert.audience() == owner
                && cert.subject() == &Subject::Specific(cert.issuer().clone())
                && cert.command().0.is_empty()
                && cert.policy().is_empty()
                && cert.expiration().is_none(),
            "invalid owner grant"
        );
        cert.verify_signature(&DidKeyResolver).await?;
        let subject = cert.issuer().clone();
        let meta = cert.meta();
        let Some(Ipld::Bytes(sealed)) = meta.get(SEALED) else {
            anyhow::bail!("missing sealed key")
        };
        dialog_credentials::secret::SealedSecret::from_bytes(sealed)?;
        let Some(Ipld::String(name)) = meta.get(NAME) else {
            anyhow::bail!("missing name")
        };
        let Some(Ipld::String(agent)) = meta.get(AGENT) else {
            anyhow::bail!("missing agent")
        };
        ensure!(
            !name.trim().is_empty() && name.chars().count() <= 100,
            "invalid space name"
        );
        let home = tonk_invite::home_address(&chain)?.context("missing space home")?;
        let agent: Did = agent.parse()?;
        ensure!(
            &agent != owner && agent != subject && owner != &subject,
            "owner, agent and space must be independent"
        );
        let Some(Ipld::List(encoded)) = meta.get(GRANTS) else {
            anyhow::bail!("missing grants")
        };
        let scopes = agent_grant_scopes(&subject, AgentAccess::ReadWrite);
        ensure!(
            encoded.len() == scopes.len(),
            "wrong number of agent grants"
        );
        let mut grants = Vec::new();
        for (value, scope) in encoded.iter().zip(scopes) {
            let Ipld::Bytes(bytes) = value else {
                anyhow::bail!("invalid grant bytes")
            };
            let grant = DelegationChain::try_from(bytes.as_slice())?;
            let leaf = grant.proofs().next().context("missing agent leaf")?;
            ensure!(
                grant.proof_cids().len() == 1
                    && tonk_invite::home_address(&grant)?.as_ref() == Some(&home)
                    && leaf.issuer() == &subject
                    && leaf.audience() == &agent
                    && leaf.subject() == &scope.subject
                    && leaf.command() == &scope.command
                    && leaf.policy() == &scope.policy()
                    && leaf.expiration().is_some(),
                "invalid direct agent grant"
            );
            leaf.verify_signature(&DidKeyResolver).await?;
            grants.push(grant);
        }
        Ok(Self {
            subject,
            agent,
            name: name.clone(),
            sealed: sealed.clone(),
            grants,
            owner_grant: chain,
        })
    }
}
