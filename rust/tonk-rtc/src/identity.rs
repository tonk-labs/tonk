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
//! # The shared identity, and why a private key is in this repository
//!
//! [`Identity::shared`] is a certificate checked into the source tree,
//! private key and all. That is deliberate and it is not a leak: it
//! exists precisely so that **nothing has to be exchanged** before a
//! browser can dial.
//!
//! A dialer has to write a fingerprint into the description it
//! fabricates, and a browser cannot skip that check the way
//! `webrtc-rs` can. Two of the three things it needs are derivable —
//! the port is fixed, the candidate is loopback — and the fingerprint
//! is not, because it is a hash of a certificate signed by a key only
//! the listener holds. Deriving one per peer was the obvious
//! alternative and does not survive contact: both sides would have to
//! produce byte-identical DER, which needs a deterministic signature,
//! and Safari's Ed25519 does not produce one.
//!
//! So the fingerprint is made *known* instead of derived, by being the
//! same everywhere. What that costs is exactly this: anyone can stand
//! up a listener presenting this certificate, and any process on the
//! machine can open a channel to one. Neither is new — `dial` already
//! records that reachability is not permission, because the ufrag is
//! chosen by the dialer and there is no secret to withhold. What
//! guards the connection is what has always guarded it: every
//! invocation carries a signed UCAN and is verified before any work is
//! done, and where iroh rides on top, its TLS authenticates the peer by
//! endpoint key under RFC 7250. An impostor on the port completes DTLS
//! and then fails closed.
//!
//! Treat this file as a published constant, never as a secret. Rotating
//! it means changing what every browser expects, so
//! [`shared_fingerprint_is_pinned`] fails loudly if it moves.
//!
//! [`shared_fingerprint_is_pinned`]: #

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

/// The certificate every tonk listener presents, and every dialer
/// expects. Public by design — see the module note.
const SHARED_PEM: &str = include_str!("../assets/shared-identity.pem");

/// The fingerprint of [`SHARED_PEM`], in SDP form.
///
/// Duplicated in `tonk-ui`'s `rtc.mjs`, because the page has to write
/// it into a description it fabricates and cannot compute it. The test
/// below pins this against the certificate itself; keeping the page in
/// step is the job of the test that asserts they match.
pub const SHARED_FINGERPRINT: &str = "sha-256 08:EC:77:D3:24:82:FE:18:D7:9D:A2:E3:BC:A1:12:00:07:80:33:9D:E0:00:DE:77:FE:D0:51:73:3A:86:39:95";

impl Identity {
    /// The identity every listener shares, so a dialer needs no address.
    ///
    /// This is what makes "nothing is exchanged" true: a browser that
    /// knows the port knows everything, because the fingerprint is a
    /// constant it already has.
    pub fn shared() -> Result<Self, PeerError> {
        Self::from_pem(SHARED_PEM)
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

    /// Why the certificate is shipped rather than derived from a
    /// well-known phrase, which is the obvious thing to want.
    ///
    /// Deriving the *key* from a string is trivial. The obstacle is
    /// that a fingerprint is the hash of the whole certificate, not of
    /// the key, and a certificate is not reproducible even here: rcgen
    /// signs ECDSA with a random nonce, so the same key and the same
    /// parameters give different bytes every time. Two ends could never
    /// agree, and a browser has no certificate builder at all.
    ///
    /// Reproducing one would mean a hand-built DER and an RFC 6979
    /// signer in both Rust and JavaScript, byte-identical, forever.
    /// Shipping the bytes is smaller and cannot drift. If this test
    /// ever fails, rcgen became deterministic and the cheaper option is
    /// worth revisiting.
    #[test]
    fn a_certificate_is_not_reproducible_from_its_key() {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let params = || {
            let mut params = rcgen::CertificateParams::new(vec!["tonk".to_owned()]).unwrap();
            params.serial_number = Some(rcgen::SerialNumber::from(1u64));
            params
        };

        assert_ne!(
            params().self_signed(&key).unwrap().der(),
            params().self_signed(&key).unwrap().der(),
            "rcgen now signs deterministically: deriving the certificate on both ends, \
             rather than shipping it, is worth reconsidering"
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

    /// The page writes this exact string into the description it
    /// fabricates, so if the certificate moves and this does not, every
    /// dial fails with a DTLS mismatch and nothing says why.
    #[test]
    fn the_shared_fingerprint_is_pinned() {
        assert_eq!(
            Identity::shared().unwrap().fingerprint().to_uppercase(),
            SHARED_FINGERPRINT.to_uppercase(),
            "the shared certificate changed; rtc.mjs must be updated in the same commit"
        );
    }

    /// The browser derives this fingerprint rather than transcribing it,
    /// so what has to stay in step is narrower than it was: the page
    /// must reach the same certificate, and must agree on the port.
    ///
    /// Agreement on the *value* is pinned from the other side, in
    /// `rtc.mjs`'s own tests, because that is where the derivation
    /// lives. This checks the two things a Rust change could break.
    #[test]
    fn the_browser_half_reaches_the_same_certificate() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tonk-ui");

        let index = std::fs::read_to_string(ui.join("index.html")).expect("tonk-ui/index.html");
        assert!(
            index.contains("../tonk-rtc/assets/shared-identity.pem"),
            "index.html no longer copies the shared certificate into the dist, so the page \
             would fetch a 404 and every dial would fail with no explanation"
        );

        let source = std::fs::read_to_string(ui.join("assets/rtc.mjs")).expect("rtc.mjs");
        let port = format!("DEFAULT_PORT = {}", crate::dial::DEFAULT_PORT);
        assert!(
            source.contains(&port),
            "rtc.mjs disagrees about the default port; expected `{port}`"
        );
        assert!(
            !source.contains(SHARED_FINGERPRINT),
            "rtc.mjs transcribes the fingerprint again; it should derive it from the \
             certificate so there is only one copy to get wrong"
        );
    }

    /// Every listener presents the same one: that is the whole point,
    /// and it is what a per-machine identity would quietly undo.
    #[test]
    fn the_shared_identity_is_the_same_everywhere() {
        assert_eq!(
            Identity::shared().unwrap().fingerprint(),
            Identity::shared().unwrap().fingerprint()
        );
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
