//! Sealing a seed to the account's public encryption key.
//!
//! The account KEK ([`crate::envelope::Kek`]) only exists inside a
//! ceremony, and a CLI device never runs one, so wrapping a space seed
//! under it would make space creation a browser-only act. Instead every
//! device seals to the account's X25519 public key, which it already
//! holds from the account space, and only a ceremony, which derives the
//! private half from the account secret, can open. Design:
//! `plan/authority-facts.md`, "Wrapped keys".
//!
//! Sealing is public, opening is recovery-gated, so the clearance model
//! is unchanged: a device compromise leaks nothing it did not already
//! hold, an account compromise costs the seeds.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::Result;
use dialog_varsig::{Did, Principal};
use hkdf::Hkdf;
use sha2_0_10::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

/// HKDF info for the AEAD key derived from the ECDH shared secret.
pub const SEAL_CONTEXT: &[u8] = b"tonk/seal/v1";

/// Multicodec `x25519-pub` (`0xec`) as an unsigned varint: the
/// `did:key` prefix for an X25519 public key.
const X25519_PUB_CODEC: [u8; 2] = [0xec, 0x01];

const VERSION: u8 = 1;
const ALGORITHM_X25519_AES_256_GCM: u8 = 0;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// version + algorithm + ephemeral public key.
const HEADER_LEN: usize = 1 + 1 + KEY_LEN;

/// What can go wrong opening or parsing a sealed blob. Decryption
/// failures are opaque on purpose: a wrong key, a wrong subject, and a
/// tampered blob are indistinguishable by design.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealedError {
    /// The blob is shorter than a well-formed sealed seed can be.
    #[error("the sealed seed is truncated")]
    Truncated,
    /// The version byte names a format this build does not read.
    #[error("unsupported sealed seed version {0}")]
    UnsupportedVersion(u8),
    /// The algorithm byte names no known scheme.
    #[error("unknown sealed seed algorithm {0}")]
    UnknownAlgorithm(u8),
    /// The key does not open this blob for this subject, or the blob
    /// was altered.
    #[error("the sealed seed did not open")]
    Sealed,
    /// The DID is not a `did:key` carrying an X25519 public key.
    #[error("not an X25519 did:key: {0}")]
    NotAnX25519Key(String),
}

/// The account's X25519 private key: what opens a [`Sealed`] seed.
/// Derived from the account secret inside a ceremony, never stored.
///
/// Crate-private on purpose. Callers reach sealing through
/// [`Secret`] and [`Seal`], which offer the capability without ever
/// handing out the key — the way `dialog_credentials::secret` does.
pub(crate) struct EncryptionKey(StaticSecret);

impl EncryptionKey {
    /// Adopt 32 secret bytes as an X25519 key. The bytes are clamped
    /// by the curve as usual.
    pub(crate) fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(StaticSecret::from(*bytes))
    }

    /// The public half: the recipient every device seals to.
    pub(crate) fn recipient(&self) -> RecipientKey {
        RecipientKey(PublicKey::from(&self.0))
    }

    /// Open a seed sealed to this key for `subject`. Fails as
    /// [`SealedError::Sealed`] for another key, another subject, and a
    /// tampered blob alike.
    pub(crate) fn open(
        &self,
        sealed: &Sealed,
        subject: &Did,
    ) -> Result<Zeroizing<[u8; 32]>, SealedError> {
        let recipient = self.recipient();
        let shared = Zeroizing::new(self.0.diffie_hellman(&sealed.ephemeral).to_bytes());
        let cipher = cipher(&shared, &sealed.ephemeral, &recipient.0);
        let plaintext = cipher
            .decrypt(
                &Nonce::from(sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: &sealed.associated_data(&recipient, subject),
                },
            )
            .map_err(|_| SealedError::Sealed)?;
        let mut plaintext = Zeroizing::new(plaintext);
        let bytes: [u8; 32] = plaintext
            .as_slice()
            .try_into()
            .map_err(|_| SealedError::Sealed)?;
        plaintext.as_mut_slice().fill(0);
        Ok(Zeroizing::new(bytes))
    }
}

