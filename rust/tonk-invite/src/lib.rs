#![warn(missing_docs)]

//! Tonk invite URL format and claim orchestration.
//!
//! This crate builds on `dialog-{credentials, ucan-core, varsig}` only and
//! compiles for both native and WASM targets. Both the `carry` CLI and the
//! `tonk-ui` web client depend on it so that invite URLs issued by one are
//! redeemable by the other.
//!
//! ## URL format
//!
//! ```text
//! <base>?access=<base58-ucan-chain>[&remote=<access-service-url>][#<base58-seed>]
//! ```
//!
//! - `access`: base58-encoded [`DelegationChain`] bytes. The chain's subject
//!   must be [`Subject::Specific`]; chains with [`Subject::Any`] are rejected
//!   because an invite must always scope to a specific repository.
//! - `remote` (optional): UCAN access service endpoint for sync. A chain
//!   whose delegations carry a [`HOME_ADDRESS`] entry names its own endpoint;
//!   the signed value wins over this parameter, which stays as the carrier
//!   for chains minted before the meta rode the delegation.
//! - `#fragment` (optional): base58 of a 32-byte Ed25519 seed. Presence marks
//!   the invite as **audience-open** — any redeemer can claim it by
//!   redelegating from the embedded ephemeral key. Absence marks it as
//!   **audience-scoped** — only the chain's recorded audience DID can claim.
//!
//! The open/scoped distinction is the *audience* axis. The *subject* axis is
//! always scoped to a specific repo — orthogonal and non-negotiable.
//!
//! [`Subject::Specific`]: dialog_ucan_core::subject::Subject::Specific
//! [`Subject::Any`]: dialog_ucan_core::subject::Subject::Any

pub mod shortcut;

use anyhow::{Context, Result};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan::{Scope, UcanProof};
use dialog_ucan_core::{
    DelegationBuilder, DelegationChain,
    subject::Subject as UcanSubject,
    time::Timestamp,
    time::timestamp::{Duration, SystemTime},
};
use dialog_varsig::{Did, Principal};
use ipld_core::ipld::Ipld;
use std::collections::BTreeMap;
use url::Url;

/// Canonical base URL for tonk invite links, and the fallback for a repo
/// with no remote to take an origin from. Callers serializing an [`Invite`]
/// pass this to [`Invite::to_url`] to mint a link rooted at tonk.network.
///
/// Changing it does not invalidate outstanding invites: the base is not a
/// lookup key, so a link already minted against another host keeps
/// redeeming for as long as that host stays up.
pub const DEFAULT_BASE_URL: &str = "https://tonk.network/join";

/// Lifetime of the authority installed by [`Invite::visit`].
///
/// A visit is intentionally useful long enough to inspect a shared space but
/// cannot become a permanent identity or membership grant by accident.
pub const VISIT_TTL_SECONDS: u64 = 60 * 60;

/// UCAN delegation `meta` key naming the sync endpoint the granted subject
/// is served from: a space invite carries the space's upstream, an account
/// grant carries the account's `xyz.tonk.account/provider-address`.
///
/// A delegation that carries it makes the endpoint part of the signed
/// payload, so the grant and the address travel together and cannot be
/// swapped independently. Read it with [`home_address`].
pub const HOME_ADDRESS: &str = "home.address";

/// The sync endpoint a delegation chain names for itself, when it does.
///
/// Scans the chain leaf-to-root and returns the first [`HOME_ADDRESS`]
/// entry: the delegation minted closest to the recipient is the one minted
/// at handoff time, so its address is the most specific.
///
/// # Errors
///
/// Returns an error when a delegation carries the key but its value is not
/// a string holding a valid URL — a grant that names its endpoint illegibly
/// is malformed, not endpoint-less.
pub fn home_address(chain: &DelegationChain) -> Result<Option<Url>> {
    let Some(value) = chain.meta(HOME_ADDRESS) else {
        return Ok(None);
    };
    let Ipld::String(address) = value else {
        anyhow::bail!("delegation `{HOME_ADDRESS}` meta is not a string");
    };
    let url = Url::parse(address)
        .with_context(|| format!("delegation `{HOME_ADDRESS}` meta is not a valid URL"))?;
    Ok(Some(url))
}

