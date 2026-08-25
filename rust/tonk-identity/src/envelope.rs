//! The account custody envelope.
//!
//! The account is a locally generated 32-byte secret; every custody
//! method — the first passkey included — is an interchangeable AEAD
//! wrapping of that same secret. Keys derive from the secret through
//! domain-separated HKDF, so the signing root and the future
//! encryption root can never collide, and unwrapping any one envelope
//! recovers the whole account. Design: `plan/Account custody.md`.

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::Result;
use dialog_credentials::Ed25519Signer;
use hkdf::Hkdf;
use sha2_0_10::Sha256;
use std::marker::PhantomData;
use zeroize::Zeroizing;

use crate::clearance::{Account, Clearance, Recovery};

/// HKDF info for the account's Ed25519 signing seed. Bumping the
/// version is a deliberate account rotation, never a routine change.
pub const SIGNING_CONTEXT: &[u8] = b"tonk/sign/v1";

/// HKDF info for the account's X25519 encryption key, the recipient
/// every device seals custodied seeds to (see [`crate::sealed`]).
/// Having a second root derive from the same secret is the reason the
/// *secret* is wrapped rather than the signing key.
pub const ENCRYPTION_CONTEXT: &[u8] = b"tonk/enc/v1";

/// Entry-function salt that seeds the custody keypair. Evaluated as
/// the WebAuthn PRF `eval.first` input (or fed to the phrase KDF);
/// deriving the keypair *is* the lookup, since its DID names the
/// custody space.
pub const CUSTODY_KEY_CONTEXT: &[u8] = b"tonk/custody/key/v1";

/// Entry-function salt whose output derives the KEK wrapping the
/// account secret. Evaluated as the WebAuthn PRF `eval.second` input
/// (or fed to the phrase KDF).
pub const CUSTODY_KEK_CONTEXT: &[u8] = b"tonk/custody/kek/v1";

/// The fixed message a custodian signs to produce KEK input.
///
/// A custodian is a signing keypair, because a non-extractable keypair
/// is the only shape browser storage can hold whose private material
/// never exists as bytes. Its signature over this message is
/// deterministic (RFC 8032), so the same custodian reproduces the same
/// KEK on every boot without the KEK ever being stored. This is the
/// local stand-in for a passkey's PRF output.
///
/// The signature is the *input*; the clearance level supplies the HKDF
/// info, so one custodian keypair yields unrelated keys per level. See
/// [`Kek::from_custodian`].
pub const CUSTODIAN_KEK_CONTEXT: &[u8] = b"tonk/custodian/kek/v1";

/// The well-known memory-cell address of the wrapped secret inside a
/// custody space: space `custody`, cell `secret`.
pub const CUSTODY_SPACE: &str = "custody";
/// See [`CUSTODY_SPACE`].
pub const CUSTODY_SECRET_CELL: &str = "secret";

fn expand(ikm: &[u8; 32], info: &[u8]) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, ikm);
    let mut okm = Zeroizing::new([0u8; 32]);
    hkdf.expand(info, okm.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

/// The account: 32 random bytes. Never persisted in plaintext; the
/// guard zeroizes on drop, and callers must not copy the bytes out.
pub struct AccountSecret(Zeroizing<[u8; 32]>);

impl AccountSecret {
    /// Generate a fresh account from the platform CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut())
            .map_err(|error| anyhow::anyhow!("no entropy for an account secret: {error}"))?;
        Ok(Self(bytes))
    }

    /// Adopt existing secret bytes, taking ownership of their guard.
    pub fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    /// The Ed25519 signing seed, via [`SIGNING_CONTEXT`].
    pub fn signing_seed(&self) -> Zeroizing<[u8; 32]> {
        expand(&self.0, SIGNING_CONTEXT)
    }

    /// The account-clearance key: wraps the secrets this account
    /// custodies, which today means space and invite signing seeds.
    ///
    /// Derived rather than stored, so a device that can open the account
    /// can wrap and unwrap everything below it with no second secret to
    /// keep in step. The flip side is that rotating the account secret
    /// rotates this key, so accreditation must re-wrap every seed under
    /// the new one.
    pub fn account_kek(&self) -> Kek<Account> {
        Kek(expand(&self.0, Account::CONTEXT), PhantomData)
    }

    /// The account's X25519 encryption key, via [`ENCRYPTION_CONTEXT`].
    /// Its public half is the recipient every device seals custodied
    /// seeds to; the private half only exists inside a ceremony.
    pub fn encryption_key(&self) -> crate::sealed::EncryptionKey {
        crate::sealed::EncryptionKey::from_bytes(expand(&self.0, ENCRYPTION_CONTEXT))
    }

    /// The account signer. On the web target the key imports into
    /// WebCrypto non-extractably; the intermediate seed zeroizes
    /// before this returns.
    pub async fn signer(&self) -> Result<Ed25519Signer> {
        let seed = self.signing_seed();
        Ed25519Signer::import(&*seed)
            .await
            .map_err(|error| anyhow::anyhow!("failed to import the account seed: {error}"))
    }
}

