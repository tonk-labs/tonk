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
use dialog_credentials::secret::{Context as SecretContext, SealedSecret};
use hkdf::Hkdf;
use sha2_0_10::Sha256;
use std::marker::PhantomData;
use zeroize::Zeroizing;

use crate::clearance::{Clearance, Recovery};

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

/// The fixed message a custodian signed to produce KEK input, under the
/// legacy [`KekMethod::Local`].
///
/// A custodian is a signing keypair, because a non-extractable keypair
/// is the only shape browser storage can hold whose private material
/// never exists as bytes. RFC 8032 makes an Ed25519 signature over this
/// message deterministic, which let the same custodian reproduce the
/// same KEK on every boot without the KEK ever being stored. WebCrypto
/// does not promise that, though: Safari randomizes its Ed25519
/// signatures, so there a KEK derived this way differs on every boot
/// and the envelope never opens again. New envelopes use
/// [`KekMethod::Custodian`] instead; this context only opens old ones.
///
/// The signature is the *input*; the clearance level supplies the HKDF
/// info, so one custodian keypair yields unrelated keys per level. See
/// [`Kek::from_custodian`].
pub const CUSTODIAN_KEK_CONTEXT: &[u8] = b"tonk/custodian/kek/v1";

/// The sealing context a custodian's KEK is concealed under
/// ([`KekMethod::Custodian`]). Versioned with the meaning of the sealed
/// bytes: a 32-byte KEK at the clearance the envelope names.
pub const CUSTODIAN_KEK_SEAL_CONTEXT: SecretContext =
    SecretContext::new("tonk/custodian/kek/sealed/v1");

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

    /// The secret's own bytes, for sealing it through a handle.
    ///
    /// [`Kek::seal`] reaches them directly, which a `CryptoKey`-backed
    /// KEK cannot: sealing there goes through
    /// [`crate::webcrypto_kek::seal_seed`], which takes bytes. Not the
    /// signing seed — that is derived *from* this, and an envelope
    /// holding it would recover a signer and nothing else.
    pub(crate) fn material(&self) -> Zeroizing<[u8; 32]> {
        self.0.clone()
    }

    /// The Ed25519 signing seed, via [`SIGNING_CONTEXT`].
    ///
    /// Crate-private: [`Self::signer`] is what callers want, and it
    /// imports the seed non-extractably rather than yielding bytes.
    pub(crate) fn signing_seed(&self) -> Zeroizing<[u8; 32]> {
        expand(&self.0, SIGNING_CONTEXT)
    }

    /// The account's X25519 encryption key, via [`ENCRYPTION_CONTEXT`].
    /// Its public half is the recipient every device seals custodied
    /// seeds to; the private half only exists inside a ceremony.
    ///
    /// Crate-private: [`Self::secret`] offers what this key can do
    /// without handing the key itself to a caller.
    pub(crate) fn encryption_key(&self) -> crate::sealed::EncryptionKey {
        crate::sealed::EncryptionKey::from_bytes(expand(&self.0, ENCRYPTION_CONTEXT))
    }

    /// Seal to and open for this account.
    ///
    /// Mirrors `Ed25519Signer::secret`: a capability over the account's
    /// encryption key, rather than the key.
    pub fn secret(&self) -> crate::sealed::AccountSecretKey<'_> {
        crate::sealed::AccountSecretKey::new(self)
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
    Kek(
        Material::Bytes(expand(entry_output, CUSTODY_KEK_CONTEXT)),
        PhantomData,
    )
}

/// How an envelope's KEK is reached. Recorded in the envelope header
/// (and bound into the AEAD as associated data) so an unlock knows
/// which entry function to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KekMethod {
    /// A this-device wrapping under a locally held custodian keypair,
    /// the KEK re-derived from the custodian's signature over
    /// [`CUSTODIAN_KEK_CONTEXT`] on every boot. Legacy: reproducible
    /// only where signing is deterministic, which WebCrypto does not
    /// guarantee. Opened, never written anew; see [`Self::Custodian`].
    Local,
    /// WebAuthn PRF evaluated at the two custody salts.
    Passkey,
    /// Recovery phrase through Argon2id, split the same two ways.
    Phrase,
    /// A this-device wrapping under a locally held custodian keypair,
    /// the KEK random and sealed to the custodian's `did:key` (a
    /// [`SealedSecret`] stored beside the envelope). Reproducible by
    /// construction: revealing is an X25519 agreement only the
    /// custodian's agreement key completes. Replaces [`Self::Local`].
    Custodian,
}

