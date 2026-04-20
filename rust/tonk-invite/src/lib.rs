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
//! - `remote` (optional): UCAN access service endpoint for sync.
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

use anyhow::{Context, Result};
use dialog_capability::access::{Authorization, Proof};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::{Scope, UcanProof};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use url::Url;

/// Canonical base URL for tonk invite links. Callers serializing an
/// [`Invite`] can pass this to [`Invite::to_url`] to mint a link rooted at
/// tonk.xyz. Changing this value is a breaking change for any outstanding
/// invite URLs that embed it.
pub const DEFAULT_BASE_URL: &str = "https://tonk.xyz/join";

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
}

impl Invite {
    /// Assemble an invite from its parts.
    ///
    /// # Errors
    ///
    /// Returns an error if the chain's subject is not
    /// [`Subject::Specific`][dialog_ucan_core::subject::Subject::Specific]
    /// — tonk invites must always target a specific repo.
    pub fn new(
        chain: DelegationChain,
        audience: InviteAudience,
        remote_url: Option<Url>,
    ) -> Result<Self> {
        anyhow::ensure!(
            chain.subject().is_some(),
            "invite delegation chain must target a specific repo subject; \
             chains with Subject::Any are not valid invites"
        );
        Ok(Self {
            chain,
            audience,
            remote_url,
        })
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
    /// URL, or if the fragment is present but does not decode to exactly
    /// 32 bytes of base58.
    pub fn parse_url(url: &str) -> Result<Self> {
        let parsed = Url::parse(url).context("invite URL is not a valid URL")?;

        let mut access: Option<String> = None;
        let mut remote_url: Option<Url> = None;
        for (key, value) in parsed.query_pairs() {
            match key.as_ref() {
                "access" => access = Some(value.into_owned()),
                "remote" => {
                    let r = Url::parse(&value)
                        .context("invite `remote` parameter is not a valid URL")?;
                    remote_url = Some(r);
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

        Self::new(chain, audience, remote_url)
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

        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("access", &access);
            if let Some(remote) = &self.remote_url {
                pairs.append_pair("remote", remote.as_str());
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
    ///   extending the chain by one hop. Flows through
    ///   [`UcanProof::claim`] and [`UcanAuthorization::delegate`] rather
    ///   than reimplementing the redelegation inline.
    /// - **Audience-scoped** (no fragment): verifies the chain's existing
    ///   audience matches `audience` and returns it as-is.
    ///
    /// Persistence of the returned chain, and configuration of any
    /// `remote_url` for sync, are the caller's responsibility.
    ///
    /// # Errors
    ///
    /// Returns an error if an open invite fails to extend, or if a scoped
    /// invite's recorded audience does not match `audience`.
    ///
    /// [`UcanProof::claim`]: dialog_ucan::UcanProof
    /// [`UcanAuthorization::delegate`]: dialog_ucan::UcanAuthorization
    pub async fn claim(self, audience: &Did) -> Result<ClaimedInvite> {
        let signer = self.signer().await?;
        let remote_url = self.remote_url.clone();

        let chain = match signer {
            Some(ephemeral) => {
                let proof = UcanProof::from(self);
                let auth = proof
                    .claim(ephemeral)
                    .map_err(|e| anyhow::anyhow!("failed to build authorization: {e}"))?;
                let delegation = auth
                    .delegate(audience.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to redelegate invite: {e}"))?;
                delegation.into_chain()
            }
            None => {
                let chain_audience = self.chain.audience();
                anyhow::ensure!(
                    *chain_audience == *audience,
                    "this scoped invite was issued to {}, but the redeemer is {}; \
                     ask the inviter to issue a new invite for your DID, \
                     or switch to the identity the invite was issued to",
                    chain_audience,
                    audience
                );
                self.chain
            }
        };

        Ok(ClaimedInvite { chain, remote_url })
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
/// by the caller, plus metadata carried over from the invite.
#[derive(Debug)]
#[non_exhaustive]
pub struct ClaimedInvite {
    /// Final delegation chain terminating at the redeemer's DID.
    pub chain: DelegationChain,
    /// Access service URL for sync, if the invite included one.
    pub remote_url: Option<Url>,
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
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

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
            .issuer(issuer)
            .audience(audience_did)
            .subject(UcanSubject::Specific(subject_did.clone()))
            .command(vec![])
            .try_build()
            .await
            .unwrap();
        DelegationChain::new(delegation)
    }

    async fn signer(seed: &[u8; 32]) -> Ed25519Signer {
        Ed25519Signer::import(seed).await.unwrap()
    }

    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    fn parse_rejects_non_url_input() {
        let err = Invite::parse_url("not a url").unwrap_err();
        assert!(err.to_string().contains("not a valid URL"), "{err}");
    }

    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    fn parse_rejects_missing_access_parameter() {
        let err = Invite::parse_url("https://tonk.xyz/join").unwrap_err();
        assert!(err.to_string().contains("`access`"), "{err}");
    }

    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    fn parse_rejects_invalid_base58_in_access() {
        let err = Invite::parse_url("https://tonk.xyz/join?access=!!!not-b58!!!").unwrap_err();
        assert!(err.to_string().contains("valid base58"), "{err}");
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn parse_rejects_wrong_length_fragment() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        // Valid, round-trippable URL — but we overwrite the fragment with a
        // 3-byte payload (base58 of [9,9,9]) to probe the length check.
        let invite = Invite::new(chain, InviteAudience::Scoped, None).unwrap();
        let mut url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        url.push('#');
        url.push_str(&bs58::encode([9u8, 9, 9]).into_string());

        let err = Invite::parse_url(&url).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exactly 32 bytes") || msg.contains("got 3"),
            "{msg}"
        );
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn build_and_parse_round_trip() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let remote = Url::parse("https://access.tonk.xyz/ucan").unwrap();

        // Scoped, with remote
        let invite =
            Invite::new(chain.clone(), InviteAudience::Scoped, Some(remote.clone())).unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        let decoded = Invite::parse_url(&url).unwrap();
        assert_eq!(*decoded.subject(), subject);
        assert!(matches!(decoded.audience, InviteAudience::Scoped));
        assert_eq!(decoded.remote_url.as_ref(), Some(&remote));

        // Open, no remote
        let invite = Invite::new(
            chain,
            InviteAudience::Open {
                seed: EPHEMERAL_SEED,
            },
            None,
        )
        .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();
        let decoded = Invite::parse_url(&url).unwrap();
        assert_eq!(*decoded.subject(), subject);
        match decoded.audience {
            InviteAudience::Open { seed } => assert_eq!(seed, EPHEMERAL_SEED),
            InviteAudience::Scoped => panic!("expected open audience"),
        }
        assert!(decoded.remote_url.is_none());
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn claim_scoped_rejects_wrong_audience() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let issued_to = signer(&AUDIENCE_SEED).await.did();
        let wrong_redeemer = signer(&EPHEMERAL_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &issued_to, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None).unwrap();

        let err = invite.claim(&wrong_redeemer).await.unwrap_err();
        assert!(err.to_string().contains("scoped invite"), "{err}");
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn claim_scoped_accepts_matching_audience() {
        let subject = signer(&SUBJECT_SEED).await.did();
        let audience = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &audience, &subject).await;
        let invite = Invite::new(chain, InviteAudience::Scoped, None).unwrap();

        let claimed = invite.claim(&audience).await.unwrap();
        assert_eq!(*claimed.subject(), subject);
        assert_eq!(*claimed.chain.audience(), audience);
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn claim_open_invite_extends_chain_to_redeemer() {
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

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn claim_from_url_round_trip() {
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
        .unwrap();
        let url = invite.to_url(DEFAULT_BASE_URL).unwrap();

        let claimed = Invite::parse_url(&url)
            .unwrap()
            .claim(&redeemer)
            .await
            .unwrap();
        assert_eq!(*claimed.subject(), subject);
        assert_eq!(*claimed.chain.audience(), redeemer);
    }
}