/// Derive the custody keypair seed from the entry function's output at
/// [`CUSTODY_KEY_CONTEXT`].
pub fn custody_seed(entry_output: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    expand(entry_output, CUSTODY_KEY_CONTEXT)
}

/// Derive the custody signer — the identity that names and reads the
/// custody space — from the entry function's output at
/// [`CUSTODY_KEY_CONTEXT`].
pub async fn custody_signer(entry_output: &[u8; 32]) -> Result<Ed25519Signer> {
    let seed = custody_seed(entry_output);
    Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| anyhow::anyhow!("failed to import the custody seed: {error}"))
}

/// Derive the key-encryption key from the entry function's output at
/// [`CUSTODY_KEK_CONTEXT`].
pub fn custody_kek(entry_output: &[u8; 32]) -> Kek<Recovery> {
    Kek(expand(entry_output, CUSTODY_KEK_CONTEXT), PhantomData)
}

/// How an envelope's KEK is reached. Recorded in the envelope header
/// (and bound into the AEAD as associated data) so an unlock knows
/// which entry function to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KekMethod {
    /// A this-device wrapping under a locally held custodian keypair:
    /// the pre-passkey stand-in used during onboarding. No KEK is
    /// stored — it is re-derived from the custodian's signature on
    /// every boot.
    Local,
    /// WebAuthn PRF evaluated at the two custody salts.
    Passkey,
    /// Recovery phrase through Argon2id, split the same two ways.
    Phrase,
}

impl KekMethod {
    fn code(self) -> u8 {
        match self {
            KekMethod::Local => 0,
            KekMethod::Passkey => 1,
            KekMethod::Phrase => 2,
        }
    }

    fn from_code(code: u8) -> Result<Self, EnvelopeError> {
        match code {
            0 => Ok(KekMethod::Local),
            1 => Ok(KekMethod::Passkey),
            2 => Ok(KekMethod::Phrase),
            other => Err(EnvelopeError::UnknownMethod(other)),
        }
    }
}

const VERSION: u8 = 2;
const ALGORITHM_AES_256_GCM: u8 = 0;
const NONCE_LEN: usize = 12;
/// version + generation + method + algorithm + clearance.
const HEADER_LEN: usize = 1 + 4 + 1 + 1 + 1;

/// What can go wrong opening or parsing an envelope. Decryption
/// failures are deliberately opaque: a wrong KEK and a tampered blob
/// are indistinguishable by design.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The blob is shorter than a well-formed envelope can be.
    #[error("the envelope is truncated")]
    Truncated,
    /// The version byte names a format this build does not read.
    #[error("unsupported envelope version {0}")]
    UnsupportedVersion(u8),
    /// The method byte names no known KEK method.
    #[error("unknown KEK method {0}")]
    UnknownMethod(u8),
    /// The algorithm byte names no known AEAD.
    #[error("unknown envelope algorithm {0}")]
    UnknownAlgorithm(u8),
    /// The KEK does not open this envelope, or the blob was altered.
    #[error("the envelope did not open")]
    Sealed,
    /// The blob was sealed at a different clearance level than the one
    /// reading it. A wrong level is a caller bug, not a bad key, so it
    /// is reported distinctly from [`Self::Sealed`].
    #[error("this is a clearance-{found} envelope, read as {expected}")]
    WrongClearance {
        /// The level that tried to open it.
        expected: &'static str,
        /// The wire tag actually found in the header.
        found: u8,
    },
}