impl KekMethod {
    fn code(self) -> u8 {
        match self {
            KekMethod::Local => 0,
            KekMethod::Passkey => 1,
            KekMethod::Phrase => 2,
            KekMethod::Custodian => 3,
        }
    }

    fn from_code(code: u8) -> Result<Self, EnvelopeError> {
        match code {
            0 => Ok(KekMethod::Local),
            1 => Ok(KekMethod::Passkey),
            2 => Ok(KekMethod::Phrase),
            3 => Ok(KekMethod::Custodian),
            other => Err(EnvelopeError::UnknownMethod(other)),
        }
    }
}

const VERSION: u8 = 2;
const ALGORITHM_AES_256_GCM: u8 = 0;
pub(crate) const NONCE_LEN: usize = 12;
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

    /// The associated-data header for a given generation and method,
    /// for a sealer that is not [`Kek::seal_bytes`].
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn header_for(generation: u32, method: KekMethod) -> [u8; HEADER_LEN] {
        Self::header(generation, method)
    }

    /// Assemble an envelope from parts a non-`aes_gcm` sealer produced.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn from_parts(
        generation: u32,
        method: KekMethod,
        nonce: [u8; NONCE_LEN],
        ciphertext: Vec<u8>,
    ) -> Self {
        Self {
            generation,
            method,
            nonce,
            ciphertext,
            clearance: PhantomData,
        }
    }

    /// The AEAD nonce, for an opener that is not [`Kek::open_bytes`].
    ///
    /// Exposed so the WebCrypto path (which decrypts from a
    /// non-extractable key handle rather than raw KEK bytes) can pass
    /// the same three inputs the `aes_gcm` path uses.
    pub fn nonce(&self) -> &[u8; NONCE_LEN] {
        &self.nonce
    }

    /// The sealed bytes.
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// The associated data this envelope is bound to. Must be passed as
    /// `additionalData` by any opener, or the tag check fails.
    pub fn aad(&self) -> [u8; HEADER_LEN] {
        Self::header(self.generation, self.method)
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

/// What a KEK is allowed to do, at the type level.
///
/// A KEK backed by a browser `CryptoKey` carries WebCrypto usages, and
/// a decrypt-only handle physically cannot seal. Encoding that in the
/// type means a caller that tries gets a compile error rather than a
/// rejected promise — and, more usefully, it makes "this key can only
/// open" a property a function signature can *require*.
pub mod capability {
    /// Can open envelopes and seal new ones.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Sealing;
    /// Can open envelopes only.
    ///
    /// What a page hands the worker: a leaked opener cannot forge a
    /// wrapping, so the blast radius of the handle is strictly smaller
    /// than the bytes it stands for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Opening;

    /// Implemented by both markers; `SEALS` reports which this is.
    pub trait Capability {
        /// Whether this capability includes sealing.
        const SEALS: bool;
    }
    impl Capability for Sealing {
        const SEALS: bool = true;
    }
    impl Capability for Opening {
        const SEALS: bool = false;
    }
}

use capability::{Capability, Opening, Sealing};

/// How a KEK's key material is held.
///
/// Same shape as `dialog_credentials::Ed25519SigningKey`: one type, a
/// `Bytes` arm that exists everywhere, and a browser arm behind a cfg.
/// Callers stay single — nothing outside this module branches on target.
enum Material {
    /// Raw 32 bytes, used through `aes_gcm`. The only form on native.
    Bytes(Zeroizing<[u8; 32]>),
    /// A WebCrypto key handle. Non-extractable, so there are no bytes
    /// to read; it is used by reference and can cross `postMessage`.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    Handle(web_sys::CryptoKey),
}

/// A key-encryption key at clearance `C`, able to do `K`.
///
/// 32 bytes (or a handle standing for them) reached through one entry
/// function. Seals and opens envelopes at its own level and no other;
/// never referenced by other wrappings.
///
/// Two type parameters, both making a mistake unrepresentable rather
/// than merely wrong:
///
/// - **`C`, the clearance.** `Kek<Profile>` cannot wrap a space seed
///   and `Kek<Account>` cannot wrap the account secret. See
///   [`crate::clearance`].
/// - **`K`, the capability.** [`capability::Opening`] has no `seal`
///   method at all, so handing a decrypt-only handle where a sealer is
///   wanted fails to compile.
pub struct Kek<C: Clearance, K: Capability = Sealing>(Material, PhantomData<(C, K)>);

impl<C: Clearance, K: Capability> Kek<C, K> {
    /// Adopt KEK bytes at this clearance, taking ownership of their
    /// guard.
    ///
    /// The caller asserts the level; nothing about raw bytes carries
    /// one. Prefer the derivations that name their level themselves —
    /// [`Kek::from_custodian`] — and
    /// keep this for bytes that arrived already bound to a level.
    pub fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(Material::Bytes(bytes), PhantomData)
    }

    /// Derive a KEK from a custodian's signature ([`KekMethod::Local`]).
    ///
    /// The caller signs [`CUSTODIAN_KEK_CONTEXT`] with a non-extractable
    /// keypair and passes the raw signature here. Expanding it through
    /// HKDF gives a KEK that is reproducible for whoever holds the key
    /// and unreachable for anyone who does not, provided the signature
    /// itself is reproducible. Safari's WebCrypto randomizes Ed25519
    /// signatures, so this only opens envelopes written before
    /// [`Self::seal_to_custodian`] existed, on platforms that sign
    /// deterministically.
    ///
    /// The level is mixed into the expansion, so one custodian keypair
    /// yields unrelated keys at each level it is used for.
    pub fn from_custodian(signature: &[u8]) -> Self {
        let hkdf = Hkdf::<Sha256>::new(None, signature);
        let mut okm = Zeroizing::new([0u8; 32]);
        hkdf.expand(C::CONTEXT, okm.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Self(Material::Bytes(okm), PhantomData)
    }

    /// The KEK a custodian sealed with [`Self::seal_to_custodian`],
    /// revealed by that same custodian.
    ///
    /// # Errors
    ///
    /// Fails as [`EnvelopeError::Sealed`] for another custodian, a
    /// tampered blob, and a revealed value that is not a KEK alike.
    pub async fn from_custodian_sealed(
        custodian: &Ed25519Signer,
        sealed: &SealedSecret,
    ) -> Result<Self, EnvelopeError> {
        let revealed = Zeroizing::new(
            custodian
                .secret(CUSTODIAN_KEK_SEAL_CONTEXT)
                .reveal(sealed)
                .await
                .map_err(|_| EnvelopeError::Sealed)?,
        );
        let mut bytes = Zeroizing::new([0u8; 32]);
        if revealed.len() != bytes.len() {
            return Err(EnvelopeError::Sealed);
        }
        bytes.copy_from_slice(&revealed);
        Ok(Self(Material::Bytes(bytes), PhantomData))
    }

    /// The `aes_gcm` cipher, for KEKs backed by real bytes.
    ///
    /// `None` for a handle-backed KEK: a non-extractable `CryptoKey` has
    /// no bytes to build one from, which is the whole point of it. Those
    /// go through [`crate::webcrypto_kek::open_with_handle`] instead.
    fn cipher(&self) -> Option<Aes256Gcm> {
        match &self.0 {
            Material::Bytes(bytes) => Some(
                Aes256Gcm::new_from_slice(bytes.as_ref())
                    .expect("32 bytes is the AES-256 key length"),
            ),
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Material::Handle(_) => None,
        }
    }

    /// The raw bytes, for tests that pin a derivation.
    ///
    /// Test-only: production code must not copy KEK material out, which
    /// is the whole reason the handle form exists.
    #[cfg(test)]
    pub(crate) fn expose_bytes(&self) -> Option<&[u8; 32]> {
        match &self.0 {
            Material::Bytes(bytes) => Some(bytes),
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Material::Handle(_) => None,
        }
    }

    /// The WebCrypto handle, when this KEK is backed by one.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn handle(&self) -> Option<&web_sys::CryptoKey> {
        match &self.0 {
            Material::Bytes(_) => None,
            Material::Handle(key) => Some(key),
        }
    }

    /// Unwrap a seed sealed by [`Self::seal_seed`].
    ///
    /// Bytes-backed only. A handle-backed KEK has no bytes to build a
    /// cipher from and returns [`EnvelopeError::Sealed`]; on wasm, call
    /// [`crate::webcrypto_kek::open_seed`], which dispatches on how the
    /// key is held.
    pub fn open_seed(&self, envelope: &Envelope<C>) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
        self.open_bytes(envelope)
    }

    /// [`Self::open_seed`] under a name the wasm dispatcher can call
    /// without shadowing itself.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub(crate) fn open_seed_bytes(
        &self,
        envelope: &Envelope<C>,
    ) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
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
            .ok_or_else(|| anyhow::anyhow!("a handle-backed KEK cannot seal"))?
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
            .ok_or(EnvelopeError::Sealed)?
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