/// A [`HOME_ADDRESS`] entry ready to hand to a delegation builder's `meta`.
pub fn home_address_meta(address: &Url) -> BTreeMap<String, Ipld> {
    BTreeMap::from([(HOME_ADDRESS.to_owned(), Ipld::String(address.to_string()))])
}

/// Length in bytes of the Ed25519 seed embedded in the URL fragment for
/// audience-open invites.
const SEED_LEN: usize = 32;

/// Ed25519 seed for the ephemeral key in an audience-open invite.
pub type EphemeralSeed = [u8; SEED_LEN];

/// Audience dimension of an invite.
///
/// This is orthogonal to the subject dimension (which is always a specific
/// repo). See the [crate-level documentation][crate] for the full model.
#[derive(Debug, Clone)]
pub enum InviteAudience {
    /// Any redeemer can claim, by redelegating from the ephemeral key whose
    /// seed is embedded in the URL fragment.
    Open {
        /// Ed25519 seed for the ephemeral key; matches the chain's audience DID.
        seed: EphemeralSeed,
    },
    /// Only the chain's recorded audience DID can claim; no redelegation.
    Scoped,
}

/// A shareable tonk invite: a delegation chain plus its audience mode and
/// an optional sync remote. Serializable to several wire formats — today a
/// URL via [`Invite::to_url`]; QR codes and invite files slot in as
/// additional methods without touching the core type.
///
/// Construct via [`Invite::new`] (programmatic) or [`Invite::parse_url`]
/// (from a URL). Both reject chains whose subject is not specific, so
/// [`Invite::subject`] always resolves.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Invite {
    /// Delegation chain granting access to the repo named by [`Invite::subject`].
    pub chain: DelegationChain,
    /// Whether the invite is open to any redeemer or scoped to one DID.
    pub audience: InviteAudience,
    /// Access service URL for sync, if the inviter attached one.
    pub remote_url: Option<Url>,
    /// Provider-independent relay that accepts raw signed revocation artifacts.
    pub revocation_url: Option<Url>,
}

impl Invite {
    /// Assemble an invite from its parts.
    ///
    /// For [`InviteAudience::Open`], the embedded seed must derive the
    /// chain's tail audience DID — otherwise the resulting URL would
    /// advertise a redelegation path that the `claim` step can't follow
    /// (principal alignment would fail at `DelegationChain::push` time
    /// with a cryptic error). Validating here turns that into an
    /// up-front mismatch error at construction and parse time.
    ///
    /// # Errors
    ///
    /// Returns an error if the chain's subject is not
    /// [`Subject::Specific`][dialog_ucan_core::subject::Subject::Specific],
    /// or if `audience` is [`InviteAudience::Open`] and the seed does
    /// not derive the chain's tail audience DID.
    pub async fn new(
        chain: DelegationChain,
        audience: InviteAudience,
        remote_url: Option<Url>,
    ) -> Result<Self> {
        anyhow::ensure!(
            chain.subject().is_some(),
            "invite delegation chain must target a specific repo subject; \
             chains with Subject::Any are not valid invites"
        );
        if let InviteAudience::Open { seed } = &audience {
            let signer = Ed25519Signer::import(seed)
                .await
                .context("failed to import ephemeral signer from seed")?;
            let derived = signer.did();
            let chain_audience = chain.audience();
            anyhow::ensure!(
                derived == *chain_audience,
                "open-invite seed derives {} but chain's tail audience is {}; \
                 an invite constructed with a mismatched seed would fail at claim time",
                derived,
                chain_audience,
            );
        }
        Ok(Self {
            chain,
            audience,
            remote_url,
            revocation_url: None,
        })
    }