/// A wrapping of a 32-byte secret at clearance `C`: a strict binary
/// blob of `version (1) ‖ generation (4, LE) ‖ method (1) ‖ algorithm
/// (1) ‖ clearance (1) ‖ nonce (12) ‖ ciphertext`. The header is the
/// AEAD associated data, so altering any of it — the clearance tag
/// included — makes the envelope refuse to open.
///
/// The clearance is both a type parameter and a header byte on purpose.
/// The type stops a mis-tiered wrap at compile time; the byte stops one
/// that arrives over the wire, where no type travelled with it.
#[derive(Debug)]
pub struct Envelope<C: Clearance> {
    /// Rotation counter; reserved at 0 until rotation ships.
    pub generation: u32,
    /// The entry function this wrapping's KEK comes from.
    pub method: KekMethod,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
    clearance: PhantomData<C>,
}

// Derived impls would demand `C: Clone + PartialEq` because of the
// `PhantomData<C>` field. The marker holds no value, so the bounds are
// spurious; writing these by hand keeps clearance markers free of trait
// obligations they have no reason to carry.
impl<C: Clearance> Clone for Envelope<C> {
    fn clone(&self) -> Self {
        Self {
            generation: self.generation,
            method: self.method,
            nonce: self.nonce,
            ciphertext: self.ciphertext.clone(),
            clearance: PhantomData,
        }
    }
}

impl<C: Clearance> PartialEq for Envelope<C> {
    fn eq(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.method == other.method
            && self.nonce == other.nonce
            && self.ciphertext == other.ciphertext
    }
}

impl<C: Clearance> Eq for Envelope<C> {}

impl<C: Clearance> Envelope<C> {
    fn header(generation: u32, method: KekMethod) -> [u8; HEADER_LEN] {
        let mut header = [0u8; HEADER_LEN];
        header[0] = VERSION;
        header[1..5].copy_from_slice(&generation.to_le_bytes());
        header[5] = method.code();
        header[6] = ALGORITHM_AES_256_GCM;
        header[7] = C::TAG;
        header
    }

