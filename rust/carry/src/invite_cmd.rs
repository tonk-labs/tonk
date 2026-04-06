//! `carry invite` — create an invite token for a collaborator.
//!
//! Generates an ephemeral membership credential, delegates repo access
//! to it, and exports the credential + delegation chain + remote address
//! as a single bearer token. The recipient uses `carry join` to import
//! it, automatically configuring sync.
//!
//! The invite is credential-as-capability: whoever holds the token can
//! join. No `invited_did` is needed.
//!
//! ## Token versions
//!
//! - **v2** (`carry_inv2_`): `[u32 cred_len][cred][chain]`. No remote
//!   info. The recipient gets a delegation chain but must configure sync
//!   manually. Kept for backward compatibility in `decode_token`.
//! - **v3** (`carry_inv3_`): `[u32 cred_len][cred][u32 chain_len][chain][remote_json]`.
//!   Includes the serialized `RemoteAddress` so `carry join` can set up
//!   `remote add` + upstream + `pull` in one step.

use crate::site::Site;
use anyhow::{Context, Result};
use dialog_credentials::Ed25519Signer;
use dialog_credentials::credential::SignerCredential;
use dialog_repository::RemoteAddress;
use dialog_ucan::DelegationChain;
use dialog_varsig::Principal;

/// Current token prefix (v3 — includes remote address).
const TOKEN_PREFIX_V3: &str = "carry_inv3_";

/// Legacy token prefix (v2 — no remote address).
const TOKEN_PREFIX_V2: &str = "carry_inv2_";

/// Execute `carry invite`.
///
/// Generates a bearer invite token that any recipient can use to join.
pub async fn execute(site: &Site, _invited_did: &str) -> Result<()> {
    let token = create_token(site).await?;

    eprintln!("Invite token (share securely with your collaborator):");
    println!("{}", token);

    Ok(())
}

/// Build an invite token from a site's identity and repository.
///
/// Generates an ephemeral membership credential, delegates the repo subject
/// to it, and returns the encoded bearer token. If the repo has a remote
/// configured (via upstream on the main branch), embeds the remote address
/// so `carry join` can set up sync automatically.
pub async fn create_token(site: &Site) -> Result<String> {
    let membership_signer = Ed25519Signer::generate()
        .await
        .context("Failed to generate membership credential")?;
    let membership = SignerCredential::from(membership_signer);

    // The profile claims its existing authority over the repo subject and
    // re-delegates it to the freshly generated membership credential.
    let chain = site
        .profile
        .access()
        .claim(&site.repo)
        .delegate(membership.did())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation to membership credential")?;

    let credential_export = membership
        .export()
        .await
        .context("Failed to export membership credential")?;

    let cred_bytes: &[u8] = credential_export.as_ref();
    let chain_bytes = chain
        .to_bytes()
        .context("Failed to serialize delegation chain")?;

    // Try to resolve the remote address from the branch's upstream.
    let remote_address = resolve_remote_address(site).await;

    match remote_address {
        Some(addr) => encode_v3(cred_bytes, &chain_bytes, &addr),
        None => encode_v2(cred_bytes, &chain_bytes),
    }
}

/// Resolve the `RemoteAddress` from the site's branch upstream, if any.
async fn resolve_remote_address(site: &Site) -> Option<RemoteAddress> {
    use dialog_repository::UpstreamState;

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

    Some(remote.address())
}