    /// Attach an explicit revocation relay URL as executor configuration.
    #[must_use]
    pub fn with_revocation_url(mut self, revocation_url: Option<Url>) -> Self {
        self.revocation_url = revocation_url;
        self
    }

    /// Repo subject DID. Guaranteed specific by construction.
    pub fn subject(&self) -> &Did {
        self.chain
            .subject()
            .expect("Invite invariant: chain.subject() is Some (validated at construction)")
    }

    /// Parse an invite URL into an [`Invite`].
    ///
    /// Validates that the URL is syntactically a URL, that `access` is
    /// present and decodes to a [`DelegationChain`] with a specific
    /// subject, and — if a fragment is present — that it decodes to
    /// exactly [`SEED_LEN`] bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the string is not a valid URL, if the `access`
    /// query parameter is missing or not valid base58, if the decoded
    /// chain fails to parse or has no specific subject
    /// ([`Subject::Any`][dialog_ucan_core::subject::Subject::Any] is
    /// rejected), if the `remote` parameter is present but not a valid
    /// URL, if a delegation carries a [`HOME_ADDRESS`] entry that is not
    /// a string holding a valid URL, if the fragment is present but does
    /// not decode to exactly
    /// 32 bytes of base58, or if the fragment seed does not derive the
    /// chain's tail audience DID (enforced by [`Invite::new`]).
    pub async fn parse_url(url: &str) -> Result<Self> {
        let parsed = Url::parse(url).context("invite URL is not a valid URL")?;

        let mut access: Option<String> = None;
        let mut remote_url: Option<Url> = None;
        let mut revocation_url: Option<Url> = None;
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "access" => access = Some(value.into_owned()),
                "remote" => {
                    let r = Url::parse(&value)
                        .context("invite `remote` parameter is not a valid URL")?;
                    remote_url = Some(r);
                }
                "revocation" => {
                    let relay = Url::parse(&value)
                        .context("invite `revocation` parameter is not a valid URL")?;
                    revocation_url = Some(relay);
                }
                _ => {}
            }
        }
        let access = access.context("invite URL is missing the `access` query parameter")?;

        let chain_bytes = bs58::decode(&access)
            .into_vec()
            .context("invite `access` parameter is not valid base58")?;
        let chain = DelegationChain::try_from(chain_bytes.as_slice()).with_context(|| {
            format!(
                "invite `access` parameter did not decode to a valid delegation chain ({} bytes)",
                chain_bytes.len()
            )
        })?;

        // The chain's own signed address wins over the loose parameter,
        // which stays as the carrier for chains minted before the meta
        // rode the delegation.
        let remote_url = home_address(&chain)?.or(remote_url);

        let audience = match parsed.fragment() {
            Some(frag) => {
                let bytes = bs58::decode(frag).into_vec().context(
                    "invite URL fragment is not valid base58 \
                     (expected a 32-byte Ed25519 seed)",
                )?;
                let seed: EphemeralSeed = bytes.as_slice().try_into().map_err(|_| {
                    anyhow::anyhow!(
                        "invite URL fragment must decode to exactly {} bytes, got {}",
                        SEED_LEN,
                        bytes.len()
                    )
                })?;
                InviteAudience::Open { seed }
            }
            None => InviteAudience::Scoped,
        };

        Ok(Self::new(chain, audience, remote_url)
            .await?
            .with_revocation_url(revocation_url))
    }

    /// Serialize the invite as a URL rooted at `base_url`.
    ///
    /// `base_url` must parse as a URL and must not already carry a query
    /// string or fragment — those slots are reserved for invite data.
    ///
    /// # Errors
    ///
    /// Returns an error if the base URL is malformed, if it already has a
    /// query or fragment, or if the delegation chain fails to serialize.
    pub fn to_url(&self, base_url: &str) -> Result<String> {
        let chain_bytes = self
            .chain
            .to_bytes()
            .context("failed to serialize delegation chain")?;
        let access = bs58::encode(&chain_bytes).into_string();

        let mut url = Url::parse(base_url).context("invite base URL is not a valid URL")?;
        anyhow::ensure!(
            url.query().is_none(),
            "invite base URL must not already contain a query string"
        );
        anyhow::ensure!(
            url.fragment().is_none(),
            "invite base URL must not already contain a fragment"
        );

        // A chain that names its own endpoint carries it signed; writing
        // the loose parameter beside it would hand a reader two answers.
        // The parameter is written only for chains from before the meta
        // rode the delegation.
        let loose_remote = match home_address(&self.chain)? {
            Some(_) => None,
            None => self.remote_url.as_ref(),
        };
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("access", &access);
            if let Some(remote) = loose_remote {
                pairs.append_pair("remote", remote.as_str());
            }
            if let Some(relay) = &self.revocation_url {
                pairs.append_pair("revocation", relay.as_str());
            }
        }

        if let InviteAudience::Open { seed } = &self.audience {
            url.set_fragment(Some(&bs58::encode(seed.as_ref()).into_string()));
        }

        Ok(url.into())
    }

    /// The ephemeral signer empowered by an audience-open invite.
    ///
    /// For [`InviteAudience::Open`] returns `Some(signer)` imported from
    /// the seed carried by the invite; the signer's DID matches the
    /// chain's tail audience, so claiming an open invite is just
    /// redelegation from this signer to the redeemer. For
    /// [`InviteAudience::Scoped`] returns `None` — the redeemer must
    /// already hold the audience key themselves.
    ///
    /// # Errors
    ///
    /// Returns an error if the embedded seed fails to import as an
    /// Ed25519 signer (e.g. cryptographically invalid bytes).
    pub async fn signer(&self) -> Result<Option<Ed25519Signer>> {
        match &self.audience {
            InviteAudience::Open { seed } => Ed25519Signer::import(seed)
                .await
                .map(Some)
                .context("failed to import ephemeral signer from invite seed"),
            InviteAudience::Scoped => Ok(None),
        }
    }

    /// Redeem this invite on behalf of `audience`, returning a delegation
    /// chain ready to persist.
    ///
    /// - **Audience-open** (fragment was present on the source URL):
    ///   redelegates from the embedded ephemeral key to `audience`,
    ///   extending the chain by one hop.
    /// - **Audience-scoped** (no fragment): verifies the chain's existing
    ///   audience matches `audience` and returns it as-is.
    ///
    /// Persistence of the returned chain, and configuration of any
    /// `remote_url` for sync, are the caller's responsibility.
    ///
    /// Implementation note: redelegation goes through
    /// [`DelegationBuilder`] + [`DelegationChain::push`] directly,
    /// rather than through [`UcanProof::claim`] /
    /// [`UcanAuthorization::delegate`]. The latter rebuilds the chain
    /// by iterating `DelegationChain`'s internal `HashMap` values,
    /// which is unordered — for multi-hop chains that path can fail
    /// intermittently with a principal-alignment error depending on
    /// the process's `RandomState`. Going direct to the chain's own
    /// `push` keeps the documented root-to-leaf ordering.
    ///
    /// TODO: revert this workaround once the fix for
    /// `UcanProof::from_chain` lands in dialog-db and the workspace
    /// pin bumps past it
    ///
    /// # Errors
    ///
    /// Returns an error if an open invite fails to extend, or if a scoped
    /// invite's recorded audience does not match `audience`.
    ///
    /// [`DelegationBuilder`]: dialog_ucan_core::DelegationBuilder
    /// [`DelegationChain::push`]: dialog_ucan_core::DelegationChain::push
    /// [`UcanProof::claim`]: dialog_ucan::UcanProof
    /// [`UcanAuthorization::delegate`]: dialog_ucan::UcanAuthorization
    pub async fn claim(self, audience: &Did) -> Result<ClaimedInvite> {
        self.redelegate(audience, None).await
    }

    /// Visit an audience-open invite with bounded, session authority.
    ///
    /// Unlike [`Invite::claim`], the redelegation expires after
    /// [`VISIT_TTL_SECONDS`]. Callers should target an ephemeral session DID,
    /// not a durable root DID. Visiting never creates membership by itself;
    /// an explicit later claim to the user's root is the durable join.
    ///
    /// # Errors
    ///
    /// Returns an error for audience-scoped invites (which cannot safely be
    /// retargeted to a guest session), invalid clocks, or failed delegation.
    pub async fn visit(self, session: &Did) -> Result<ClaimedInvite> {
        anyhow::ensure!(
            matches!(&self.audience, InviteAudience::Open { .. }),
            "audience-scoped invites cannot be opened as a guest"
        );
        let expiration = Timestamp::new(SystemTime::now() + Duration::from_secs(VISIT_TTL_SECONDS))
            .map_err(|error| anyhow::anyhow!("visit expiration out of range: {error}"))?;
        self.redelegate(session, Some(expiration)).await
    }

    async fn redelegate(
        self,
        audience: &Did,
        expiration: Option<Timestamp>,
    ) -> Result<ClaimedInvite> {
        let signer = self.signer().await?;
        let remote_url = self.remote_url.clone();
        let revocation_url = self.revocation_url.clone();

        let chain = match signer {
            Some(ephemeral) => {
                let subject = self
                    .chain
                    .subject()
                    .cloned()
                    .map(UcanSubject::Specific)
                    .unwrap_or(UcanSubject::Any);
                // The chain carries algorithm-agnostic signatures, so the
                // redelegation is issued through the polymorphic signer.
                let mut builder = DelegationBuilder::new()
                    .issuer(Signer::from(ephemeral))
                    .audience(audience)
                    .subject(subject)
                    .command(vec![]);
                if let Some(expiration) = expiration {
                    builder = builder.expiration(expiration);
                }
                let delegation = builder
                    .try_build()
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to build redelegation: {e}"))?;
                self.chain
                    .push(delegation)
                    .map_err(|e| anyhow::anyhow!("failed to extend delegation chain: {e}"))?
            }
            None => {
                let chain_audience = self.chain.audience();
                anyhow::ensure!(
                    *chain_audience == *audience,
                    "this invite is for {} and cannot be redeemed by {}; \
                     ask the inviter to issue a new invite for your DID, \
                     or switch to the identity the invite was issued to",
                    chain_audience,
                    audience
                );
                self.chain
            }
        };

        Ok(ClaimedInvite {
            chain,
            remote_url,
            revocation_url,
        })
    }
}

