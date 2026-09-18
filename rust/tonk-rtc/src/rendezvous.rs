//! The certificate both ends derive, from a phrase both ends know.
//!
//! A browser dialling a listener has to write that listener's DTLS
//! fingerprint into a description it fabricates, and cannot skip the
//! check the way `webrtc-rs` can. Two of the three things it needs are
//! derivable — the port is agreed, the candidate is loopback — and the
//! fingerprint is the third.
//!
//! A fingerprint is the hash of a whole *certificate*, not of a key, so
//! deriving the key from a phrase is not enough: both ends have to
//! produce the same certificate byte for byte. That fails with ordinary
//! tooling, because ECDSA signs with a random nonce — `rcgen`'s default
//! signer gives different bytes for the same key every time, which
//! [`a_certificate_is_not_reproducible_with_a_random_nonce`] pins.
//!
//! It succeeds with a deterministic one. RFC 6979 derives the nonce
//! from the key and the message, so the signature — and therefore the
//! certificate, and therefore the fingerprint — is a pure function of
//! the phrase. `p256` implements it, `rcgen` accepts an external signer
//! through [`rcgen::RemoteKeyPair`], and every other field is pinned
//! here rather than left to a default that a version bump could move.
//!
//! # One implementation, compiled twice
//!
//! This module builds for wasm as well as native, which is the point:
//! the browser runs *this* code rather than a JavaScript reimplementation
//! that would have to agree with it byte for byte forever. That also
//! settles the awkward part — WebCrypto's Ed25519 is not deterministic
//! in Safari — because no browser is asked to sign anything.
//!
//! # Why not have the browser derive it with its own crypto
//!
//! Measured in Chromium rather than reasoned about — same key, same
//! message, signed twice:
//!
//! | WebCrypto algorithm | Same signature twice? |
//! | --- | --- |
//! | `ECDSA` P-256 | **no** — random nonce |
//! | `RSASSA-PKCS1-v1_5` | **yes** — deterministic padding |
//! | `RSA-PSS` | no — random salt |
//! | `Ed25519` | yes in Chromium, **no in Safari** |
//!
//! So native WebCrypto cannot reproduce what this module builds: its
//! P-256 signs with a random nonce, and only RFC 6979 makes the
//! certificate a function of the phrase. Ed25519 is worse than it
//! looks — deterministic by specification, and engine-dependent in
//! fact, which is a guarantee that cannot be relied on.
//!
//! RSA is the one that is genuinely deterministic in a browser, and
//! `RTCPeerConnection.generateCertificate` accepts it, so it is a real
//! DTLS option. It still does not help, because the obstacle is
//! *deriving the key*, not signing with it. An RSA keypair from a
//! phrase means a pinned prime search that two implementations
//! reproduce exactly, it takes hundreds of milliseconds at 2048 bits,
//! and WebCrypto cannot import a key from a seed at all — only generate
//! a random one or import a whole key you already hold. A P-256 secret
//! is a 32-byte scalar, so deriving it is one hash.
//!
//! None of which this design depends on, because the browser runs this
//! code rather than its own. The table is here so the question does not
//! get re-asked from memory.
//!
//! # What is public, and why that is safe
//!
//! The phrase is published, so the private key is derivable by anyone,
//! so anyone can present this certificate. That is not a weakening:
//! `dial` already records that reachability is not permission, because
//! the dialer picks its own ICE credential and there is no secret to
//! withhold. Authorization is per invocation — a signed UCAN, verified
//! before any work — and where iroh rides on top, its TLS authenticates
//! the peer by endpoint key under RFC 7250. An impostor on the port
//! completes DTLS and then fails closed.
//!
//! [`a_certificate_is_not_reproducible_with_a_random_nonce`]: #

use p256::ecdsa::{SigningKey, signature::Signer};
use sha2::{Digest as _, Sha256};

