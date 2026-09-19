//! The iroh identity this device profile is reachable as, derived rather
//! than stored.
//!
//! # Why derived
//!
//! An earlier version kept a seed in the profile's credential store. It
//! worked, and it was the wrong shape: a second secret with its own
//! lifecycle, which has to be created, migrated and reasoned about
//! separately from the profile it belongs to. A derivation has none of
//! that — the profile is the only secret, and the peer key is a function
//! of it.
//!
//! # Why not the profile or operator key itself
//!
//! Because iroh cannot use either. `iroh::SecretKey` is constructible
//! only from `generate()` or `from_bytes(&[u8; 32])`, and it wraps an
//! `ed25519_dalek::SigningKey`: rustls performs the QUIC handshake in
//! process and needs the key material. There is no external-signer hook
//! to hand a WebCrypto `CryptoKey` to, on any target — iroh has no
//! wasm-specific TLS path, so a browser runs the same rustls/ring stack
//! a native process does.
//!
//! Both dialog keys are non-extractable in a browser: the profile key by
//! construction, and the derived operator key because `Ed25519Signer::
//! import` archives it non-extractable. So the transport key must be a
//! *third* value, derived from the same root and never handed back to
//! WebCrypto. That separation is worth having anyway — the operator goes
//! on signing invocations through a key that never leaves the platform,
//! while this one only ever authenticates a transport.
//!
//! # Stability
//!
//! As stable as the profile export it derives from. Where the seed is
//! extractable the key is the same across restarts. Where the platform
//! will not export it, dialog derives from a *signature* instead — and
//! on engines whose Ed25519 is randomized (Safari; see
//! `project_safari_ed25519_randomized`) that makes the peer key
//! per-session.
//!
//! That is acceptable *here* and nowhere else: the browser is the
//! dialer. Nobody records its identity as a remote, so a name that
//! changes between sessions costs nothing. The dialed side — a `tonk`
//! someone adds as a remote — must be durable, which is why the CLI
//! holds its own key rather than sharing this path.

use dialog_credentials::SignerCredential;

use crate::TonkWorkerError;

/// Domain separation for the transport key.
///
/// Distinct from dialog's own operator context, so this key and the
/// operator are independent values derived from one profile rather than
/// two names for the same bytes.
const PEER_DERIVATION_CONTEXT: &str = "tonk rtc peer identity";

/// The iroh secret key this profile is reachable as.
///
/// Takes the credential and returns an opaque key: the seed is never a
/// value a caller can hold, so nothing downstream can leak or persist
/// it by accident.
pub async fn peer_key(credential: &SignerCredential) -> Result<iroh::SecretKey, TonkWorkerError> {
    let signer = credential.0.as_ed25519().ok_or_else(|| {
        TonkWorkerError::Internal("a peer key needs an ed25519 profile".to_owned())
    })?;

    let material = derivation_material(signer).await?;
    Ok(iroh::SecretKey::from_bytes(&blake3::derive_key(
        PEER_DERIVATION_CONTEXT,
        &material,
    )))
}

/// The bytes the peer key is derived from.
///
/// Mirrors how dialog derives the operator: the exported seed when the
/// platform allows it, and otherwise a signature over a fixed input,
/// because a key that cannot be exported can still be asked to sign.
async fn derivation_material(
    signer: &dialog_credentials::Ed25519Signer,
) -> Result<Vec<u8>, TonkWorkerError> {
    use dialog_credentials::key::KeyExport;

    let export = signer
        .export()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("peer key derivation: {error}")))?;

    match export {
        KeyExport::Extractable(seed) => Ok(seed),
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        KeyExport::NonExtractable { .. } => {
            // The trait lives in `dialog_varsig`, not `dialog_credentials`
            // — the latter exports a `Signer` ENUM under the same name,
            // which shadows it at every re-export path.
            use dialog_varsig::signature::Signer as SignerTrait;

            let signature: dialog_varsig::eddsa::Ed25519Signature =
                SignerTrait::sign(signer, PEER_DERIVATION_CONTEXT.as_bytes())
                    .await
                    .map_err(|error| {
                        TonkWorkerError::Internal(format!("peer key derivation: {error}"))
                    })?;
            let bytes: [u8; 64] = signature.into();
            Ok(bytes.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_credentials::{Ed25519Signer, SignerCredential};

    /// The same profile derives the same peer, every time.
    ///
    /// This is the property a remote depends on: it records an identity
    /// and has to keep reaching it. Deriving means there is nothing
    /// stored to go stale, but it also means a change in the derivation
    /// silently renames every peer — so the pin is on the behaviour
    /// rather than on a constant.
    #[dialog_common::test]
    async fn it_derives_one_peer_key_per_profile() {
        let signer = Ed25519Signer::import(&[9u8; 32]).await.unwrap();
        let credential = SignerCredential(signer.into());

        let first = peer_key(&credential).await.unwrap();
        let second = peer_key(&credential).await.unwrap();

        assert_eq!(
            first.public(),
            second.public(),
            "a profile that derived two identities would be two peers"
        );
    }

    /// A different profile is a different peer.
    #[dialog_common::test]
    async fn it_gives_different_profiles_different_peers() {
        let mine = SignerCredential(Ed25519Signer::import(&[1u8; 32]).await.unwrap().into());
        let theirs = SignerCredential(Ed25519Signer::import(&[2u8; 32]).await.unwrap().into());

        assert_ne!(
            peer_key(&mine).await.unwrap().public(),
            peer_key(&theirs).await.unwrap().public(),
        );
    }

    /// The peer key is not the profile key.
    ///
    /// It cannot be — iroh needs raw bytes and a profile key is
    /// non-extractable in a browser — but asserting it keeps the
    /// separation honest: the transport identity is its own value, and
    /// the signing key never leaves the platform.
    #[dialog_common::test]
    async fn it_is_not_the_profile_key_itself() {
        let signer = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let credential = SignerCredential(signer.clone().into());

        let peer = peer_key(&credential).await.unwrap();
        assert_ne!(
            peer.public().as_bytes(),
            &[7u8; 32],
            "the peer key must not be the seed it derives from"
        );
    }
}
