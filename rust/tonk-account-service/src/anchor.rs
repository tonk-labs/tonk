//! The service's recovery-anchor identity.
//!
//! The anchor is an ordinary Ed25519 keypair. It has no inherent standing:
//! it can act for an account only where that account's root delegated to it
//! at genesis, and that delegation is one visible, revocable link in the
//! user's own chain. Nothing here is a trusted authority — swap the seed and
//! every account that never delegated to the new key is simply unreachable
//! by this service, which is the property we want.

use dialog_credentials::Ed25519Signer;
use zeroize::Zeroizing;

/// Worker secret holding the anchor seed, hex-encoded.
pub const ANCHOR_SEED_SECRET: &str = "RECOVERY_ANCHOR_SEED";

/// Why the configured anchor seed could not be used.
#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    /// The seed is absent, not hex, or the wrong length.
    #[error("recovery anchor seed is unusable: {0}")]
    Seed(String),
    /// The key could not be imported.
    #[error("recovery anchor key could not be imported: {0}")]
    Import(String),
}

/// Build the anchor signer from a hex-encoded 32-byte seed.
pub async fn from_seed_hex(seed_hex: &str) -> Result<Ed25519Signer, AnchorError> {
    let decoded = hex::decode(seed_hex.trim())
        .map_err(|error| AnchorError::Seed(format!("not hex: {error}")))?;
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
        decoded
            .as_slice()
            .try_into()
            .map_err(|_| AnchorError::Seed(format!("expected 32 bytes, got {}", decoded.len())))?,
    );
    Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| AnchorError::Import(error.to_string()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use dialog_varsig::Principal;

    #[dialog_common::test]
    async fn it_derives_a_stable_did_from_one_seed() {
        let seed = hex::encode([3u8; 32]);
        let first = from_seed_hex(&seed).await.unwrap();
        let second = from_seed_hex(&format!(" {seed}\n")).await.unwrap();

        assert_eq!(first.did(), second.did());
    }

    #[dialog_common::test]
    async fn it_rejects_a_seed_of_the_wrong_length() {
        assert!(matches!(
            from_seed_hex(&hex::encode([3u8; 16])).await,
            Err(AnchorError::Seed(_))
        ));
        assert!(matches!(
            from_seed_hex("nonsense").await,
            Err(AnchorError::Seed(_))
        ));
    }
}
