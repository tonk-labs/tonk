//! The public principal: a key anyone can derive, which is what makes a
//! space readable by anyone.
//!
//! Its seed is the blake3 hash of no bytes, so every client derives the
//! same key without being told it. Publishing a space delegates
//! `/use/get` on it to this key and leaves the chain in the space as the
//! key's ticket (`ticket/{public did}`), exactly as an invite leaves its
//! grant for an ephemeral key. Nothing about the key is special to a
//! verifier: the delegation's command is what limits it to reads.
//!
//! Each account holds a powerline from this key ([`powerline`]), so a
//! public space's ticket proves for the account's own devices once it is
//! saved: they read under their own keys, never the shared one.

use anyhow::{Context, Result};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::{Did, Principal};

use crate::EphemeralSeed;

/// The public principal's seed: blake3 of no bytes.
pub fn seed() -> EphemeralSeed {
    *blake3::hash(&[]).as_bytes()
}

/// The public principal's key.
///
/// # Errors
///
/// Returns an error if the seed does not import as an Ed25519 key.
pub async fn principal() -> Result<Signer> {
    Ed25519Signer::import(&seed())
        .await
        .map(Signer::from)
        .context("failed to import the public key")
}

/// The public principal's DID: the audience a published space's ticket
/// is kept for.
///
/// # Errors
///
/// Returns an error if the seed does not import as an Ed25519 key.
pub async fn did() -> Result<Did> {
    Ok(principal().await?.did())
}

/// A powerline from the public principal to `account`: anything the
/// public key is granted, the account holds too.
///
/// Anyone can sign as the public key, so this grants nothing that is not
/// already everyone's. What it buys is that a published space's ticket,
/// once saved, proves for the account and so for every device the
/// account has granted, which then read under their own keys.
///
/// # Errors
///
/// Returns an error if the delegation cannot be built or signed.
pub async fn powerline(account: &Did) -> Result<DelegationChain> {
    let delegation = DelegationBuilder::new()
        .issuer(principal().await?)
        .audience(account)
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await
        .map_err(|error| anyhow::anyhow!("failed to mint the public powerline: {error:?}"))?;
    Ok(DelegationChain::new(delegation))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Every client has to arrive at the same key without being told it,
    /// so the seed is pinned: changing it would orphan every published
    /// space's ticket.
    #[dialog_common::test]
    async fn it_derives_the_key_from_the_hash_of_nothing() {
        let hex: String = seed().iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[dialog_common::test]
    async fn it_mints_a_powerline_to_the_account() {
        let account = Ed25519Signer::import(&[3u8; 32]).await.unwrap().did();
        let chain = powerline(&account).await.unwrap();
        assert_eq!(chain.issuer(), &did().await.unwrap());
        assert_eq!(chain.audience(), &account);
        assert!(chain.subject().is_none(), "a powerline names no subject");
    }
}
