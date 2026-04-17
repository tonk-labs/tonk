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
use dialog_credentials::Ed25519Signer;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;
use url::Url;

/// Default base URL for invite links. Used when no override is
/// provided. Changing this value is a breaking change for any outstanding
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
#[derive(Debug)]
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

/// Decoded components of an invite URL.
#[derive(Debug)]
#[non_exhaustive]
pub struct DecodedInvite {
    /// Delegation chain granting access to `subject`.
    pub chain: DelegationChain,
    /// Repo subject DID.
    pub subject: Did,
    /// Whether the invite is open to any redeemer or scoped to one DID.
    pub audience: InviteAudience,
    /// Access service URL for sync, if included via `&remote=`.
    pub remote_url: Option<Url>,
}

/// Outcome of [`claim`]: a delegation chain ready to be persisted by the
/// caller, plus metadata about the invite.
#[derive(Debug)]
#[non_exhaustive]
pub struct ClaimedInvite {
    /// Final delegation chain terminating at the redeemer's DID.
    pub chain: DelegationChain,
    /// Repo subject DID (always `Subject::Specific`).
    pub subject: Did,
    /// Access service URL for sync, if the invite included one.
    pub remote_url: Option<Url>,
}

/// Parse an invite URL into its components.
///
/// Validates that the URL is syntactically a URL, that `access` is present
/// and decodes to a [`DelegationChain`] with a specific subject, and — if a
/// fragment is present — that it decodes to exactly [`SEED_LEN`] bytes.
///
/// # Errors
///
/// Returns an error if the string is not a valid URL, if the `access` query
/// parameter is missing or not valid base58, if the decoded chain fails to
/// parse or has no specific subject ([`Subject::Any`] is rejected), if the
/// `remote` parameter is present but not a valid URL, or if the fragment is
/// present but does not decode to exactly 32 bytes of base58.
///
/// [`Subject::Any`]: dialog_ucan_core::subject::Subject::Any
pub fn parse_invite_url(url: &str) -> Result<DecodedInvite> {
    let parsed = Url::parse(url).context("invite URL is not a valid URL")?;

    let mut access: Option<String> = None;
    let mut remote_url: Option<Url> = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "access" => access = Some(value.into_owned()),
            "remote" => {
                let r =
                    Url::parse(&value).context("invite `remote` parameter is not a valid URL")?;
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
    let subject = chain.subject().cloned().context(
        "invite delegation chain must target a specific repo subject; \
         chains with Subject::Any are not valid invites",
    )?;

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

    Ok(DecodedInvite {
        chain,
        subject,
        audience,
        remote_url,
    })
}

/// Build an invite URL from a delegation chain and optional components.
///
/// `base_url` must parse as a URL and must not already carry a query string
/// or fragment — those slots are reserved for invite data.
///
/// # Errors
///
/// Returns an error if the base URL is malformed, if it already has a query
/// or fragment, or if the delegation chain fails to serialize.
pub fn build_invite_url(
    base_url: &str,
    chain: &DelegationChain,
    remote_url: Option<&Url>,
    secret_seed: Option<&EphemeralSeed>,
) -> Result<String> {
    let chain_bytes = chain
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
        if let Some(remote) = remote_url {
            pairs.append_pair("remote", remote.as_str());
        }
    }

    if let Some(seed) = secret_seed {
        url.set_fragment(Some(&bs58::encode(seed.as_ref()).into_string()));
    }

    Ok(url.into())
}

/// Redelegate from an ephemeral key to a target DID.
///
/// Takes an existing delegation chain (whose audience is the ephemeral key's
/// DID and whose subject is a specific repo) and extends it with a new
/// delegation from the ephemeral key to `audience`, preserving the subject.
///
/// # Errors
///
/// Returns an error if the chain has no specific subject (tonk invites must
/// always scope to a specific repo — [`Subject::Any`] is never minted), if
/// the ephemeral key cannot be imported from the seed, if the redelegation
/// cannot be built, or if the new delegation cannot be appended to the chain
/// (e.g. because the chain's tail audience does not match the ephemeral key's
/// DID).
///
/// [`Subject::Any`]: dialog_ucan_core::subject::Subject::Any
pub async fn redelegate(
    chain: DelegationChain,
    ephemeral_seed: &EphemeralSeed,
    audience: &Did,
) -> Result<DelegationChain> {
    let subject = chain.subject().cloned().context(
        "refusing to redelegate a chain with no specific subject; \
         invites must always target a specific repo (Subject::Any is rejected)",
    )?;

    let ephemeral = Ed25519Signer::import(ephemeral_seed)
        .await
        .context("failed to import ephemeral key from seed")?;

    let delegation = DelegationBuilder::new()
        .issuer(ephemeral)
        .audience(audience)
        .subject(UcanSubject::Specific(subject))
        .command(vec![])
        .try_build()
        .await
        .context("failed to build redelegation")?;

    chain
        .push(delegation)
        .context("failed to extend delegation chain with redelegation")
}

