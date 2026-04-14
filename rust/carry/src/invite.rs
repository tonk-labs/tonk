//! Platform-independent invite primitives.
//!
//! This module contains the URL format, parsing, and redelgation logic
//! shared between the native CLI (`invite_cmd`, `join_cmd`) and the
//! WASM-based web join UI.
//!
//! ## URL format
//!
//! ```text
//! <base>?access=<base58-ucan-chain>[&remote=<access-service-url>]#<base58-private-key>
//! ```
//!
//! - `access`: base58-encoded delegation chain bytes
//! - `remote` (optional): UCAN access service endpoint for sync
//! - `#fragment` (optional): base58-encoded Ed25519 seed for open invites

use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::subject::Subject as UcanSubject;
use dialog_ucan::{DelegationBuilder, DelegationChain};
use dialog_varsig::Did;

/// Default base URL for invite links.
pub const DEFAULT_BASE_URL: &str = "https://tonk.xyz/join";

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

/// Parse an invite URL into its components.
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

/// Build an invite URL from a delegation chain and optional components.
pub fn build_invite_url(
    base_url: &str,
    chain: &DelegationChain,
    remote_url: Option<&str>,
    secret_seed: Option<&[u8]>,
) -> Result<String> {
    let chain_bytes = chain
        .to_bytes()
        .context("Failed to serialize delegation chain")?;
    let access = bs58::encode(&chain_bytes).into_string();

    let mut url = format!("{}?access={}", base_url, access);

    if let Some(remote) = remote_url {
        url.push_str("&remote=");
        url.push_str(remote);
    }

    if let Some(seed) = secret_seed {
        url.push('#');
        url.push_str(&bs58::encode(seed).into_string());
    }

    Ok(url)
}

/// Redelegate from an ephemeral key to a target DID.
///
/// Takes an existing delegation chain (ending at the ephemeral key's DID)
/// and extends it with a new delegation from the ephemeral key to `audience`.
/// Returns the extended chain.
pub async fn redelegate(
    chain: DelegationChain,
    ephemeral_seed: &[u8; 32],
    audience: &Did,
) -> Result<DelegationChain> {
    let ephemeral = Ed25519Signer::import(ephemeral_seed)
        .await
        .context("Failed to import ephemeral key")?;

    let subject = chain
        .subject()
        .cloned()
        .map(UcanSubject::Specific)
        .unwrap_or(UcanSubject::Any);

    let delegation = DelegationBuilder::new()
        .issuer(ephemeral)
        .audience(audience)
        .subject(subject)
        .command(vec![])
        .try_build()
        .await
        .context("Failed to build redelgation")?;

    chain
        .push(delegation)
        .context("Failed to extend delegation chain")
}
