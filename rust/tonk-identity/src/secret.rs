//! Sealing a command's secret field to the worker that runs it.
//!
//! Some commands carry a secret in a field: `tonk/join`'s url holds the
//! invite's seed in its fragment, `account/finish-sign-in-via`'s url holds
//! a grant, an email invitation's id is a bearer token. A command's
//! fields are copied into the session overlay when its invoker follows
//! it, where every view on the branch can read them, and the overlay is
//! handed to the next worker. So the invoker seals such a field before
//! it is sent, and only the worker can open it.
//!
//! The key is derived from the worker's operator: its X25519 private
//! half is `HKDF-SHA256(ikm = operator-derived seed, info =`
//! [`SECRET_CONTEXT`]`)`, where the seed is the operator key's own
//! agreement-based derivation under [`DERIVE_LABEL`]. The operator is
//! derived from the profile and its access branch and is the same across
//! boots and session renewals, so a blob sealed before a worker restart
//! still opens after it. Switching profile or account changes the
//! operator, and a blob sealed for the previous one is refused as
//! [`SecretError::EarlierSession`].
//!
//! The public half is the [`RecipientKey`] invokers seal to; the worker
//! publishes it, and invokers need nothing else. Sealing reuses
//! [`crate::sealed`]: X25519 agreement, HKDF, AES-256-GCM.
//!
//! A sealed field is `recipient public key (32) ‖ sealed blob`. The
//! recipient prefix lets the worker tell a blob sealed for another
//! session from one that was tampered with; the blob's associated data
//! binds the recipient and the attribute it was sealed for, so a blob
//! cannot be moved into another field.

use anyhow::Result;
use dialog_credentials::Ed25519Signer;
use dialog_credentials::key::KeyExport;
use dialog_credentials::secret::{Context, SecretExtractableDerive};
use hkdf::Hkdf;
use sha2_0_10::Sha256;
use zeroize::Zeroizing;

use crate::sealed::{EncryptionKey, RecipientKey, Sealed, SealedError};

/// The dialog derivation context the operator key derives the seed under.
const DERIVE: Context = Context::new("tonk/command-secret");

/// The label the seed is derived under. Changing it changes every
/// worker's recipient, which refuses every blob sealed before.
pub const DERIVE_LABEL: &[u8] = b"v1";

/// HKDF info turning the derived seed into the X25519 private key.
pub const SECRET_CONTEXT: &[u8] = b"tonk/command-secret/v1";

/// Prefix of every binding, so a command secret can never be confused
/// with a seed sealed under [`crate::sealed`] for a subject.
const BINDING_PREFIX: &[u8] = b"tonk/command-secret/v1/";

/// The length of the recipient prefix on a sealed field.
const RECIPIENT_LEN: usize = 32;

/// Why a secret field did not open. Each one refuses the command.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    /// The field holds no sealed blob at all, such as plain text from an
    /// invoker that did not seal it.
    #[error("the secret was sent without being sealed")]
    NotSealed,
    /// The field is shaped like no sealed blob this build reads.
    #[error("the sealed secret is malformed: {0}")]
    Malformed(SealedError),
    /// The blob was sealed for another worker key: one from before a
    /// profile or account switch, or another device's.
    #[error("the secret was sealed for an earlier session")]
    EarlierSession,
    /// The blob is addressed to this worker but does not open for this
    /// field: it was altered, or moved from another field.
    #[error("the sealed secret did not open")]
    Refused,
}

/// What opens a secret sealed to this worker. Derived from the operator,
/// never stored.
pub struct SecretKey(EncryptionKey);

impl SecretKey {
    /// Derive the key from the operator's signer.
    ///
    /// Deterministic per operator: the same profile on the same access
    /// branch derives the same key on every boot.
    pub async fn derive(operator: &Ed25519Signer) -> Result<Self> {
        let derived = SecretExtractableDerive::derive(&operator.secret(DERIVE), DERIVE_LABEL)
            .await
            .map_err(|error| anyhow::anyhow!("the operator key did not derive: {error}"))?;
        #[allow(irrefutable_let_patterns)]
        let KeyExport::Extractable(seed) = derived
            .export()
            .await
            .map_err(|error| anyhow::anyhow!("the derived key did not export: {error:?}"))?
        else {
            anyhow::bail!("the derived key is not extractable");
        };
        let seed = Zeroizing::new(seed);
        Ok(Self::from_seed(&seed))
    }

    /// The key for `seed`: `HKDF-SHA256(seed, info = SECRET_CONTEXT)`.
    pub(crate) fn from_seed(seed: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(None, seed);
        let mut okm = Zeroizing::new([0u8; 32]);
        hkdf.expand(SECRET_CONTEXT, okm.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self(EncryptionKey::from_bytes(okm))
    }

    /// The public half invokers seal to.
    pub fn recipient(&self) -> RecipientKey {
        self.0.recipient()
    }

    /// Open `field`, sealed to this key for `attribute`.
    pub fn reveal(&self, field: &[u8], attribute: &str) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        let (recipient, sealed) = split(field)?;
        if recipient != self.recipient() {
            return Err(SecretError::EarlierSession);
        }
        self.0
            .open_bound(&sealed, &binding(attribute))
            .map_err(|_| SecretError::Refused)
    }