impl From<Invite> for UcanProof {
    /// Project the invite's delegation chain into a [`UcanProof`], with
    /// scope derived from the chain's subject and command. Combined with
    /// [`Invite::signer`] this gives an invite-to-authorization pipeline
    /// that reuses the dialog-ucan claim/delegate machinery.
    fn from(invite: Invite) -> Self {
        let scope = Scope::from_chain(&invite.chain);
        UcanProof::from_chain(&invite.chain, scope)
    }
}

/// Outcome of [`Invite::claim`]: a delegation chain ready to be persisted
/// by the caller, paired with the invite's sync remote.
///
/// We return this wrapper rather than a bare [`DelegationChain`] so that
/// the `Subject::Specific` invariant validated at [`Invite`] construction
/// carries through to [`ClaimedInvite::subject`] without re-checking, and
/// so `remote_url` threads through to callers that want to persist sync
/// config alongside the chain.
///
/// `#[non_exhaustive]` reserves room to grow additional carry-over fields
/// (e.g. capability metadata) without a breaking change.
#[derive(Debug)]
#[non_exhaustive]
pub struct ClaimedInvite {
    /// Final delegation chain terminating at the redeemer's DID.
    pub chain: DelegationChain,
    /// Access service URL for sync, if the invite included one.
    pub remote_url: Option<Url>,
    /// Revocation submission relay, if the invite included one.
    pub revocation_url: Option<Url>,
}

