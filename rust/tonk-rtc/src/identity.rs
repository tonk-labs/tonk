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
//! So the certificate is generated once, persisted as PEM by the
//! caller, and reused. This module does not choose where that PEM
//! lives — it round-trips through [`Identity::to_pem`] and
//! [`Identity::from_pem`] so the CLI can put it wherever it keeps
//! local state.

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

impl Identity {
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
