//! The certificate a listener is known by.
//!
//! A dialer authenticates this side by comparing the DTLS certificate
//! against a fingerprint it read from the published address, so that
//! fingerprint has to stay put. Two things would otherwise move it:
//!
//! - **Restarting.** A generated-per-run certificate means every
//!   restart silently invalidates every published address.
//! - **Muxing.** Each dial gets its own `RTCPeerConnection`, and each
//!   would mint its own certificate unless handed one. Without a fixed
//!   identity the second dialer checks the published fingerprint
//!   against a different certificate and refuses.
//!
//! So the certificate is generated once and reused. It round-trips
//! through [`Identity::to_pem`] and [`Identity::from_pem`], so a caller
//! that wants a per-machine one can put the PEM wherever it keeps local
//! state.
//!
//! # The rendezvous identity
//!
//! [`Identity::rendezvous`] is derived from a published phrase rather
//! than generated or stored, so **nothing has to be exchanged** before
//! a browser can dial. [`crate::rendezvous`] is where that lives and
//! why it is sound.
//!
//! A per-machine identity is still available and still persisted, for
//! a listener that would rather hand out an address than be reachable
//! by anyone who knows the phrase. It costs a dialer the address it
//! would otherwise not have needed.

use webrtc::peer_connection::certificate::RTCCertificate;

use crate::peer::PeerError;

/// A listener's long-lived certificate.
#[derive(Clone)]
pub struct Identity {
    certificate: RTCCertificate,
}

impl std::fmt::Debug for Identity {
    /// Prints the fingerprint rather than the key material.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Identity")
            .field("fingerprint", &self.fingerprint())
            .finish()
    }
}

/// Wrap base64 the way PEM does, so `pem::parse_many` accepts it.
fn pem_block(tag: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    let body = base64::engine::general_purpose::STANDARD.encode(bytes);
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(64)
        .map(|chunk| std::str::from_utf8(chunk).expect("base64 is ascii"))
        .collect();
    format!(
        "-----BEGIN {tag}-----\n{}\n-----END {tag}-----\n",
        lines.join("\n")
    )
}

impl Identity {
    /// The identity derived from [`rendezvous::RENDEZVOUS`].
    ///
    /// Every listener presents this and every dialer expects it, having
    /// derived the same fingerprint from the same phrase — which is
    /// what makes dialling need nothing published. See
    /// [`crate::rendezvous`] for why a public certificate is sound.
    pub fn rendezvous() -> Result<Self, PeerError> {
        Self::derived(crate::rendezvous::RENDEZVOUS)
    }

    /// The identity `phrase` derives.
    pub fn derived(phrase: &str) -> Result<Self, PeerError> {
        use p256::pkcs8::EncodePrivateKey as _;

        let key = crate::rendezvous::signing_key(phrase)
            .map_err(|error| PeerError::Identity(error.to_string()))?;
        let pkcs8 = p256::SecretKey::from(&key)
            .to_pkcs8_der()
            .map_err(|error| PeerError::Identity(error.to_string()))?;
        let der = crate::rendezvous::certificate_der(phrase)
            .map_err(|error| PeerError::Identity(error.to_string()))?;

        // rcgen dates the certificate to 4096; webrtc-rs keeps its own
        // copy of that in an `EXPIRES` block, so the two must agree or
        // it would expire in this type while staying valid on the wire.
        const YEAR_4096: u64 = 67_090_118_400;

        let pem = format!(
            "{}\n{}\n{}",
            pem_block("EXPIRES", &YEAR_4096.to_le_bytes()),
            pem_block("PRIVATE_KEY", pkcs8.as_bytes()),
            pem_block("CERTIFICATE", &der),
        );

        Self::from_pem(&pem)
    }

    /// Mint a fresh identity.
    ///
    /// ECDSA P-256 rather than Ed25519: it is what every browser's DTLS
    /// stack accepts, and this certificate exists to be checked by a
    /// browser.
    pub fn generate() -> Result<Self, PeerError> {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(|error| PeerError::Identity(error.to_string()))?;
        Ok(Self {
            certificate: RTCCertificate::from_key_pair(key_pair)
                .map_err(|error| PeerError::Identity(error.to_string()))?,
        })
    }

    /// Restore a persisted identity.
    pub fn from_pem(pem: &str) -> Result<Self, PeerError> {
        Ok(Self {
            certificate: RTCCertificate::from_pem(pem)
                .map_err(|error| PeerError::Identity(error.to_string()))?,
        })
    }

    /// Serialize for storage. Contains the private key: treat it the
    /// way the rest of the CLI treats key material.
    pub fn to_pem(&self) -> String {
        self.certificate.serialize_pem()
    }

    /// The fingerprint to publish, in the form an SDP
    /// `a=fingerprint` line takes — `"sha-256 ab:cd:…"`.
    pub fn fingerprint(&self) -> String {
        self.certificate
            .get_fingerprints()
            .first()
            .map(|print| format!("{} {}", print.algorithm, print.value))
            .unwrap_or_default()
    }

    /// The certificate itself, for a peer connection's configuration.
    pub(crate) fn certificate(&self) -> RTCCertificate {
        self.certificate.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decisive cross-check: webrtc-rs, given the derived
    /// certificate, computes the same fingerprint the derivation
    /// predicts. A browser derives that value and writes it into the
    /// description it fabricates, so if these disagreed every dial
    /// would fail the DTLS check with nothing to read.
    #[test]
    fn the_derived_certificate_has_the_derived_fingerprint() {
        assert_eq!(
            Identity::rendezvous().unwrap().fingerprint().to_uppercase(),
            crate::rendezvous::fingerprint(crate::rendezvous::RENDEZVOUS)
                .unwrap()
                .to_uppercase()
        );
    }

    /// Nothing is stored, so every process derives the same one.
    #[test]
    fn the_rendezvous_identity_is_the_same_everywhere() {
        assert_eq!(
            Identity::rendezvous().unwrap().fingerprint(),
            Identity::rendezvous().unwrap().fingerprint()
        );
    }

    #[test]
    fn an_identity_survives_persistence() {
        let original = Identity::generate().unwrap();
        let restored = Identity::from_pem(&original.to_pem()).unwrap();
        assert_eq!(original.fingerprint(), restored.fingerprint());
    }

    /// The published address names this fingerprint, so a restart that
    /// changes it invalidates every address already handed out.
    #[test]
    fn the_fingerprint_is_the_shape_an_sdp_line_takes() {
        let identity = Identity::generate().unwrap();
        let fingerprint = identity.fingerprint();
        let (algorithm, value) = fingerprint.split_once(' ').expect("algorithm and value");
        assert_eq!(algorithm, "sha-256");
        assert_eq!(value.split(':').count(), 32, "sha-256 is 32 octets");
    }

    #[test]
    fn two_identities_differ() {
        assert_ne!(
            Identity::generate().unwrap().fingerprint(),
            Identity::generate().unwrap().fingerprint()
        );
    }

    /// Key material must not reach a log through a stray `{:?}`.
    #[test]
    fn debug_does_not_print_key_material() {
        let identity = Identity::generate().unwrap();
        let rendered = format!("{identity:?}");
        assert!(!rendered.contains("PRIVATE KEY"), "{rendered}");
        assert!(rendered.contains("sha-256"));
    }
}