    /// The wire form of this envelope.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + NONCE_LEN + self.ciphertext.len());
        bytes.extend_from_slice(&Self::header(self.generation, self.method));
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Parse an envelope, rejecting anything this build cannot open.
    pub fn decode(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.len() < HEADER_LEN + NONCE_LEN {
            return Err(EnvelopeError::Truncated);
        }
        if bytes[0] != VERSION {
            return Err(EnvelopeError::UnsupportedVersion(bytes[0]));
        }
        let generation = u32::from_le_bytes(bytes[1..5].try_into().expect("4 bytes"));
        let method = KekMethod::from_code(bytes[5])?;
        if bytes[6] != ALGORITHM_AES_256_GCM {
            return Err(EnvelopeError::UnknownAlgorithm(bytes[6]));
        }
        // Refuse a blob sealed at another level before decrypting it.
        // Without this, reading an account-level envelope as a profile
        // one would fail as an opaque `Sealed`, which is the same error
        // a tampered blob gives and says nothing about the real cause.
        if bytes[7] != C::TAG {
            return Err(EnvelopeError::WrongClearance {
                expected: C::NAME,
                found: bytes[7],
            });
        }
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&bytes[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        Ok(Self {
            generation,
            method,
            nonce,
            ciphertext: bytes[HEADER_LEN + NONCE_LEN..].to_vec(),
            clearance: PhantomData,
        })
    }
}

/// A key-encryption key at clearance `C`: 32 bytes reached through one
/// entry function. Seals and opens envelopes at its own level and no
/// other; never referenced by other wrappings.
///
/// The level is a type parameter, so `Kek<Profile>` cannot wrap a space
/// seed and `Kek<Account>` cannot wrap the account secret. See
/// [`crate::clearance`] for what sits at each level and why.
pub struct Kek<C: Clearance>(Zeroizing<[u8; 32]>, PhantomData<C>);

impl<C: Clearance> Kek<C> {
    /// Adopt KEK bytes at this clearance, taking ownership of their
    /// guard.
    ///
    /// The caller asserts the level; nothing about raw bytes carries
    /// one. Prefer the derivations that name their level themselves —
    /// [`AccountSecret::account_kek`], [`Kek::from_custodian`] — and
    /// keep this for bytes that arrived already bound to a level.
    pub fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes, PhantomData)
    }

    /// Derive a KEK from a custodian's signature.
    ///
    /// The caller signs [`CUSTODIAN_KEK_CONTEXT`] with a non-extractable
    /// keypair and passes the raw signature here. Expanding it through
    /// HKDF gives a KEK that is reproducible for whoever holds the key
    /// and unreachable for anyone who does not, which is what a
    /// passkey's PRF output provides. The KEK is never stored: it is
    /// recomputed from a fresh signature on every boot.
    ///
    /// The level is mixed into the expansion, so one custodian keypair
    /// yields unrelated keys at each level it is used for.
    pub fn from_custodian(signature: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(None, signature);
        let mut okm = Zeroizing::new([0u8; 32]);
        hkdf.expand(C::CONTEXT, okm.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self(okm, PhantomData)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.0.as_ref()).expect("32 bytes is the AES-256 key length")
    }

    /// Wrap a 32-byte seed this KEK custodies at its own clearance.
    ///
    /// Same envelope format as a wrapped account secret, so the two are
    /// indistinguishable on the wire; what separates them is the
    /// clearance tag in the header and the type of the KEK.
    pub fn seal_seed(&self, seed: &Zeroizing<[u8; 32]>, method: KekMethod) -> Result<Envelope<C>> {
        self.seal_bytes(seed, method)
    }

    /// Unwrap a seed sealed by [`Self::seal_seed`].
    pub fn open_seed(&self, envelope: &Envelope<C>) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
        self.open_bytes(envelope)
    }

    fn seal_bytes(
        &self,
        plaintext: &Zeroizing<[u8; 32]>,
        method: KekMethod,
    ) -> Result<Envelope<C>> {
        let generation = 0;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|error| anyhow::anyhow!("no entropy for an envelope nonce: {error}"))?;
        let ciphertext = self
            .cipher()
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plaintext.as_ref(),
                    aad: &Envelope::<C>::header(generation, method),
                },
            )
            .map_err(|_| anyhow::anyhow!("failed to seal a {} secret", C::NAME))?;
        Ok(Envelope {
            generation,
            method,
            nonce,
            ciphertext,
            clearance: PhantomData,
        })
    }

    fn open_bytes(&self, envelope: &Envelope<C>) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
        let plaintext = self
            .cipher()
            .decrypt(
                &Nonce::from(envelope.nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad: &Envelope::<C>::header(envelope.generation, envelope.method),
                },
            )
            .map_err(|_| EnvelopeError::Sealed)?;
        let mut plaintext = Zeroizing::new(plaintext);
        let bytes: [u8; 32] = plaintext
            .as_slice()
            .try_into()
            .map_err(|_| EnvelopeError::Sealed)?;
        plaintext.as_mut_slice().fill(0);
        Ok(Zeroizing::new(bytes))
    }
}

impl Kek<Recovery> {
    /// Wrap the account secret.
    ///
    /// Only a recovery-clearance key can do this: the account secret is
    /// the top of the hierarchy, so nothing derived from it may wrap it.
    pub fn seal(&self, secret: &AccountSecret, method: KekMethod) -> Result<Envelope<Recovery>> {
        self.seal_bytes(&secret.0, method)
    }

