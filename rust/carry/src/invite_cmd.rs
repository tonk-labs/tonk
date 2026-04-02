//! `carry invite` — create an invite token for a collaborator.
//!
//! Generates an ephemeral membership credential, delegates repo access
//! to it, and exports the credential + delegation chain as a single
//! bearer token. The recipient uses `carry join` to import it.
//!
//! The invite is credential-as-capability: whoever holds the token can
//! join. No `invited_did` is needed.

use crate::site::Site;
use anyhow::{Context, Result};
use dialog_capability::Subject;
use dialog_capability::ucan::Ucan;
use dialog_credentials::Ed25519Signer;
use dialog_credentials::credential::SignerCredential;
use dialog_ucan::DelegationChain;
use dialog_varsig::Principal;

/// Invite token prefix (version 2 — credential-as-capability model).
const TOKEN_PREFIX: &str = "carry_inv2_";

/// Execute `carry invite`.
///
/// Generates a bearer invite token that any recipient can use to join.
pub async fn execute(site: &Site, _invited_did: &str) -> Result<()> {
    // Generate an ephemeral membership credential
    let membership_signer = Ed25519Signer::generate()
        .await
        .context("Failed to generate membership credential")?;
    let membership = SignerCredential::from(membership_signer);

    // Delegate repo subject from profile → membership credential
    let chain = Ucan::delegate(&Subject::from(site.repo.did()))
        .issuer(site.profile.credential().signer().clone())
        .audience(membership.did())
        .perform(&site.operator)
        .await
        .context("Failed to create delegation to membership credential")?;

    // Export the membership credential
    let credential_export = membership
        .export()
        .await
        .context("Failed to export membership credential")?;

    // Serialize: credential_export_bytes || delegation_chain_bytes
    // with a 4-byte length prefix for the credential portion
    let cred_bytes: &[u8] = credential_export.as_ref();
    let chain_bytes = chain
        .to_bytes()
        .context("Failed to serialize delegation chain")?;

    let mut payload = Vec::with_capacity(4 + cred_bytes.len() + chain_bytes.len());
    payload.extend_from_slice(&(cred_bytes.len() as u32).to_le_bytes());
    payload.extend_from_slice(cred_bytes);
    payload.extend_from_slice(&chain_bytes);

    // Encode as base58 with prefix
    let token = format!("{}{}", TOKEN_PREFIX, bs58::encode(&payload).into_string());

    eprintln!("Invite token (share securely with your collaborator):");
    println!("{}", token);

    Ok(())
}

/// Decode an invite token into its credential export bytes and delegation chain.
///
/// Public for use by `join_cmd`.
pub fn decode_token(token: &str) -> Result<(Vec<u8>, DelegationChain)> {
    let stripped = token
        .strip_prefix(TOKEN_PREFIX)
        .context("Invalid invite token: missing carry_inv2_ prefix")?;

    let payload = bs58::decode(stripped)
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

    Ok((cred_bytes, chain))
}
