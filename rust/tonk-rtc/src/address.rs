//! The versioned WebRTC route record shared by native and browser peers.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

/// The route record version this build writes.
pub const VERSION: u8 = 1;

fn legacy_version() -> u8 {
    VERSION
}

/// One IP literal and UDP port a dialer can send to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// An IP literal, never an SDP fragment or hostname.
    pub host: String,
    /// The bound UDP port.
    pub port: u16,
}

/// A route, not an identity or a grant of authority.
///
/// The DTLS fingerprint authenticates only the outer carrier. The expected
/// iroh endpoint key authenticates the peer, and Dialog verifies authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Wire version. Unversioned spike records are read as version 1.
    #[serde(default = "legacy_version")]
    pub version: u8,
    /// At most sixteen IP/port candidates.
    pub candidates: Vec<Candidate>,
    /// SHA-256 certificate fingerprint in SDP notation.
    pub fingerprint: String,
}

/// Why a route cannot safely be used to construct SDP.
#[derive(Debug, thiserror::Error)]
#[error("invalid WebRTC route: {0}")]
pub struct AddressError(pub String);

impl Address {
    /// Encode the record as unpadded base64url JSON.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("an address serializes"))
    }

    /// Decode and validate a route before passing it to a carrier.
    pub fn decode(encoded: &str) -> Result<Self, AddressError> {
        if encoded.len() > 16_384 {
            return Err(AddressError("record exceeds 16 KiB".into()));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded.trim())
            .map_err(|e| AddressError(e.to_string()))?;
        let address: Self =
            serde_json::from_slice(&bytes).map_err(|e| AddressError(e.to_string()))?;
        address.validate()?;
        Ok(address)
    }

    /// Reject unsupported versions, SDP injection and unbounded candidate lists.
    pub fn validate(&self) -> Result<(), AddressError> {
        if self.version != VERSION {
            return Err(AddressError(format!(
                "version {}; expected {VERSION}",
                self.version
            )));
        }
        if self.candidates.is_empty() || self.candidates.len() > 16 {
            return Err(AddressError("expected 1–16 candidates".into()));
        }
        for candidate in &self.candidates {
            if candidate.port == 0 || candidate.host.parse::<std::net::IpAddr>().is_err() {
                return Err(AddressError(
                    "candidate requires an IP literal and nonzero port".into(),
                ));
            }
        }
        let valid = self
            .fingerprint
            .strip_prefix("sha-256 ")
            .is_some_and(|value| {
                let parts: Vec<_> = value.split(':').collect();
                parts.len() == 32
                    && parts
                        .iter()
                        .all(|part| part.len() == 2 && part.bytes().all(|c| c.is_ascii_hexdigit()))
            });
        if !valid {
            return Err(AddressError(
                "expected a SHA-256 certificate fingerprint".into(),
            ));
        }
        Ok(())
    }

    /// Whether every route stays on this machine.
    pub fn is_loopback(&self) -> bool {
        self.candidates.iter().all(|candidate| {
            candidate
                .host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> Address {
        Address {
            version: VERSION,
            candidates: vec![Candidate {
                host: "127.0.0.1".into(),
                port: 55555,
            }],
            fingerprint: format!("sha-256 {}", ["AB"; 32].join(":")),
        }
    }

    #[test]
    fn versioned_routes_round_trip_without_changing_port_or_fingerprint() {
        let route = address();
        assert_eq!(Address::decode(&route.encode()).unwrap(), route);
        assert!(route.is_loopback());
    }

    #[test]
    fn malformed_routes_fail_before_the_sdp_boundary() {
        let mut route = address();
        route.version = 2;
        assert!(Address::decode(&route.encode()).is_err());
        route = address();
        route.candidates[0].host = "127.0.0.1\r\na=ice-pwd:injected".into();
        assert!(route.validate().is_err());
        route = address();
        route.candidates[0].port = 0;
        assert!(route.validate().is_err());
        route = address();
        route.fingerprint.push_str("\r\na=setup:passive");
        assert!(route.validate().is_err());
        route = address();
        route.candidates = vec![route.candidates[0].clone(); 17];
        assert!(route.validate().is_err());
    }
}