impl<C: Clearance> Kek<C, Opening> {
    /// Adopt a non-extractable WebCrypto key handle as an opener.
    ///
    /// The handle carries only the `decrypt` usage, so this is the one
    /// capability it can have. Built by the page from a passkey's PRF
    /// output (see [`crate::webcrypto_kek::derive_custody_kek_handle`])
    /// and posted to the worker, which can open envelopes with it and
    /// cannot read it.
    ///
    /// The caller asserts the clearance, as with [`Kek::from_bytes`]:
    /// nothing about a key handle carries one.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn from_handle(handle: web_sys::CryptoKey) -> Self {
        Self(Material::Handle(handle), PhantomData)
    }
}

impl<C: Clearance> Kek<C, Sealing> {
    /// Adopt a non-extractable WebCrypto key handle as a sealer.
    ///
    /// The counterpart to [`Kek::from_handle`]: same derived key, but
    /// the handle carries `encrypt` rather than `decrypt`, so this one
    /// can wrap. Built by
    /// [`crate::webcrypto_kek::derive_custody_sealing_handle`].
    ///
    /// Sealing through a handle is not about transport — it happens in
    /// the page that ran the ceremony — but about the raw KEK never
    /// existing: `deriveKey` yields a key that was never readable,
    /// where `custody_kek` yields 32 bytes a caller must be trusted to
    /// zero.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    pub fn from_sealing_handle(handle: web_sys::CryptoKey) -> Self {
        Self(Material::Handle(handle), PhantomData)
    }

    /// A fresh random KEK, sealed to `custodian` so only it can produce
    /// the KEK again ([`KekMethod::Custodian`]).
    ///
    /// Store the returned [`SealedSecret`] beside the envelope; a later
    /// boot gets the KEK back with [`Self::from_custodian_sealed`]. The
    /// seal adds no secrecy the custodian did not already provide, since
    /// whoever can use its agreement key can reveal it; what it adds is
    /// reproducibility. Revealing is an X25519 agreement, so unlike
    /// [`Self::from_custodian`] it does not depend on how the platform
    /// signs.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform has no entropy for the KEK, or
    /// the custodian's identity yields no agreement key to seal to.
    ///
    /// The concrete signer: sealing goes through
    /// `secret(context).conceal`, which reaches an X25519 agreement key
    /// derived from an Ed25519 one. Dialog's algorithm-agnostic `Signer`
    /// offers no such thing.
    pub async fn seal_to_custodian(custodian: &Ed25519Signer) -> Result<(Self, SealedSecret)> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut())
            .map_err(|error| anyhow::anyhow!("no entropy for a custodian KEK: {error}"))?;
        let sealed = custodian
            .secret(CUSTODIAN_KEK_SEAL_CONTEXT)
            .conceal(bytes.as_ref())
            .await
            .map_err(|error| anyhow::anyhow!("failed to seal the KEK to the custodian: {error}"))?;
        Ok((Self(Material::Bytes(bytes), PhantomData), sealed))
    }

    /// Wrap a 32-byte seed this KEK custodies at its own clearance.
    ///
    /// Same envelope format as a wrapped account secret, so the two are
    /// indistinguishable on the wire; what separates them is the
    /// clearance tag in the header and the type of the KEK.
    ///
    /// Only on a [`Sealing`] KEK: an [`Opening`] one is a decrypt-only
    /// handle, and asking it to seal is a compile error rather than a
    /// rejected promise at runtime.
    pub fn seal_seed(&self, seed: &Zeroizing<[u8; 32]>, method: KekMethod) -> Result<Envelope<C>> {
        self.seal_bytes(seed, method)
    }
}