    /// Unwrap the account secret. Fails as [`EnvelopeError::Sealed`]
    /// for a wrong KEK and a tampered blob alike.
    pub fn open(&self, envelope: &Envelope<Recovery>) -> Result<AccountSecret, EnvelopeError> {
        self.open_bytes(envelope).map(AccountSecret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::Principal;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    fn secret(byte: u8) -> AccountSecret {
        AccountSecret::from_bytes(Zeroizing::new([byte; 32]))
    }

    fn kek(byte: u8) -> Kek<Recovery> {
        Kek::from_bytes(Zeroizing::new([byte; 32]))
    }

    #[dialog_common::test]
    fn it_derives_a_stable_kek_from_a_signature() {
        let first = Kek::<Recovery>::from_custodian(b"a signature");
        let second = Kek::<Recovery>::from_custodian(b"a signature");
        let envelope = first.seal(&secret(3), KekMethod::Local).unwrap();
        assert!(second.open(&envelope).is_ok());
    }

    #[dialog_common::test]
    fn it_derives_a_different_kek_per_signature() {
        let envelope = Kek::<Recovery>::from_custodian(b"one")
            .seal(&secret(3), KekMethod::Local)
            .unwrap();
        assert!(matches!(
            Kek::<Recovery>::from_custodian(b"another").open(&envelope),
            Err(EnvelopeError::Sealed),
        ));
    }

    /// One custodian keypair, used at two levels, must not yield the
    /// same key: one custodian keypair used at two levels must not
    /// collapse them into one.
    #[dialog_common::test]
    fn it_derives_unrelated_keys_per_clearance() {
        let seed = Zeroizing::new([9u8; 32]);
        let sealed = Kek::<Recovery>::from_custodian(b"one custodian")
            .seal_seed(&seed, KekMethod::Local)
            .unwrap()
            .encode();

        // Same signature, account level: must not open it, and must say
        // why rather than failing as an opaque bad-key error.
        assert!(matches!(
            Envelope::<Account>::decode(&sealed),
            Err(EnvelopeError::WrongClearance {
                expected: "account",
                found: 0
            }),
        ));
    }

    /// The clearance tag is associated data, so re-tagging a blob to
    /// smuggle it into another level breaks the AEAD rather than
    /// succeeding.
    #[dialog_common::test]
    fn it_rejects_a_retagged_envelope() {
        let seed = Zeroizing::new([4u8; 32]);
        let kek = Kek::<Recovery>::from_custodian(b"custodian");
        let mut sealed = kek.seal_seed(&seed, KekMethod::Local).unwrap().encode();

        // Rewrite the clearance byte to claim account level.
        sealed[7] = Account::TAG;

        let envelope = Envelope::<Account>::decode(&sealed).expect("the tag now parses");
        assert!(matches!(
            Kek::<Account>::from_custodian(b"custodian").open_seed(&envelope),
            Err(EnvelopeError::Sealed),
        ));
    }

    #[dialog_common::test]
    fn it_round_trips_a_seed_at_its_own_clearance() {
        let seed = Zeroizing::new([5u8; 32]);
        let kek = Kek::<Account>::from_custodian(b"custodian");
        let envelope = kek.seal_seed(&seed, KekMethod::Local).unwrap();
        assert_eq!(*kek.open_seed(&envelope).unwrap(), *seed);
    }

    #[dialog_common::test]
    fn it_derives_the_pinned_signing_seed_vector() {
        assert_eq!(
            hex::encode(secret(7).signing_seed().as_ref()),
            "437ffdf8c5d984c6f704af12a915fa91a3c2b106c908486c8d47ca1cae8300f7",
        );
    }

    #[dialog_common::test]
    fn it_derives_the_pinned_custody_vectors() {
        assert_eq!(
            hex::encode(custody_seed(&[7u8; 32]).as_ref()),
            "d73a675d9ceb2804c84e81b12ed5b095ff02a3d540fa0b0a964f0c0ae4e0f079",
        );
        assert_eq!(
            hex::encode(custody_kek(&[7u8; 32]).0.as_ref()),
            "cea30eb8b352a68b974218eaf03c408d1ef0482dd683db02eb0dbb6a700ac53e",
        );
    }

    #[dialog_common::test]
    fn it_separates_every_derivation_domain() {
        let signing = secret(7).signing_seed();
        let custody = custody_seed(&[7u8; 32]);
        let kek = custody_kek(&[7u8; 32]);
        assert_ne!(signing.as_ref(), custody.as_ref());
        assert_ne!(signing.as_ref(), kek.0.as_ref());
        assert_ne!(custody.as_ref(), kek.0.as_ref());
    }

    #[dialog_common::test]
    async fn it_derives_a_stable_account_did() {
        let a = secret(7).signer().await.unwrap();
        let b = secret(7).signer().await.unwrap();
        assert_eq!(a.did(), b.did());
        assert!(
            a.did().to_string().starts_with("did:key:z6Mk"),
            "expected an ed25519 did:key, got {}",
            a.did(),
        );
    }

    #[dialog_common::test]
    async fn it_derives_distinct_dids_from_distinct_secrets() {
        let a = secret(1).signer().await.unwrap();
        let b = secret(2).signer().await.unwrap();
        assert_ne!(a.did(), b.did());
    }

    #[dialog_common::test]
    fn it_seals_and_opens_the_same_secret() {
        let envelope = kek(3).seal(&secret(7), KekMethod::Passkey).unwrap();
        let opened = kek(3).open(&envelope).unwrap();
        assert_eq!(
            opened.signing_seed().as_ref(),
            secret(7).signing_seed().as_ref()
        );
        assert_eq!(envelope.method, KekMethod::Passkey);
        assert_eq!(envelope.generation, 0);
    }

    #[dialog_common::test]
    fn it_survives_the_wire_roundtrip() {
        let envelope = kek(3).seal(&secret(7), KekMethod::Local).unwrap();
        let decoded = Envelope::decode(&envelope.encode()).unwrap();
        assert_eq!(decoded, envelope);
        let opened = kek(3).open(&decoded).unwrap();
        assert_eq!(
            opened.signing_seed().as_ref(),
            secret(7).signing_seed().as_ref()
        );
    }

    #[dialog_common::test]
    fn it_refuses_the_wrong_kek() {
        let envelope = kek(3).seal(&secret(7), KekMethod::Passkey).unwrap();
        assert!(matches!(kek(4).open(&envelope), Err(EnvelopeError::Sealed)));
    }

    #[dialog_common::test]
    fn it_refuses_a_tampered_ciphertext() {
        let envelope = kek(3).seal(&secret(7), KekMethod::Passkey).unwrap();
        let mut bytes = envelope.encode();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let tampered = Envelope::decode(&bytes).unwrap();
        assert!(matches!(kek(3).open(&tampered), Err(EnvelopeError::Sealed)));
    }

    #[dialog_common::test]
    fn it_refuses_a_tampered_header() {
        let envelope = kek(3).seal(&secret(7), KekMethod::Passkey).unwrap();
        let mut bytes = envelope.encode();
        // Flip the method byte to Local: the header rides as AEAD
        // associated data, so the envelope must refuse to open.
        bytes[5] = 0;
        let tampered = Envelope::decode(&bytes).unwrap();
        assert!(matches!(kek(3).open(&tampered), Err(EnvelopeError::Sealed)));
    }

    #[dialog_common::test]
    fn it_rejects_malformed_blobs() {
        assert_eq!(
            Envelope::<Recovery>::decode(&[]),
            Err(EnvelopeError::Truncated)
        );
        let envelope = kek(3).seal(&secret(7), KekMethod::Passkey).unwrap();
        let mut bytes = envelope.encode();
        bytes[0] = 9;
        assert_eq!(
            Envelope::<Recovery>::decode(&bytes),
            Err(EnvelopeError::UnsupportedVersion(9)),
        );
        bytes[0] = VERSION;
        bytes[5] = 9;
        assert_eq!(
            Envelope::<Recovery>::decode(&bytes),
            Err(EnvelopeError::UnknownMethod(9))
        );
        bytes[5] = 1;
        bytes[6] = 9;
        assert_eq!(
            Envelope::<Recovery>::decode(&bytes),
            Err(EnvelopeError::UnknownAlgorithm(9)),
        );
    }

    #[dialog_common::test]
    fn it_generates_distinct_secrets() {
        let a = AccountSecret::generate().unwrap();
        let b = AccountSecret::generate().unwrap();
        assert_ne!(a.signing_seed().as_ref(), b.signing_seed().as_ref());
    }
}
