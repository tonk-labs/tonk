//! `carry invite <DID>` — delegate repository access to a collaborator.
//!
//! Creates a UCAN delegation chain from the repo subject to the given
//! DID, bundles it with the remote address (if configured), and outputs
//! a token that the recipient can redeem with `carry join`.
//!
//! ## Token format
//!
//! A carry invite token is a `carry_inv_` prefix followed by a
//! base58-encoded JSON envelope:
//!
//! ```json
//! {
//!   "chain": "<base58-encoded DelegationChain bytes>",
//!   "url": "https://access.example.com"   // optional
//! }
//! ```
//!
//! The delegation chain contains the subject DID (the repo being shared).
//! The URL is the UCAN-S3 access service endpoint for sync. If the repo
//! has no remote configured, the `url` field is omitted and the recipient
//! must configure sync manually.

use crate::site::Site;
use anyhow::{Context, Result};
use dialog_ucan::DelegationChain;
use dialog_varsig::Did;

/// Token prefix.
const TOKEN_PREFIX: &str = "carry_inv_";

/// Execute `carry invite <DID>`.
pub async fn execute(site: &Site, audience_did: &str) -> Result<()> {
    let audience: Did = audience_did
        .parse()
        .with_context(|| format!("invalid DID: {}", audience_did))?;

    let token = create_token(site, &audience).await?;

    eprintln!("Invite token (share with your collaborator):");
    println!("{}", token);

    Ok(())
}

/// Build an invite token delegating repo access to `audience`.
pub async fn create_token(site: &Site, audience: &Did) -> Result<String> {
    let chain = site
        .profile
        .access()
        .claim(&site.repo)
        .delegate(audience.clone())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation")?;

    let chain_bytes = chain
        .to_bytes()
        .context("Failed to serialize delegation chain")?;

    let url = resolve_access_url(site).await;

    let mut envelope = serde_json::json!({
        "chain": bs58::encode(&chain_bytes).into_string(),
    });
    if let Some(url) = url {
        envelope["url"] = serde_json::Value::String(url);
    }

    let envelope_bytes = serde_json::to_vec(&envelope)?;
    Ok(format!(
        "{}{}",
        TOKEN_PREFIX,
        bs58::encode(&envelope_bytes).into_string()
    ))
}

/// Decoded invite token contents.
pub struct DecodedToken {
    /// The delegation chain granting access to the repo subject.
    pub chain: DelegationChain,
    /// The repo subject DID (extracted from the delegation chain).
    pub subject: Did,
    /// The access service URL, if included.
    pub url: Option<String>,
}

/// Decode an invite token.
pub fn decode_token(token: &str) -> Result<DecodedToken> {
    let stripped = token
        .strip_prefix(TOKEN_PREFIX)
        .context("Invalid invite token: expected carry_inv_ prefix")?;

    let envelope_bytes = bs58::decode(stripped)
        .into_vec()
        .context("Invalid invite token: bad base58 encoding")?;

    let envelope: serde_json::Value =
        serde_json::from_slice(&envelope_bytes).context("Invalid invite token: bad JSON")?;

    let chain_b58 = envelope["chain"]
        .as_str()
        .context("Invalid invite token: missing 'chain' field")?;
    let chain_bytes = bs58::decode(chain_b58)
        .into_vec()
        .context("Invalid invite token: bad base58 in chain")?;
    let chain = DelegationChain::try_from(chain_bytes.as_slice())
        .context("Invalid invite token: failed to decode delegation chain")?;

    let subject = chain
        .subject()
        .cloned()
        .context("Invalid invite token: delegation chain has no subject")?;

    let url = envelope["url"].as_str().map(|s| s.to_string());

    Ok(DecodedToken {
        chain,
        subject,
        url,
    })
}

/// Resolve the access service URL from the repo's upstream remote, if any.
async fn resolve_access_url(site: &Site) -> Option<String> {
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
