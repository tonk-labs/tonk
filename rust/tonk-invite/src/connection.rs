//! Versioned bearer invitations for ordinary space delegations.
//!
//! Import validates offline authority. Executors must still check ordinary UCAN
//! revocations on every remote request. Neither parsing nor confirmation activates
//! a grant. The caller supplies independently trusted routing and exact scopes.

use anyhow::{Context, Result, ensure};
use dialog_credentials::{DidKeyResolver, Ed25519Signer};
use dialog_ucan::{Parameters, Scope};
use dialog_ucan_core::{
    DelegationChain, command::Command, delegation::chain::check_chain, subject::Subject,
    time::Timestamp,
};
use dialog_varsig::{Did, Principal};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use ipld_core::ipld::Ipld;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use url::Url;

/// Default requested grant duration for both invitation and CLI-owned identities.
/// Issuers must report an ancestor limit instead of silently shortening this.
pub const DEFAULT_GRANT_TTL_SECONDS: u64 = 90 * 24 * 60 * 60;
/// Maximum accepted encoded invitation size, before allocating decoded storage.
pub const MAX_ENVELOPE_LENGTH: usize = 1024 * 1024;
const LEGACY_PREFIX: &str = "tonk-agent-v1=";
const PREFIX: &str = "tonk-agent-v2=";
const GRANTS_PARAMETER: &str = "agent";

// HTTPS is required except for explicit loopback development endpoints. Syntax
// checks prevent unsafe carrier URLs; independent service trust remains required.
fn validate_endpoint(url: &Url) -> Result<()> {
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "connection_invalid_url"
    );
    ensure!(
        url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "connection_invalid_url"
    );
    Ok(())
}

/// Candidate transport scopes; real CLI build coverage is a separate gate.
/// In particular this excludes the management-bearing `meta` branch.
pub fn candidate_build_scopes(subject: &Did) -> Vec<Scope> {
    let mut scopes = Vec::new();
    for verb in ["get", "put"] {
        for (kind, item, parameters) in [
            (
                "memory",
                "cell",
                vec![("space", "branch/main"), ("cell", "revision")],
            ),
            ("archive", "block", vec![("catalog", "index")]),
            ("archive", "blob", vec![]),
        ] {
            scopes.push(Scope {
                subject: Subject::Specific(subject.clone()),
                command: Command(vec!["use".into(), verb.into(), kind.into(), item.into()]),
                parameters: Parameters(
                    parameters
                        .into_iter()
                        .map(|(key, value)| (key.into(), Ipld::String(value.into())))
                        .collect(),
                ),
            });
        }
    }
    scopes
}

/// Stable grant-group identity shared by browser management and CLI receipts.
pub fn grant_set_id(subject: &str, recipient: &str, grant_cids: &[String]) -> String {
    let mut grant_cids = grant_cids.to_vec();
    grant_cids.sort();
    let bytes =
        serde_json::to_vec(&(subject, recipient, grant_cids)).expect("public strings serialize");
    blake3::hash(&bytes).to_hex().to_string()
}

/// Reject a requested deadline that exceeds any ancestor's effective deadline.
/// The error includes the actual limiting Unix timestamp for truthful UI.
pub fn require_grant_deadline(chains: &[DelegationChain], requested: Timestamp) -> Result<()> {
    if let Some(limit) = chains.iter().filter_map(DelegationChain::expiration).min() {
        ensure!(
            requested <= limit,
            "connection_expiry_limited: upstream authority expires at {}",
            limit.to_unix()
        );
    }
    Ok(())
}

/// Public grants checked for one subject, recipient, exact scope set and route.
/// This is not a cached assertion that the grants remain unrevoked.
#[derive(Debug, Clone)]
pub struct SpaceGrantBundle {
    chains: Vec<DelegationChain>,
    subject: Did,
    recipient: Did,
    expires_at: Timestamp,
    remote: Url,
}