/// Redeem an invite URL on behalf of `audience`, returning a delegation
/// chain ready to persist.
///
/// - **Audience-open** (URL fragment present): redelegates from the ephemeral
///   key embedded in the fragment to `audience`, producing an extended chain.
/// - **Audience-scoped** (no fragment): verifies the chain's existing
///   audience matches `audience` and returns it as-is.
///
/// Persistence of the returned chain, and configuration of any `remote_url`
/// for sync, are the caller's responsibility — see `carry`'s `claim` command
/// for a native reference implementation and `tonk-ui`'s invite flow for a
/// WASM one.
///
/// # Errors
///
/// Returns an error from [`parse_invite_url`] if the URL is malformed, from
/// [`redelegate`] if an open invite fails to extend, or a bespoke error if a
/// scoped invite's recorded audience does not match `audience`.
pub async fn claim(url: &str, audience: &Did) -> Result<ClaimedInvite> {
    let decoded = parse_invite_url(url)?;

    let chain = match decoded.audience {
        InviteAudience::Open { seed } => redelegate(decoded.chain, &seed, audience).await?,
        InviteAudience::Scoped => {
            let chain_audience = decoded.chain.audience();
            anyhow::ensure!(
                *chain_audience == *audience,
                "this scoped invite was issued to {}, but the redeemer is {}; \
                 ask the inviter to issue a new invite for your DID, \
                 or switch to the identity the invite was issued to",
                chain_audience,
                audience
            );
            decoded.chain
        }
    };

    Ok(ClaimedInvite {
        chain,
        subject: decoded.subject,
        remote_url: decoded.remote_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::ed25519::Ed25519Signer;
    use dialog_varsig::principal::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test;

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

    #[test]
    fn parse_rejects_non_url_input() {
        let err = parse_invite_url("not a url").unwrap_err();
        assert!(err.to_string().contains("not a valid URL"), "{err}");
    }

    #[test]
    fn parse_rejects_missing_access_parameter() {
        let err = parse_invite_url("https://tonk.xyz/join").unwrap_err();
        assert!(err.to_string().contains("`access`"), "{err}");
    }

    #[test]
    fn parse_rejects_invalid_base58_in_access() {
        let err = parse_invite_url("https://tonk.xyz/join?access=!!!not-b58!!!").unwrap_err();
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
        let mut url = build_invite_url(DEFAULT_BASE_URL, &chain, None, None).unwrap();
        url.push('#');
        url.push_str(&bs58::encode([9u8, 9, 9]).into_string());

        let err = parse_invite_url(&url).unwrap_err();
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
        let url = build_invite_url(DEFAULT_BASE_URL, &chain, Some(&remote), None).unwrap();
        let decoded = parse_invite_url(&url).unwrap();
        assert_eq!(decoded.subject, subject);
        assert!(matches!(decoded.audience, InviteAudience::Scoped));
        assert_eq!(decoded.remote_url.as_ref(), Some(&remote));

        // Open, no remote
        let url = build_invite_url(DEFAULT_BASE_URL, &chain, None, Some(&EPHEMERAL_SEED)).unwrap();
        let decoded = parse_invite_url(&url).unwrap();
        assert_eq!(decoded.subject, subject);
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
        let url = build_invite_url(DEFAULT_BASE_URL, &chain, None, None).unwrap();

        let err = claim(&url, &wrong_redeemer).await.unwrap_err();
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
        let url = build_invite_url(DEFAULT_BASE_URL, &chain, None, None).unwrap();

        let claimed = claim(&url, &audience).await.unwrap();
        assert_eq!(claimed.subject, subject);
        assert_eq!(*claimed.chain.audience(), audience);
    }

    #[cfg_attr(
        not(all(target_arch = "wasm32", target_os = "unknown")),
        tokio::test(flavor = "current_thread")
    )]
    #[cfg_attr(all(target_arch = "wasm32", target_os = "unknown"), wasm_bindgen_test)]
    async fn claim_open_invite_extends_chain_to_redeemer() {
        // Setup: a chain `issuer -> ephemeral_key`, scoped to a specific repo.
        // The invite URL carries the ephemeral seed in its fragment.
        let subject = signer(&SUBJECT_SEED).await.did();
        let ephemeral_did = signer(&EPHEMERAL_SEED).await.did();
        let redeemer = signer(&AUDIENCE_SEED).await.did();
        let chain = make_chain(&ISSUER_SEED, &ephemeral_did, &subject).await;
        let pre_len = chain.proof_cids().len();
        let url = build_invite_url(DEFAULT_BASE_URL, &chain, None, Some(&EPHEMERAL_SEED)).unwrap();

        let claimed = claim(&url, &redeemer).await.unwrap();

        assert_eq!(claimed.subject, subject, "subject must not change");
        assert_eq!(
            claimed.chain.subject(),
            Some(&subject),
            "chain subject must remain scoped to the original repo"
        );
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
}