impl Kek<Recovery, Sealing> {
    /// Wrap the account secret.
    ///
    /// Only a recovery-clearance key can do this: the account secret is
    /// the top of the hierarchy, so nothing derived from it may wrap it.
    pub fn seal(&self, secret: &AccountSecret, method: KekMethod) -> Result<Envelope<Recovery>> {
        self.seal_bytes(&secret.0, method)
    }
}

impl<K: Capability> Kek<Recovery, K> {
    /// Unwrap the account secret. Fails as [`EnvelopeError::Sealed`]
    /// for a wrong KEK and a tampered blob alike.
    ///
    /// Available at both capabilities: opening is what an [`Opening`]
    /// KEK is for.
    pub fn open(&self, envelope: &Envelope<Recovery>) -> Result<AccountSecret, EnvelopeError> {
        self.open_bytes(envelope).map(AccountSecret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clearance::Account;
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
    async fn it_reopens_a_kek_sealed_to_its_custodian() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let (kek, sealed) = Kek::<Recovery>::seal_to_custodian(&custodian)
            .await
            .unwrap();
        let envelope = kek.seal(&secret(5), KekMethod::Custodian).unwrap();

        // The sealed KEK reaches the next boot as bytes.
        let sealed = SealedSecret::from_bytes(&sealed.to_bytes()).unwrap();
        let reopened = Kek::<Recovery>::from_custodian_sealed(&custodian, &sealed)
            .await
            .expect("the custodian reveals its own KEK")
            .open(&envelope)
            .expect("the revealed KEK opens the envelope");
        assert_eq!(reopened.0.as_ref(), &[5u8; 32]);
    }

    #[dialog_common::test]
    async fn a_stranger_cannot_reveal_a_custodian_sealed_kek() {
        let custodian = Ed25519Signer::generate().await.unwrap();
        let (_, sealed) = Kek::<Recovery>::seal_to_custodian(&custodian)
            .await
            .unwrap();

        let stranger = Ed25519Signer::generate().await.unwrap();
        assert!(matches!(
            Kek::<Recovery>::from_custodian_sealed(&stranger, &sealed).await,
            Err(EnvelopeError::Sealed)
        ));
    }

    #[dialog_common::test]
    fn it_round_trips_the_custodian_method() {
        let envelope = kek(1).seal(&secret(2), KekMethod::Custodian).unwrap();
        let decoded = Envelope::<Recovery>::decode(&envelope.encode()).unwrap();
        assert_eq!(decoded.method, KekMethod::Custodian);
        assert!(kek(1).open(&decoded).is_ok());
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
            hex::encode(custody_kek(&[7u8; 32]).expose_bytes().unwrap()),
            "cea30eb8b352a68b974218eaf03c408d1ef0482dd683db02eb0dbb6a700ac53e",
        );
    }

    #[dialog_common::test]
    fn it_separates_every_derivation_domain() {
        let signing = secret(7).signing_seed();
        let custody = custody_seed(&[7u8; 32]);
        let kek = custody_kek(&[7u8; 32]);
        assert_ne!(signing.as_ref(), custody.as_ref());
        assert_ne!(signing.as_ref(), kek.expose_bytes().unwrap().as_ref());
        assert_ne!(custody.as_ref(), kek.expose_bytes().unwrap().as_ref());
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