/// Encode a v3 token: `[u32 cred_len][cred][u32 chain_len][chain][remote_json]`.
fn encode_v3(cred_bytes: &[u8], chain_bytes: &[u8], remote: &RemoteAddress) -> Result<String> {
    let remote_json = serde_json::to_vec(remote).context("Failed to serialize remote address")?;

    let mut payload =
        Vec::with_capacity(4 + cred_bytes.len() + 4 + chain_bytes.len() + remote_json.len());
    payload.extend_from_slice(&(cred_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(cred_bytes);
    payload.extend_from_slice(&(chain_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(chain_bytes);
    payload.extend_from_slice(&remote_json);

    Ok(format!(
        "{}{}",
        TOKEN_PREFIX_V3,
        bs58::encode(&payload).into_string()
    ))
}

/// Encode a v2 token (fallback when no remote is configured):
/// `[u32 cred_len][cred][chain]`.
fn encode_v2(cred_bytes: &[u8], chain_bytes: &[u8]) -> Result<String> {
    let mut payload = Vec::with_capacity(4 + cred_bytes.len() + chain_bytes.len());
    payload.extend_from_slice(&(cred_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(cred_bytes);
    payload.extend_from_slice(chain_bytes);

    Ok(format!(
        "{}{}",
        TOKEN_PREFIX_V2,
        bs58::encode(&payload).into_string()
    ))
}

/// Decoded invite token contents.
pub struct DecodedToken {
    /// Raw credential export bytes.
    pub cred_bytes: Vec<u8>,
    /// The delegation chain granting access to the repo subject.
    pub chain: DelegationChain,
    /// The remote address (v3 only). `None` for v2 tokens.
    pub remote: Option<RemoteAddress>,
}

/// Decode an invite token (v2 or v3) into its parts.
pub fn decode_token(token: &str) -> Result<DecodedToken> {
    if let Some(stripped) = token.strip_prefix(TOKEN_PREFIX_V3) {
        return decode_v3(stripped);
    }
    if let Some(stripped) = token.strip_prefix(TOKEN_PREFIX_V2) {
        return decode_v2(stripped);
    }
    anyhow::bail!(
        "Invalid invite token: unrecognised prefix (expected carry_inv2_ or carry_inv3_)"
    );
}

fn decode_v3(b58: &str) -> Result<DecodedToken> {
    let payload = bs58::decode(b58)
        .into_vec()
        .context("Invalid invite token: bad base58 encoding")?;

    if payload.len() < 8 {
        anyhow::bail!("Invalid invite token: too short for v3");
    }

    // cred
    let cred_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let cred_end = 4 + cred_len;
    if payload.len() < cred_end + 4 {
        anyhow::bail!("Invalid invite token: credential portion truncated");
    }
    let cred_bytes = payload[4..cred_end].to_vec();

    // chain
    let chain_len = u32::from_le_bytes([
        payload[cred_end],
        payload[cred_end + 1],
        payload[cred_end + 2],
        payload[cred_end + 3],
    ]) as usize;
    let chain_end = cred_end + 4 + chain_len;
    if payload.len() < chain_end {
        anyhow::bail!("Invalid invite token: chain portion truncated");
    }
    let chain = DelegationChain::try_from(&payload[cred_end + 4..chain_end])
        .context("Invalid invite token: failed to decode delegation chain")?;

    // remote
    let remote: RemoteAddress = serde_json::from_slice(&payload[chain_end..])
        .context("Invalid invite token: failed to decode remote address")?;

    Ok(DecodedToken {
        cred_bytes,
        chain,
        remote: Some(remote),
    })
}

fn decode_v2(b58: &str) -> Result<DecodedToken> {
    let payload = bs58::decode(b58)
        .into_vec()
        .context("Invalid invite token: bad base58 encoding")?;

    if payload.len() < 4 {
        anyhow::bail!("Invalid invite token: too short");
    }

    let cred_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;

    if payload.len() < 4 + cred_len {
        anyhow::bail!("Invalid invite token: credential portion truncated");
    }

    let cred_bytes = payload[4..4 + cred_len].to_vec();
    let chain_bytes = &payload[4 + cred_len..];

    let chain = DelegationChain::try_from(chain_bytes)
        .context("Invalid invite token: failed to decode delegation chain")?;

    Ok(DecodedToken {
        cred_bytes,
        chain,
        remote: None,
    })
}