impl SpaceGrantBundle {
    /// Validate signatures, rooted chains, exact leaf rights and bounded expiry.
    ///
    /// `trusted_remote` must come from the existing independently verified service
    /// configuration, not an unsigned URL parameter. Every leaf must sign it.
    /// Ancestor policies must be a subset of the requested equality predicates;
    /// other policies are conservatively refused rather than guessed equivalent.
    pub async fn validate(
        chains: Vec<DelegationChain>,
        recipient: &Did,
        scopes: &[Scope],
        trusted_remote: &Url,
        now: Timestamp,
    ) -> Result<Self> {
        validate_endpoint(trusted_remote)?;
        ensure!(
            !scopes.is_empty() && chains.len() == scopes.len(),
            "connection_scope_mismatch"
        );
        let Subject::Specific(subject) = &scopes[0].subject else {
            anyhow::bail!("connection_subject_mismatch");
        };
        let mut matched = vec![false; scopes.len()];
        let mut expires_at = None;
        for chain in &chains {
            ensure!(
                chain.subject() == Some(subject),
                "connection_subject_mismatch"
            );
            ensure!(
                chain.audience() == recipient,
                "connection_recipient_mismatch"
            );
            check_chain(chain.proofs(), subject, Some(now)).context("connection_invalid_chain")?;
            let leaf = chain.proofs().last().context("connection_invalid_chain")?;
            let index = scopes
                .iter()
                .enumerate()
                .position(|(i, scope)| {
                    !matched[i]
                        && scope.subject == Subject::Specific(subject.clone())
                        && leaf.command() == &scope.command
                        && leaf.policy() == &scope.policy()
                })
                .context("connection_scope_mismatch")?;
            matched[index] = true;
            let policy = scopes[index].policy();
            for hop in chain.proofs() {
                ensure!(hop.issuer() != recipient, "connection_recipient_not_fresh");
                hop.verify_signature(&DidKeyResolver)
                    .await
                    .context("connection_invalid_signature")?;
                ensure!(
                    leaf.command().starts_with(hop.command())
                        && hop
                            .policy()
                            .iter()
                            .all(|predicate| policy.contains(predicate)),
                    "connection_unsupported_ancestor_scope"
                );
            }
            let deadline = leaf.expiration().context("connection_missing_expiry")?;
            ensure!(
                chain.expiration() == Some(deadline),
                "connection_expiry_limited"
            );
            expires_at = Some(expires_at.map_or(deadline, |prior: Timestamp| prior.min(deadline)));
            ensure!(
                leaf.meta().get(crate::HOME_ADDRESS)
                    == Some(&Ipld::String(trusted_remote.to_string())),
                "connection_untrusted_route"
            );
        }
        Ok(Self {
            chains,
            subject: subject.clone(),
            recipient: recipient.clone(),
            expires_at: expires_at.expect("nonempty checked"),
            remote: trusted_remote.clone(),
        })
    }

    /// Complete public proof chains, with no private signing material.
    pub fn chains(&self) -> &[DelegationChain] {
        &self.chains
    }
    /// The only space subject authorized by this bundle.
    pub fn subject(&self) -> &Did {
        &self.subject
    }
    /// DID whose signing key exercises the supplied grants.
    pub fn recipient(&self) -> &Did {
        &self.recipient
    }
    /// Earliest explicit effective deadline in the bundle.
    pub fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
    /// Independently trusted endpoint matched to signed routing metadata.
    pub fn remote(&self) -> &Url {
        &self.remote
    }
}

/// Reusable bearer identity and its ordinary grant bundle.
/// Debug output intentionally redacts the seed and full bearer URL.
#[derive(Clone)]
pub struct AgentInvite {
    seed: [u8; 32],
    grants: SpaceGrantBundle,
}

impl std::fmt::Debug for AgentInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentInvite")
            .field("recipient", self.grants.recipient())
            .field("subject", self.grants.subject())
            .field("seed", &"[REDACTED]")
            .finish()
    }
}

impl AgentInvite {
    /// Validate an issuer-created seed and grant set without changing its identity.
    pub async fn new(
        seed: [u8; 32],
        chains: Vec<DelegationChain>,
        scopes: &[Scope],
        trusted_remote: &Url,
        now: Timestamp,
    ) -> Result<Self> {
        let signer = Ed25519Signer::import(&seed)
            .await
            .context("connection_invalid_key")?;
        let grants =
            SpaceGrantBundle::validate(chains, &signer.did(), scopes, trusted_remote, now).await?;
        Ok(Self { seed, grants })
    }