impl ClaimedInvite {
    /// Repo subject DID. Carried over from the source [`Invite`], whose
    /// construction guarantees [`Subject::Specific`][dialog_ucan_core::subject::Subject::Specific].
    pub fn subject(&self) -> &Did {
        self.chain
            .subject()
            .expect("ClaimedInvite invariant: chain.subject() is Some (inherited from Invite)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_ucan_core::DelegationBuilder;
    use dialog_ucan_core::subject::Subject as UcanSubject;
    use dialog_varsig::principal::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const ISSUER_SEED: [u8; 32] = [1u8; 32];
    const SUBJECT_SEED: [u8; 32] = [2u8; 32];
    const AUDIENCE_SEED: [u8; 32] = [3u8; 32];
    const EPHEMERAL_SEED: [u8; 32] = [4u8; 32];

    /// Build a single-hop delegation chain: `issuer -> audience`, scoped to
    /// `subject`. Test callers pick the three seeds.
    async fn make_chain(
        issuer_seed: &[u8; 32],
        audience_did: &Did,
        subject_did: &Did,
    ) -> DelegationChain {
        let issuer = Ed25519Signer::import(issuer_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    /// Extend `chain` by one hop, delegated from `issuer` to
    /// `audience`. The existing chain's leaf audience must equal the
    /// issuer's DID, otherwise `push` will reject.
    async fn extend_chain(
        chain: DelegationChain,
        issuer: Ed25519Signer,
        audience: &Did,
    ) -> DelegationChain {
        let subject = chain
            .subject()
            .cloned()
            .map(UcanSubject::Specific)
            .unwrap_or(UcanSubject::Any);
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience)
            .subject(subject)
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        chain.push(delegation).unwrap()
    }

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    /// Like [`make_chain`], with a [`HOME_ADDRESS`] entry on the delegation.
    async fn make_chain_with_remote(
        issuer_seed: &[u8; 32],
        audience_did: &Did,
        subject_did: &Did,
        remote: &Url,
    ) -> DelegationChain {
        let issuer = Ed25519Signer::import(issuer_seed).await.unwrap();
        let delegation = DelegationBuilder::new()
            .issuer(dialog_credentials::Signer::from(issuer))
            .audience(audience_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .meta(home_address_meta(remote))
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    #[dialog_common::test]
    async fn it_reads_the_remote_from_the_delegation_meta() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let remote = Url::parse("https://staging.tonk.xyz/ucan/").unwrap();
        let chain = make_chain_with_remote(&ISSUER_SEED, &audience, &subject, &remote).await;

        // No `remote=` parameter on the URL at all: the chain names it.
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        assert!(!url.contains("remote="), "{url}");

        let decoded = Invite::parse_url(&url).await.unwrap();
        assert_eq!(decoded.remote_url, Some(remote));
    }

    #[dialog_common::test]
    async fn it_prefers_the_signed_remote_over_the_url_parameter() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let signed = Url::parse("https://staging.tonk.xyz/ucan/").unwrap();
        let tampered = Url::parse("https://attacker.example/ucan/").unwrap();
        let chain = make_chain_with_remote(&ISSUER_SEED, &audience, &subject, &signed).await;

        let invite = Invite::new(chain, InviteAudience::Scoped, Some(tampered))
            .await
            .unwrap();
        let decoded = Invite::parse_url(&invite.to_url(DEFAULT_BASE_URL).unwrap())
            .await
            .unwrap();
        assert_eq!(decoded.remote_url, Some(signed));
    }

    #[dialog_common::test]
    async fn it_rejects_non_url_input() {
        let err = Invite::parse_url("not a url").await.unwrap_err();
        assert!(err.to_string().contains("not a valid URL"), "{err}");
    }

    #[dialog_common::test]
    async fn it_rejects_missing_access_parameter() {
        let err = Invite::parse_url("https://tonk.network/join")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("`access`"), "{err}");
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_base58_in_access() {
        let err = Invite::parse_url("https://tonk.network/join?access=!!!not-b58!!!")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("valid base58"), "{err}");
    }

    #[dialog_common::test]
    async fn it_rejects_wrong_length_fragment() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        // Valid, round-trippable URL — but we overwrite the fragment with a
        // 3-byte payload (base58 of [9,9,9]) to probe the length check.
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();
        let mut url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        url.push('#');
        url.push_str(&bs58::encode([9u8, 9, 9]).into_string());

        let err = Invite::parse_url(&url).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exactly 32 bytes") || msg.contains("got 3"),
            "{msg}"
        );
    }