    /// Check that `field` opens for `attribute` without keeping what it
    /// holds.
    pub fn verify(&self, field: &[u8], attribute: &str) -> Result<(), SecretError> {
        self.reveal(field, attribute).map(drop)
    }
}

impl RecipientKey {
    /// Seal `plaintext` for `attribute` to this recipient: what an
    /// invoker puts in a secret field instead of the plaintext.
    pub fn seal_secret(&self, plaintext: &[u8], attribute: &str) -> Result<Vec<u8>> {
        let sealed = self.seal_bound(plaintext, &binding(attribute))?;
        let mut field = Vec::with_capacity(RECIPIENT_LEN + plaintext.len() + 64);
        field.extend_from_slice(self.as_bytes());
        field.extend_from_slice(&sealed.encode());
        Ok(field)
    }

    /// The recipient a sealed field is addressed to, without opening it.
    pub fn of_sealed(field: &[u8]) -> Result<Self, SecretError> {
        split(field).map(|(recipient, _)| recipient)
    }
}

fn binding(attribute: &str) -> Vec<u8> {
    [BINDING_PREFIX, attribute.as_bytes()].concat()
}

fn split(field: &[u8]) -> Result<(RecipientKey, Sealed), SecretError> {
    if field.len() < RECIPIENT_LEN {
        return Err(SecretError::Malformed(SealedError::Truncated));
    }
    let (recipient, blob) = field.split_at(RECIPIENT_LEN);
    let recipient: [u8; RECIPIENT_LEN] = recipient.try_into().expect("32 bytes");
    let sealed = Sealed::decode(blob).map_err(SecretError::Malformed)?;
    Ok((RecipientKey::from_bytes(recipient), sealed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::Principal as _;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    const URL: &str = "xyz.tonk.command.join/sealed-url";

    fn key(byte: u8) -> SecretKey {
        SecretKey::from_seed(&[byte; 32])
    }

    #[dialog_common::test]
    fn it_opens_what_was_sealed_to_it_for_the_same_attribute() {
        let key = key(1);
        let field = key
            .recipient()
            .seal_secret(b"https://tonk.example/join#seed", URL)
            .unwrap();
        let opened = key.reveal(&field, URL).unwrap();
        assert_eq!(opened.as_slice(), b"https://tonk.example/join#seed");
    }

    #[dialog_common::test]
    fn the_sealed_field_does_not_hold_the_plaintext() {
        let field = key(1)
            .recipient()
            .seal_secret(b"https://tonk.example/join#seed", URL)
            .unwrap();
        assert!(!field.windows(4).any(|window| window == b"seed"));
    }

    #[dialog_common::test]
    fn it_refuses_a_blob_sealed_for_another_session() {
        let field = key(1).recipient().seal_secret(b"secret", URL).unwrap();
        assert_eq!(key(2).reveal(&field, URL), Err(SecretError::EarlierSession));
    }

    #[dialog_common::test]
    fn it_refuses_a_blob_moved_to_another_attribute() {
        let key = key(1);
        let field = key.recipient().seal_secret(b"secret", URL).unwrap();
        assert_eq!(
            key.reveal(&field, "xyz.tonk.command.finish-sign-in-via/sealed-url"),
            Err(SecretError::Refused)
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_tampered_blob() {
        let key = key(1);
        let mut field = key.recipient().seal_secret(b"secret", URL).unwrap();
        let last = field.len() - 1;
        field[last] ^= 1;
        assert_eq!(key.reveal(&field, URL), Err(SecretError::Refused));
    }

    #[dialog_common::test]
    fn it_refuses_plaintext_as_malformed() {
        assert!(matches!(
            key(1).reveal(b"https://tonk.example/join#seed", URL),
            Err(SecretError::Malformed(_))
        ));
        assert!(matches!(
            key(1).reveal(b"short", URL),
            Err(SecretError::Malformed(SealedError::Truncated))
        ));
    }

    #[dialog_common::test]
    fn the_same_seed_derives_the_same_recipient() {
        assert_eq!(key(1).recipient(), key(1).recipient());
        assert_ne!(key(1).recipient(), key(2).recipient());
        assert!(
            key(1)
                .recipient()
                .did()
                .as_str()
                .starts_with("did:key:z6LS")
        );
    }

    #[dialog_common::test]
    async fn the_same_operator_derives_the_same_key() {
        let operator = Ed25519Signer::import(&[7u8; 32]).await.unwrap();
        let first = SecretKey::derive(&operator).await.unwrap();
        let again = SecretKey::derive(&operator).await.unwrap();
        assert_eq!(first.recipient(), again.recipient());
        let other = Ed25519Signer::import(&[8u8; 32]).await.unwrap();
        let other = SecretKey::derive(&other).await.unwrap();
        assert_ne!(first.recipient(), other.recipient());
    }
}