    /// Public grant bundle, suitable for journals and management indexes.
    pub fn grants(&self) -> &SpaceGrantBundle {
        &self.grants
    }
    /// Explicit secret export for the credential store; never journal or log this.
    pub fn secret_seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// Encode public grants in the query and the bearer seed alone in the fragment.
    ///
    /// Keeping the grants out of the fragment lets the ordinary shortcut service
    /// store them while the private seed remains client-side across its redirect.
    pub fn to_url(&self, base: &str) -> Result<String> {
        let mut url = Url::parse(base).context("connection_invalid_url")?;
        validate_endpoint(&url)?;
        let chains = self
            .grants
            .chains
            .iter()
            .map(|chain| chain.to_bytes().map(Ipld::Bytes))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let data = Ipld::Map(BTreeMap::from([
            ("version".into(), Ipld::Integer(2)),
            ("grants".into(), Ipld::List(chains)),
        ]));
        let bytes = serde_ipld_dagcbor::to_vec(&data)?;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&bytes)
            .context("connection_invalid_envelope")?;
        let grants =
            bs58::encode(encoder.finish().context("connection_invalid_envelope")?).into_string();
        ensure!(
            grants.len() <= MAX_ENVELOPE_LENGTH,
            "connection_envelope_too_large"
        );
        url.query_pairs_mut().append_pair(GRANTS_PARAMETER, &grants);
        let fragment = format!("{PREFIX}{}", bs58::encode(self.seed).into_string());
        url.set_fragment(Some(&fragment));
        Ok(url.into())
    }

    /// Decode and validate a versioned bearer against caller-selected scope/route.
    /// This never redelegates or performs an approval/redemption request.
    pub async fn parse_url(
        value: &str,
        scopes: &[Scope],
        trusted_remote: &Url,
        now: Timestamp,
    ) -> Result<Self> {
        let (seed, chains) = Self::decode_url(value)?;
        Self::new(seed, chains, scopes, trusted_remote, now).await
    }
    /// Inspect a bearer only after verifying its signatures and fixed data rights.
    /// The signed claimed endpoint is still untrusted routing information: resolve
    /// it through the application's trusted service discovery before importing.
    pub async fn inspect_url(value: &str, now: Timestamp) -> Result<InvitationHint> {
        let (seed, chains) = Self::decode_url(value)?;
        let subject = chains[0]
            .subject()
            .cloned()
            .context("connection_subject_mismatch")?;
        let leaf = chains[0]
            .proofs()
            .last()
            .context("connection_invalid_chain")?;
        let Some(Ipld::String(remote)) = leaf.meta().get(crate::HOME_ADDRESS) else {
            anyhow::bail!("connection_untrusted_route");
        };
        let remote = Url::parse(remote).context("connection_invalid_url")?;
        let invite = Self::new(
            seed,
            chains,
            &candidate_build_scopes(&subject),
            &remote,
            now,
        )
        .await?;
        Ok(InvitationHint {
            subject,
            recipient: invite.grants.recipient.clone(),
            remote,
        })
    }

    fn decode_url(value: &str) -> Result<([u8; 32], Vec<DelegationChain>)> {
        ensure!(
            value.len() <= MAX_ENVELOPE_LENGTH + 4096,
            "connection_envelope_too_large"
        );
        let url = Url::parse(value).context("connection_invalid_url")?;
        let mut base = url.clone();
        base.set_query(None);
        base.set_fragment(None);
        validate_endpoint(&base)?;
        let fragment = url.fragment().context("connection_missing_key")?;
        ensure!(
            fragment.len() <= MAX_ENVELOPE_LENGTH,
            "connection_envelope_too_large"
        );
        if let Some(encoded) = fragment.strip_prefix(LEGACY_PREFIX) {
            ensure!(url.query().is_none(), "connection_invalid_url");
            return Self::decode_legacy(encoded);
        }
        let encoded_seed = fragment
            .strip_prefix(PREFIX)
            .context("connection_unsupported_version")?;
        let seed = bs58::decode(encoded_seed)
            .into_vec()
            .context("connection_invalid_key")?;
        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|_| anyhow::anyhow!("connection_invalid_key"))?;
        let mut grants = url
            .query_pairs()
            .filter(|(key, _)| key == GRANTS_PARAMETER)
            .map(|(_, value)| value.into_owned());
        let encoded_grants = grants.next().context("connection_invalid_envelope")?;
        ensure!(
            grants.next().is_none() && encoded_grants.len() <= MAX_ENVELOPE_LENGTH,
            "connection_invalid_envelope"
        );
        let bytes = bs58::decode(encoded_grants)
            .into_vec()
            .context("connection_invalid_envelope")?;
        let mut envelope = Self::decode_v2_envelope(&bytes)?;
        ensure!(
            envelope.remove("version") == Some(Ipld::Integer(2)),
            "connection_unsupported_version"
        );
        let chains = Self::decode_grants(&mut envelope)?;
        Ok((seed, chains))
    }

    fn decode_v2_envelope(bytes: &[u8]) -> Result<BTreeMap<String, Ipld>> {
        // Accept the uncompressed v2 form emitted during development so a link
        // copied before compression was added remains usable.
        if let Ok(Ipld::Map(envelope)) = serde_ipld_dagcbor::from_slice(bytes) {
            return Ok(envelope);
        }
        let mut decoded = Vec::new();
        ZlibDecoder::new(bytes)
            .take((MAX_ENVELOPE_LENGTH + 1) as u64)
            .read_to_end(&mut decoded)
            .context("connection_invalid_envelope")?;
        ensure!(
            decoded.len() <= MAX_ENVELOPE_LENGTH,
            "connection_envelope_too_large"
        );
        let Ipld::Map(envelope) =
            serde_ipld_dagcbor::from_slice(&decoded).context("connection_invalid_envelope")?
        else {
            anyhow::bail!("connection_invalid_envelope")
        };
        Ok(envelope)
    }

    fn decode_legacy(encoded: &str) -> Result<([u8; 32], Vec<DelegationChain>)> {
        let bytes = bs58::decode(encoded)
            .into_vec()
            .context("connection_invalid_envelope")?;
        let Ipld::Map(mut envelope) =
            serde_ipld_dagcbor::from_slice(&bytes).context("connection_invalid_envelope")?
        else {
            anyhow::bail!("connection_invalid_envelope")
        };
        ensure!(
            envelope.remove("version") == Some(Ipld::Integer(1)),
            "connection_unsupported_version"
        );
        let Some(Ipld::Bytes(seed)) = envelope.remove("seed") else {
            anyhow::bail!("connection_missing_key")
        };
        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|_| anyhow::anyhow!("connection_invalid_key"))?;
        let chains = Self::decode_grants(&mut envelope)?;
        Ok((seed, chains))
    }

    fn decode_grants(envelope: &mut BTreeMap<String, Ipld>) -> Result<Vec<DelegationChain>> {
        let Some(Ipld::List(grants)) = envelope.remove("grants") else {
            anyhow::bail!("connection_invalid_envelope")
        };
        ensure!(
            envelope.is_empty() && !grants.is_empty() && grants.len() <= 64,
            "connection_invalid_envelope"
        );
        grants
            .into_iter()
            .map(|grant| {
                let Ipld::Bytes(bytes) = grant else {
                    anyhow::bail!("connection_invalid_envelope")
                };
                DelegationChain::try_from(bytes.as_slice()).context("connection_invalid_chain")
            })
            .collect::<Result<Vec<_>>>()
    }
}

/// Cryptographically checked public invitation facts, before route discovery.
#[derive(Debug, Clone)]
pub struct InvitationHint {
    /// Claimed single space subject, verified against all proof chains.
    pub subject: Did,
    /// Recipient public key reconstructed from the supplied seed.
    pub recipient: Did,
    /// Signed claimed endpoint; must still pass trusted service discovery.
    pub remote: Url,
}