    #[dialog_common::test]
    async fn it_round_trips_through_url() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience_signer = signer(&AUDIENCE_SEED).await;
        let audience = audience_signer.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let remote = Url::parse("https://tonk.network/ucan/").unwrap();

        // Scoped, with remote
        let invite = Invite::new(chain.clone(), InviteAudience::Scoped, Some(remote.clone()))
            .await
            .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        let decoded = Invite::parse_url(&url).await.unwrap();
        assert_eq!(*decoded.subject(), subject);
        assert!(matches!(decoded.audience, InviteAudience::Scoped));
        assert_eq!(decoded.remote_url.as_ref(), Some(&remote));

        // Open, no remote. The chain's tail audience must be the
        // ephemeral key's DID for Invite::new to accept it.
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let open_chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let invite = Invite::new(
            open_chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .await
        .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        let decoded = Invite::parse_url(&url).await.unwrap();
        assert_eq!(*decoded.subject(), subject);
        match decoded.audience {
            InviteAudience::Open { seed } => assert_eq!(seed, EPHEMERAL_SEED),
            InviteAudience::Scoped => panic!("expected open audience"),
        }
        assert!(decoded.remote_url.is_none());
    }

    #[dialog_common::test]
    async fn it_round_trips_the_revocation_submission_url() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let relay = Url::parse("https://accounts.example/revocations").unwrap();
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap()
            .with_revocation_url(Some(relay.clone()));