/// An X25519 public key: the addressee of a [`Sealed`] seed. Carried
/// in facts as a `did:key` under the `x25519-pub` multicodec
/// (`did:key:z6LS…`), so a recipient is an entity like any other
/// reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecipientKey(PublicKey);

impl Principal for RecipientKey {
    /// The `did:key:z6LS…` form.
    fn did(&self) -> Did {
        let mut bytes = Vec::with_capacity(X25519_PUB_CODEC.len() + KEY_LEN);
        bytes.extend_from_slice(&X25519_PUB_CODEC);
        bytes.extend_from_slice(self.0.as_bytes());
        let encoded = bs58::encode(bytes).into_string();
        format!("did:key:z{encoded}")
            .parse()
            .expect("a did:key with a method and identifier")
    }
}

impl RecipientKey {
    /// Seal to this recipient, holding nothing that could open.
    ///
    /// Mirrors `Ed25519Verifier::secret`: the public identity offers a
    /// conceal-only capability.
    pub fn secret(&self) -> AccountSeal {
        AccountSeal(*self)
    }

    /// Seal a 32-byte seed for `subject` to this recipient. Needs no
    /// secret: a fresh ephemeral X25519 key is agreed against the
    /// recipient and discarded.
    fn seal(&self, seed: &Zeroizing<[u8; 32]>, subject: &Did) -> Result<Sealed> {
        let mut ephemeral_bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(ephemeral_bytes.as_mut())
            .map_err(|error| anyhow::anyhow!("no entropy for an ephemeral key: {error}"))?;
        let ephemeral_secret = StaticSecret::from(*ephemeral_bytes);
        let ephemeral = PublicKey::from(&ephemeral_secret);
        let shared = Zeroizing::new(ephemeral_secret.diffie_hellman(&self.0).to_bytes());

        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|error| anyhow::anyhow!("no entropy for a seal nonce: {error}"))?;
        let mut sealed = Sealed {
            ephemeral,
            nonce,
            ciphertext: Vec::new(),
        };
        sealed.ciphertext = cipher(&shared, &ephemeral, &self.0)
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: seed.as_ref(),
                    aad: &sealed.associated_data(self, subject),
                },
            )
            .map_err(|_| anyhow::anyhow!("failed to seal the seed"))?;
        Ok(sealed)
    }
}

/// Seals to one recipient. Public-key only: conceals, cannot reveal.
///
/// Reached through [`RecipientKey::secret`], the way dialog's `Seal`
/// comes from a verifier.
#[derive(Debug, Clone, Copy)]
pub struct AccountSeal(RecipientKey);

impl AccountSeal {
    /// Conceal a seed belonging to `subject` so only this recipient
    /// can reveal it.
    ///
    /// `subject` is the space or invite the seed belongs to, not the
    /// account. It binds the seal as associated data, so a blob sealed
    /// for one subject refuses to open as another.
    pub fn conceal(&self, seed: &Zeroizing<[u8; 32]>, subject: &Did) -> Result<Sealed> {
        self.0.seal(seed, subject)
    }
}

impl From<RecipientKey> for AccountSeal {
    fn from(key: RecipientKey) -> Self {
        Self(key)
    }
}

impl From<AccountSecretKey<'_>> for AccountSeal {
    /// An account that can open can also be sealed to.
    ///
    /// Lets a caller holding the account pass it where only sealing is
    /// wanted — rotation's target, say — without reaching past the
    /// capability for a key.
    fn from(key: AccountSecretKey<'_>) -> Self {
        key.recipient().secret()
    }
}

impl Principal for AccountSeal {
    fn did(&self) -> Did {
        self.0.did()
    }
}

/// Seals to and opens for one account.
///
/// Reached through [`crate::envelope::AccountSecret::secret`], the way
/// dialog's `Secret` comes from a signer. Holds the account secret, so
/// it derives the encryption key per call rather than storing one — the
/// key never leaves this module.
#[derive(Clone, Copy)]
pub struct AccountSecretKey<'a>(&'a crate::envelope::AccountSecret);

impl<'a> AccountSecretKey<'a> {
    pub(crate) fn new(account: &'a crate::envelope::AccountSecret) -> Self {
        Self(account)
    }

