//! `carry invite [<MEMBER>]` -- generate an invite URL granting access to a space.
//!
//! When `<MEMBER>` (a `did:key`) is provided, the delegation targets that
//! specific DID and the resulting URL contains no private key fragment.
//!
//! When `<MEMBER>` is omitted, carry generates a fresh ephemeral Ed25519
//! keypair, delegates to it, and embeds the private key in the URL fragment
//! (which is never sent to the server). Anyone who receives the URL can
//! redelegate from the ephemeral key to their own identity via `carry join`.
//!
//! ## URL format
//!
//! ```text
//! <base>?access=<base58-ucan-chain>#<base58-private-key>
//! ```
//!
//! The `access` query parameter contains the base58-encoded delegation chain.
//! The `#` fragment contains the ephemeral private key (only present for open
//! invites). The fragment is never sent to the server per RFC 3986.

use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::DelegationChain;
use dialog_varsig::{Did, Principal};

/// Default base URL for invite links.
const DEFAULT_BASE_URL: &str = "https://tonk.xyz/join";

/// The result of creating an invite.
pub struct Invite {
    /// The invite URL to share.
    pub url: String,
    /// The delegation chain (for asserting into the space).
    pub chain: DelegationChain,
    /// The member DID that was delegated to.
    pub audience: Did,
}

/// Execute `carry invite [<MEMBER>] [--url <BASE>]`.
pub async fn execute(site: &Site, member: Option<&str>, base_url: Option<&str>) -> Result<()> {
    let audience: Option<Did> = member
        .map(|m| m.parse().with_context(|| format!("invalid DID: {}", m)))
        .transpose()?;

    let invite = create_invite(site, audience.as_ref(), base_url).await?;

    println!("{}", invite.url);

    Ok(())
}

/// Build an invite URL delegating repo access.
///
/// If `audience` is `None`, generates an ephemeral keypair and embeds its
/// private key in the URL fragment.
pub async fn create_invite(
    site: &Site,
    audience: Option<&Did>,
    base_url: Option<&str>,
) -> Result<Invite> {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL);

    let (target_did, secret_fragment) = match audience {
        Some(did) => (did.clone(), None),
        None => {
            let ephemeral = Ed25519Signer::generate()
                .await
                .context("Failed to generate ephemeral keypair")?;
            let did = ephemeral.did();
            let seed = ephemeral
                .export()
                .await
                .context("Failed to export ephemeral key")?;
            let seed_bytes = match seed {
                dialog_credentials::KeyExport::Extractable(bytes) => bytes,
                #[allow(unreachable_patterns)]
                _ => anyhow::bail!("Ephemeral key is not extractable"),
            };
            (did, Some(bs58::encode(&seed_bytes).into_string()))
        }
    };

    let chain = site
        .profile
        .access()
        .claim(&site.repo)
        .delegate(target_did.clone())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation")?;

    let chain_bytes = chain
        .to_bytes()
        .context("Failed to serialize delegation chain")?;
    let access = bs58::encode(&chain_bytes).into_string();

    let mut url = format!("{}?access={}", base, access);

    // Include the access service URL so the joiner can configure sync.
    if let Some(remote_url) = resolve_access_url(site).await {
        url.push_str("&remote=");
        url.push_str(&remote_url);
    }

    if let Some(ref fragment) = secret_fragment {
        url.push('#');
        url.push_str(fragment);
    }

    Ok(Invite {
        url,
        chain,
        audience: target_did,
    })
}

/// Decoded invite URL contents.
pub struct DecodedInvite {
    /// The delegation chain granting access to the repo subject.
    pub chain: DelegationChain,
    /// The repo subject DID (extracted from the delegation chain).
    pub subject: Did,
    /// The ephemeral private key seed, if present in the URL fragment.
    pub secret_seed: Option<Vec<u8>>,
    /// The UCAN access service URL for sync, if included via `&remote=`.
    pub remote_url: Option<String>,
}

// Parse URL string and return decoded invite
pub fn parse_invite_url(url: &str) -> Result<DecodedInvite> {
    let (url, secret_seed) = match url.split_once('#') {
        Some((l, r)) => (l, Some(r)),
        None => (url, None),
    };

    let (_, query) = url
        .split_once("?access=")
        .context("missing ?access= query parameter")?;

    // Split access chain from optional &remote= parameter
    let (access_b58, remote_url) = match query.split_once("&remote=") {
        Some((chain, remote)) => (chain, Some(remote.to_string())),
        None => (query, None),
    };

    let chain_bytes = bs58::decode(access_b58)
        .into_vec()
        .context("invalid base58 in access parameter")?;
    let chain =
        DelegationChain::try_from(chain_bytes.as_slice()).context("invalid delegation chain")?;
    let subject = chain
        .subject()
        .cloned()
        .context("delegation chain has no subject")?;
    let secret_seed = secret_seed
        .map(|f| bs58::decode(f).into_vec().context("invalid secret"))
        .transpose()?;

    Ok(DecodedInvite {
        chain,
        subject,
        secret_seed,
        remote_url,
    })
}

/// Resolve the access service URL from the repo's upstream remote, if any.
pub async fn resolve_access_url(site: &Site) -> Option<String> {
    use dialog_repository::{SiteAddress, UpstreamState};

    let upstream = site.branch.upstream()?;
    let remote_name = match upstream {
        UpstreamState::Remote { name, .. } => name,
        _ => return None,
    };

    let remote = site
        .repo
        .remote(remote_name)
        .load()
        .perform(&site.operator)
        .await
        .ok()?;

    match remote.address().site() {
        SiteAddress::Ucan(ucan) => Some(ucan.endpoint().to_string()),
        _ => None,
    }
}
