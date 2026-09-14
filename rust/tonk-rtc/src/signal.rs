//! The session descriptions the two peers exchange, and how they travel.
//!
//! Signalling is the part WebRTC deliberately leaves to the application:
//! before an [`RTCPeerConnection`] can reach its peer, each side has to
//! hand the other a session description (SDP) out of band. This module
//! defines that payload and its wire encoding; *how* it gets carried is
//! the signalling channel's problem, and the PoC's channel lives in
//! [`crate::loopback`].
//!
//! The encoding is base64url of a JSON envelope. Two reasons for the
//! envelope rather than the bare SDP: the `version` lets a future peer
//! recognise a payload it cannot parse instead of feeding garbage to
//! `setRemoteDescription`, and there are already obvious fields to add
//! (an ICE restart nonce, the space this channel is for) that would
//! otherwise need a second, incompatible format.
//!
//! [`RTCPeerConnection`]: https://developer.mozilla.org/en-US/docs/Web/API/RTCPeerConnection

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

/// The envelope version this build writes and accepts.
const VERSION: u8 = 1;

/// Which half of the negotiation a description is.
///
/// Carried explicitly so a peer handed the wrong one fails with a clear
/// message rather than inside the ICE machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The description the initiating peer produced.
    Offer,
    /// The description the answering peer produced in reply.
    Answer,
}

/// A session description on its way between peers.
///
/// The SDP travels complete, with ICE candidates already gathered into
/// it — see [`crate::peer`] for why the PoC does not trickle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Description {
    /// Envelope version. Rejected on decode when it is not [`VERSION`].
    pub version: u8,
    /// Whether this is the offer or the answer.
    pub role: Role,
    /// The SDP itself, verbatim.
    pub sdp: String,
}

/// Why a payload could not be read as a [`Description`].
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The text was not valid base64url.
    #[error("the signalling payload is not valid base64url: {0}")]
    Base64(#[from] base64::DecodeError),
    /// The decoded bytes were not the JSON envelope.
    #[error("the signalling payload is not a session description: {0}")]
    Json(#[from] serde_json::Error),
    /// The envelope came from an incompatible peer.
    #[error("the signalling payload is version {found}; this build speaks version {VERSION}")]
    Version {
        /// The version the payload declared.
        found: u8,
    },
    /// The payload was the other half of the negotiation.
    #[error("expected {expected:?} but the payload carries {found:?}")]
    Role {
        /// What the caller asked for.
        expected: Role,
        /// What arrived.
        found: Role,
    },
}

impl Description {
    /// Wrap an offer SDP.
    pub fn offer(sdp: impl Into<String>) -> Self {
        Self {
            version: VERSION,
            role: Role::Offer,
            sdp: sdp.into(),
        }
    }

    /// Wrap an answer SDP.
    pub fn answer(sdp: impl Into<String>) -> Self {
        Self {
            version: VERSION,
            role: Role::Answer,
            sdp: sdp.into(),
        }
    }

    /// Encode for transport: base64url of the JSON envelope.
    ///
    /// URL-safe and unpadded because every carrier in sight puts this in
    /// a URL fragment or a form field.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("a description always serializes");
        URL_SAFE_NO_PAD.encode(json)
    }

    /// Decode a payload and check it is the half the caller expected.
    pub fn decode(encoded: &str, expected: Role) -> Result<Self, DecodeError> {
        let bytes = URL_SAFE_NO_PAD.decode(encoded.trim())?;
        let description: Self = serde_json::from_slice(&bytes)?;
        if description.version != VERSION {
            return Err(DecodeError::Version {
                found: description.version,
            });
        }
        if description.role != expected {
            return Err(DecodeError::Role {
                expected,
                found: description.role,
            });
        }
        Ok(description)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_survives_the_round_trip() {
        let original = Description::offer("v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\n");
        let decoded = Description::decode(&original.encode(), Role::Offer).unwrap();
        assert_eq!(decoded.sdp, original.sdp);
        assert_eq!(decoded.role, Role::Offer);
    }

    #[test]
    fn the_encoding_is_url_safe_and_unpadded() {
        // A fragment carrier must not have to escape anything, and `=`
        // in a fragment is a parameter separator for `URLSearchParams`.
        let encoded = Description::offer("a".repeat(100)).encode();
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "unexpected characters in {encoded}"
        );
    }

    #[test]
    fn the_wrong_half_is_rejected_by_role() {
        let offer = Description::offer("v=0\r\n").encode();
        assert!(matches!(
            Description::decode(&offer, Role::Answer),
            Err(DecodeError::Role {
                expected: Role::Answer,
                found: Role::Offer
            })
        ));
    }

    #[test]
    fn an_incompatible_version_is_named_rather_than_parsed() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"version":99,"role":"offer","sdp":"v=0"}"#);
        assert!(matches!(
            Description::decode(&payload, Role::Offer),
            Err(DecodeError::Version { found: 99 })
        ));
    }

    #[test]
    fn garbage_is_an_error_rather_than_a_panic() {
        assert!(matches!(
            Description::decode("!!!not base64!!!", Role::Offer),
            Err(DecodeError::Base64(_))
        ));
        assert!(matches!(
            Description::decode(&URL_SAFE_NO_PAD.encode(b"not json"), Role::Offer),
            Err(DecodeError::Json(_))
        ));
    }
}