    /// Conceal a seed belonging to `subject` to this account.
    pub fn conceal(&self, seed: &Zeroizing<[u8; 32]>, subject: &Did) -> Result<Sealed> {
        self.0.encryption_key().recipient().seal(seed, subject)
    }

    /// Reveal a seed sealed to this account for `subject`.
    ///
    /// Fails as [`SealedError::Sealed`] for another account, another
    /// subject, and a tampered blob alike.
    pub fn reveal(
        &self,
        sealed: &Sealed,
        subject: &Did,
    ) -> Result<Zeroizing<[u8; 32]>, SealedError> {
        self.0.encryption_key().open(sealed, subject)
    }

    /// The recipient this account is sealed to: what a ceremony
    /// publishes as the account's `AccountSealedInbox` fact.
    pub fn recipient(&self) -> RecipientKey {
        self.0.encryption_key().recipient()
    }
}

impl Principal for AccountSecretKey<'_> {
    /// The `did:key:z6LS…` of the recipient half, not the account's
    /// Ed25519 identity.
    fn did(&self) -> Did {
        self.recipient().did()
    }
}

impl TryFrom<&Did> for RecipientKey {
    type Error = SealedError;

    /// Parse a `did:key:z6LS…`.
    fn try_from(did: &Did) -> Result<Self, SealedError> {
        let invalid = || SealedError::NotAnX25519Key(did.to_string());
        let encoded = did.as_str().strip_prefix("did:key:z").ok_or_else(invalid)?;
        let bytes = bs58::decode(encoded).into_vec().map_err(|_| invalid())?;
        let key = bytes
            .strip_prefix(&X25519_PUB_CODEC[..])
            .ok_or_else(invalid)?;
        let key: [u8; KEY_LEN] = key.try_into().map_err(|_| invalid())?;
        Ok(Self(PublicKey::from(key)))
    }
}

/// The AEAD key for one seal: `HKDF(shared, salt = ephemeral ‖
/// recipient, info = SEAL_CONTEXT)`. Both public keys go into the salt so
/// the key is bound to this exact agreement.
fn cipher(shared: &Zeroizing<[u8; 32]>, ephemeral: &PublicKey, recipient: &PublicKey) -> Aes256Gcm {
    let mut salt = [0u8; KEY_LEN * 2];
    salt[..KEY_LEN].copy_from_slice(ephemeral.as_bytes());
    salt[KEY_LEN..].copy_from_slice(recipient.as_bytes());
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.as_ref());
    let mut okm = Zeroizing::new([0u8; 32]);
    hkdf.expand(SEAL_CONTEXT, okm.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    Aes256Gcm::new_from_slice(okm.as_ref()).expect("32 bytes is the AES-256 key length")
}