/// The phrase every tonk derives its rendezvous certificate from.
///
/// Versioned, because changing it changes the fingerprint every dialer
/// expects: a new suffix is how this rotates without a flag day, by
/// letting a dialer try both.
pub const RENDEZVOUS: &str = "tonk/rtc/rendezvous/v1";

/// The subject and issuer of the derived certificate.
///
/// Pinned rather than defaulted: `webrtc-rs`' own certificate helper
/// puts a *random* alphanumeric name in every certificate it builds,
/// which alone would make the bytes unreproducible.
const SUBJECT: &str = "tonk-rendezvous";

/// The serial number of the derived certificate. Pinned for the same
/// reason as [`SUBJECT`]: rcgen randomizes it otherwise.
const SERIAL: u64 = 1;

/// An RFC 6979 signer, which is what makes the certificate a function
/// of the phrase rather than of the moment it was built.
struct Deterministic {
    key: SigningKey,
    public: Vec<u8>,
}

impl rcgen::RemoteKeyPair for Deterministic {
    fn public_key(&self) -> &[u8] {
        &self.public
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let signature: p256::ecdsa::Signature = self.key.sign(message);
        Ok(signature.to_der().as_bytes().to_vec())
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

/// The signing key `phrase` derives.
///
/// The hash is the key: a P-256 scalar is 32 bytes and so is a SHA-256
/// digest. The chance of a digest falling outside the curve order is
/// about one in 2^128, and is reported rather than papered over.
pub fn signing_key(phrase: &str) -> Result<SigningKey, KeyError> {
    let seed = Sha256::digest(phrase.as_bytes());
    SigningKey::from_bytes(&seed).map_err(|_| KeyError::NotAScalar)
}

/// The certificate `phrase` derives, DER-encoded.
pub fn certificate_der(phrase: &str) -> Result<Vec<u8>, KeyError> {
    let key = signing_key(phrase)?;
    let public = key
        .verifying_key()
        .to_encoded_point(false)
        .as_bytes()
        .to_vec();

    let pair = rcgen::KeyPair::from_remote(Box::new(Deterministic { key, public }))
        .map_err(|error| KeyError::Certificate(error.to_string()))?;

    let mut params = rcgen::CertificateParams::new(vec![SUBJECT.to_owned()])
        .map_err(|error| KeyError::Certificate(error.to_string()))?;
    params.serial_number = Some(rcgen::SerialNumber::from(SERIAL));
    // `not_before` and `not_after` keep rcgen's defaults, which are
    // 1975 and 4096 — far enough out that expiry is not a thing this
    // has to manage, and fixed rather than relative so they do not move
    // with the clock.

    Ok(params
        .self_signed(&pair)
        .map_err(|error| KeyError::Certificate(error.to_string()))?
        .der()
        .to_vec())
}

/// The fingerprint `phrase` derives, in the form an SDP
/// `a=fingerprint` line takes.
pub fn fingerprint(phrase: &str) -> Result<String, KeyError> {
    let digest = Sha256::digest(certificate_der(phrase)?);
    let octets: Vec<String> = digest.iter().map(|byte| format!("{byte:02X}")).collect();
    Ok(format!("sha-256 {}", octets.join(":")))
}

/// The port `phrase` derives, in the dynamic range.
///
/// Derived for the same reason as the certificate: it is a value both
/// ends must agree on with nothing exchanged, and a phrase is easier to
/// keep honest than two hand-copied constants. The dynamic range is
/// 49152..=65535, so nothing here collides with a registered service.
pub fn port(phrase: &str) -> u16 {
    let digest = Sha256::digest(format!("{phrase}#port").as_bytes());
    let offset = u16::from_be_bytes([digest[0], digest[1]]) % 16384;
    49152 + offset
}

/// Why a phrase yields no certificate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The digest is not a valid P-256 scalar — about one phrase in
    /// 2^128, and worth naming rather than unwrapping.
    #[error("this phrase does not hash to a usable key; pick another")]
    NotAScalar,
    /// The certificate would not build.
    #[error("the certificate could not be built: {0}")]
    Certificate(String),
}

#[cfg(test)]
mod tests;