        let decoded = Invite::parse_url(&invite.to_url(DEFAULT_BASE_URL).unwrap())
            .await
            .unwrap();
        assert_eq!(decoded.revocation_url, Some(relay));
    }

    #[dialog_common::test]
    async fn it_keeps_existing_invites_without_relay_metadata_parseable() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();

        let decoded = Invite::parse_url(&invite.to_url(DEFAULT_BASE_URL).unwrap())
            .await
            .unwrap();
        assert!(decoded.revocation_url.is_none());
    }

    #[dialog_common::test]
    async fn it_rejects_scoped_claim_by_wrong_audience() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let issued_to = signer(&AUDIENCE_SEED).await.did();
        let wrong_redeemer = signer(&EPHEMERAL_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &issued_to, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();

        let err = invite.claim(&wrong_redeemer).await.unwrap_err();
        assert!(err.to_string().contains("cannot be redeemed"), "{err}");
    }

    #[dialog_common::test]
    async fn it_accepts_scoped_claim_by_matching_audience() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();

        let claimed = invite.claim(&audience).await.unwrap();
        assert_eq!(*claimed.subject(), subject);
        assert_eq!(*claimed.chain.audience(), audience);
    }

    /// Invite::new must reject an Open variant whose seed does not
    /// derive the chain's tail audience DID. This is the construction
    /// guard that matches the existing parse_url guard.
    #[dialog_common::test]
    async fn it_rejects_open_invite_with_mismatched_seed() {
        const OTHER_SEED: [u8; 32] = [7u8; 32];
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        // Chain is delegated to the ephemeral DID, but we'll claim the
        // invite is open with a seed that derives a different DID.
        let chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let err = Invite::new(chain, InviteAudience::Open { seed: OTHER_SEED }, None)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("derives") && msg.contains("tail audience"),
            "{msg}"
        );
    }

    /// Multi-hop open-invite claim: an issuer delegates to a mid key,
    /// which delegates to an ephemeral key embedded in the invite;
    /// the redeemer claims by redelegating one more hop. The
    /// resulting chain should have three proofs and terminate at the
    /// redeemer.
    #[dialog_common::test]
    async fn it_extends_multi_hop_chain_to_redeemer_when_claiming_open_invite() {
        const MID_SEED: [u8; 32] = [5u8; 32];
        const ROOT_SEED: [u8; 32] = [6u8; 32];

        let subject = signer(&SUBJECT_SEED).await.did();
        let mid_did = signer(&MID_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let redeemer = signer(&AUDIENCE_SEED).await.did();

        let first_hop = make_chain(&ROOT_SEED, &mid_did, &subject).await;
        let mid_signer = signer(&MID_SEED).await;
        let two_hop = extend_chain(first_hop, mid_signer, &ephemeral_did).await;
        assert_eq!(two_hop.proof_cids().len(), 2);

        let invite = Invite::new(
            two_hop,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .await
        .unwrap();

        let claimed = invite.claim(&redeemer).await.unwrap();
        assert_eq!(claimed.chain.proof_cids().len(), 3);
        assert_eq!(*claimed.chain.audience(), redeemer);
        assert_eq!(*claimed.subject(), subject);
    }

    #[dialog_common::test]
    async fn it_visits_with_bounded_session_authority_without_changing_the_invite() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let session = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .await
        .unwrap();

        let visited = invite.visit(&session).await.unwrap();

        assert_eq!(*visited.chain.audience(), session);
        assert_eq!(*visited.subject(), subject);
        let expiration = visited.chain.expiration().expect("visit must expire");
        assert!(expiration.to_unix() > Timestamp::now().to_unix());
        assert!(expiration.to_unix() <= Timestamp::now().to_unix() + VISIT_TTL_SECONDS);
    }

    #[dialog_common::test]
    async fn it_refuses_to_turn_a_scoped_invite_into_a_guest_visit() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None)
            .await
            .unwrap();

        let error = invite.visit(&audience).await.unwrap_err();
        assert!(error.to_string().contains("cannot be opened as a guest"));
    }

    #[dialog_common::test]
    async fn it_extends_chain_to_redeemer_when_claiming_open_invite() {
        // Setup: a chain `issuer -> ephemeral_key`, scoped to a specific repo.
        // The invite carries the ephemeral seed; anyone can claim it.
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let redeemer = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let pre_len = chain.proof_cids().len();
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .await
        .unwrap();

        let claimed = invite.claim(&redeemer).await.unwrap();

        assert_eq!(*claimed.subject(), subject, "subject must not change");
        assert_eq!(
            *claimed.chain.audience(),
            redeemer,
            "chain audience must terminate at the redeemer"
        );
        assert_eq!(
            claimed.chain.proof_cids().len(),
            pre_len + 1,
            "chain should grow by exactly one redelegation"
        );
        assert!(
            claimed.remote_url.is_none(),
            "remote_url must be absent — none was provided"
        );
    }

    #[dialog_common::test]
    async fn it_claims_invite_parsed_from_url() {
        // End-to-end: an open invite minted by one client, serialized to a
        // URL, and claimed by another via parse_url + claim.
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let redeemer = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .await
        .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();

        let claimed = Invite::parse_url(&url)
            .await
            .unwrap()
            .claim(&redeemer)
            .await
            .unwrap();
        assert_eq!(*claimed.subject(), subject);
        assert_eq!(*claimed.chain.audience(), redeemer);
    }
}