/// A seed sealed to one recipient for one subject: `version (1) ‖
/// algorithm (1) ‖ ephemeral public key (32) ‖ nonce (12) ‖
/// ciphertext`. The header, the recipient DID, and the subject DID are
/// the associated data, so a blob cannot be re-pointed at another space
/// or another account without refusing to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    ephemeral: PublicKey,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl Sealed {
    fn header(&self) -> [u8; HEADER_LEN] {
        let mut header = [0u8; HEADER_LEN];
        header[0] = VERSION;
        header[1] = ALGORITHM_X25519_AES_256_GCM;
        header[2..].copy_from_slice(self.ephemeral.as_bytes());
        header
    }

    fn associated_data(&self, recipient: &RecipientKey, subject: &Did) -> Vec<u8> {
        let recipient = recipient.did();
        let mut aad =
            Vec::with_capacity(HEADER_LEN + recipient.as_str().len() + subject.as_str().len());
        aad.extend_from_slice(&self.header());
        aad.extend_from_slice(recipient.as_str().as_bytes());
        aad.extend_from_slice(subject.as_str().as_bytes());
        aad
    }

    /// The wire form.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + NONCE_LEN + self.ciphertext.len());
        bytes.extend_from_slice(&self.header());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Parse a sealed seed, rejecting anything this build cannot open.
    pub fn decode(bytes: &[u8]) -> Result<Self, SealedError> {
        if bytes.len() < HEADER_LEN + NONCE_LEN {
            return Err(SealedError::Truncated);
        }
        if bytes[0] != VERSION {
            return Err(SealedError::UnsupportedVersion(bytes[0]));
        }
        if bytes[1] != ALGORITHM_X25519_AES_256_GCM {
            return Err(SealedError::UnknownAlgorithm(bytes[1]));
        }
        let ephemeral: [u8; KEY_LEN] = bytes[2..HEADER_LEN].try_into().expect("32 bytes");
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        Ok(Self {
            ephemeral: PublicKey::from(ephemeral),
            nonce,
            ciphertext: bytes[HEADER_LEN + NONCE_LEN..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::AccountSecret;
    use dialog_varsig::did;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    fn account(byte: u8) -> AccountSecret {
        AccountSecret::from_bytes(Zeroizing::new([byte; 32]))
    }

    fn seed(byte: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new([byte; 32])
    }

    #[dialog_common::test]
    fn it_seals_to_the_recipient_and_opens_with_the_account_key() {
        let key = account(1);
        let subject = did!("key:z6MkSpace");
        let sealed = key.secret().conceal(&seed(7), &subject).unwrap();
        let opened = key.secret().reveal(&sealed, &subject).unwrap();
        assert_eq!(*opened, [7u8; 32]);
    }

    #[dialog_common::test]
    fn it_survives_the_wire() {
        let key = account(1);
        let subject = did!("key:z6MkSpace");
        let sealed = key.secret().conceal(&seed(7), &subject).unwrap();
        let decoded = Sealed::decode(&sealed.encode()).unwrap();
        assert_eq!(decoded, sealed);
        assert_eq!(*key.secret().reveal(&decoded, &subject).unwrap(), [7u8; 32]);
    }

    #[dialog_common::test]
    fn it_refuses_another_account_key() {
        let subject = did!("key:z6MkSpace");
        let sealed = account(1).secret().conceal(&seed(7), &subject).unwrap();
        assert_eq!(
            account(2).secret().reveal(&sealed, &subject),
            Err(SealedError::Sealed)
        );
    }

    #[dialog_common::test]
    fn it_refuses_another_subject() {
        let key = account(1);
        let sealed = key
            .secret()
            .conceal(&seed(7), &did!("key:z6MkSpace"))
            .unwrap();
        assert_eq!(
            key.secret().reveal(&sealed, &did!("key:z6MkOther")),
            Err(SealedError::Sealed)
        );
    }

    #[dialog_common::test]
    fn it_refuses_a_tampered_blob() {
        let key = account(1);
        let subject = did!("key:z6MkSpace");
        let mut bytes = key.secret().conceal(&seed(7), &subject).unwrap().encode();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let sealed = Sealed::decode(&bytes).unwrap();
        assert_eq!(
            key.secret().reveal(&sealed, &subject),
            Err(SealedError::Sealed)
        );
    }

    #[dialog_common::test]
    fn it_names_the_recipient_as_an_x25519_did_key() {
        let recipient = account(1).secret().recipient();
        let did = recipient.did();
        assert!(did.as_str().starts_with("did:key:z6LS"), "{did}");
        assert_eq!(RecipientKey::try_from(&did).unwrap(), recipient);
    }

    #[dialog_common::test]
    fn it_refuses_an_ed25519_did_key_as_a_recipient() {
        let ed25519 = did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK");
        assert!(matches!(
            RecipientKey::try_from(&ed25519),
            Err(SealedError::NotAnX25519Key(_))
        ));
    }

    #[dialog_common::test]
    fn it_derives_the_same_recipient_from_the_same_secret() {
        assert_eq!(
            account(1).secret().recipient(),
            account(1).secret().recipient()
        );
        assert_ne!(
            account(1).secret().recipient(),
            account(2).secret().recipient()
        );
    }

    #[dialog_common::test]
    fn it_rejects_malformed_blobs() {
        assert_eq!(Sealed::decode(&[1, 0]), Err(SealedError::Truncated));
        let mut bytes = vec![9u8; HEADER_LEN + NONCE_LEN];
        assert_eq!(
            Sealed::decode(&bytes),
            Err(SealedError::UnsupportedVersion(9))
        );
        bytes[0] = VERSION;
        assert_eq!(
            Sealed::decode(&bytes),
            Err(SealedError::UnknownAlgorithm(9))
        );
    }
}
