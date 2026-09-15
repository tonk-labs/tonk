//! Direct dial: the CLI publishes an address, a peer dials it, and
//! nothing travels back.
//!
//! An SDP answer carries three things a dialer needs — a DTLS
//! fingerprint, ICE credentials, and candidates. All three can be known
//! in advance, so a dialer can build this side's half of the handshake
//! itself and never wait for a reply.
//!
//! The trick that makes it work in a browser: browsers refuse to let you
//! invent your own *local* description from nothing, but they DO accept
//! a `createOffer` result whose `a=ice-ufrag` and `a=ice-pwd` have been
//! rewritten, and they use those on the wire (measured in Chromium; it
//! is what `libp2p-webrtc-websys` ships). So the dialer sets BOTH sides'
//! credentials to one shared string, and there is nothing left to
//! exchange.
//!
//! # Consequences worth being explicit about
//!
//! - **The credential is a bearer secret.** Whoever holds the address
//!   record can open a channel. That is the same property an invite URL
//!   has, but it means the record must not be published more widely than
//!   the right to connect.
//!
//! - **DTLS is one-way authenticated.** The dialer verifies this side
//!   against the published fingerprint; this side cannot verify the
//!   dialer, because a dialer's certificate is generated per page load
//!   and cannot be known in advance. Fingerprint verification is
//!   therefore disabled here, and something on top of the channel has to
//!   establish who the peer is. libp2p uses Noise for this; tonk has
//!   DIDs and UCAN delegations already.
//!
//! - **One credential means one dialer at a time.** Concurrent dialers
//!   would be indistinguishable on the wire, since ICE separates peers
//!   by ufrag. Supporting several at once needs a per-dial random ufrag
//!   and a UDP mux that reads it out of the first STUN packet — the
//!   mechanism `libp2p-webrtc` uses, and the next step from here.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::dtls_transport::dtls_role::DTLSRole;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::peer::{CHANNEL_LABEL, PeerError, Session};

/// How long to wait for ICE gathering before publishing what we have.
const GATHER_DEADLINE: Duration = Duration::from_secs(3);

/// Bytes of entropy behind the dial credential.
///
/// Encoded base64url this yields 32 characters, comfortably over the
/// 22-character minimum RFC 5245 puts on an ICE password — the same
/// string serves as both ufrag and password.
const CREDENTIAL_BYTES: usize = 24;

/// One address a dialer can send to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// The IP literal, as it will appear in the dialer's SDP.
    pub host: String,
    /// The UDP port this listener is bound to.
    pub port: u16,
}

/// Everything a dialer needs, and nothing this side has to be told.
///
/// This is the record intended to live as a cardinality-one fact in a
/// replicated space: a peer reads it whenever sync happens to deliver
/// it, and dials later with no further coordination. Discovery
/// tolerates arbitrary latency; the handshake involves no sync at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Where to send. Several, because a host may be multi-homed; a
    /// dialer puts them all in the description it synthesises and lets
    /// ICE pick.
    pub candidates: Vec<Candidate>,
    /// The DTLS fingerprint, as an SDP `a=fingerprint` value —
    /// `"sha-256 AB:CD:…"`. This is what authenticates this side.
    pub fingerprint: String,
    /// The shared ICE credential, used as BOTH `ice-ufrag` and
    /// `ice-pwd` on both sides. A bearer secret; see the module note.
    pub credential: String,
}

impl Address {
    /// Encode for a URL fragment or a fact value.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("an address always serializes");
        URL_SAFE_NO_PAD.encode(json)
    }

    /// Decode a record produced by [`Self::encode`].
    pub fn decode(encoded: &str) -> Result<Self, crate::signal::DecodeError> {
        let bytes = URL_SAFE_NO_PAD.decode(encoded.trim())?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// A listening peer, waiting to be dialed.
pub struct Listener {
    /// Held so the connection outlives the listener; dropping it tears
    /// the peer down.
    _connection: Arc<RTCPeerConnection>,
    address: Address,
    incoming: AsyncMutex<mpsc::UnboundedReceiver<Session>>,
}

/// A fresh bearer credential from the OS entropy source.
fn credential() -> Result<String, PeerError> {
    let mut bytes = [0u8; CREDENTIAL_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| PeerError::Entropy(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The offer this side answers.
///
/// Entirely fabricated: no dialer has spoken yet. Its ICE credentials
/// are the shared ones (so the `USERNAME` this side expects is
/// `credential:credential`, exactly what a dialer using the published
/// record will send), and its fingerprint is a placeholder this side
/// never checks — see the module note on one-way authentication.
fn fabricated_offer(credential: &str) -> String {
    let placeholder = ["00"; 32].join(":");
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 0.0.0.0\r\n\
         s=-\r\n\
         t=0 0\r\n\
         a=fingerprint:sha-256 {placeholder}\r\n\
         a=group:BUNDLE 0\r\n\
         m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
         c=IN IP4 0.0.0.0\r\n\
         a=setup:actpass\r\n\
         a=mid:0\r\n\
         a=sendrecv\r\n\
         a=sctp-port:5000\r\n\
         a=max-message-size:65536\r\n\
         a=ice-ufrag:{credential}\r\n\
         a=ice-pwd:{credential}\r\n"
    )
}

/// Pull the `a=fingerprint` value out of a description.
fn fingerprint_of(sdp: &str) -> Option<String> {
    sdp.lines()
        .find_map(|line| line.trim().strip_prefix("a=fingerprint:"))
        .map(str::to_owned)
}

/// Pull every host candidate's address out of a description.
///
/// `a=candidate:<foundation> <component> <transport> <priority> <ip> <port> typ host`
/// — only component 1 is taken, because components 1 and 2 (RTP and
/// RTCP) share a port here and would otherwise be listed twice.
fn candidates_of(sdp: &str) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    for line in sdp.lines() {
        let Some(rest) = line.trim().strip_prefix("a=candidate:") else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 8 || fields[1] != "1" || fields[7] != "host" {
            continue;
        }
        let Ok(port) = fields[5].parse::<u16>() else {
            continue;
        };
        let candidate = Candidate {
            host: fields[4].to_owned(),
            port,
        };
        if !found.contains(&candidate) {
            found.push(candidate);
        }
    }
    found
}

/// Start listening, and produce the address a peer can dial.
pub async fn listen() -> Result<Listener, PeerError> {
    let credential = credential()?;

    let mut settings = SettingEngine::default();
    // Both sides use one string for both fields. This is what removes
    // the round trip: a dialer rewrites its own credentials to match,
    // so the `USERNAME` on the wire is `credential:credential` and
    // neither side has to learn anything from the other.
    settings.set_ice_credentials(credential.clone(), credential.clone());
    // A dialer's certificate is generated per page load, so it cannot be
    // known in advance and cannot be checked. See the module note.
    settings.disable_certificate_fingerprint_verification(true);
    // Answer with `a=setup:active` every time, so a dialer synthesising
    // this side's description can hard-code the role instead of being
    // told it. Without this the role follows the resolved ICE role and
    // the dialer would have to guess.
    settings.set_answering_dtls_role(DTLSRole::Client)?;
    // Accept a browser's mDNS candidates; see `peer::settings`.
    settings.set_ice_multicast_dns_mode(webrtc::ice::mdns::MulticastDnsMode::QueryOnly);
    // Without this there is no `127.0.0.1` candidate at all, and the
    // same-machine case — the one this exists for first — has nothing to
    // dial.
    settings.set_include_loopback_candidate(true);

    let api = APIBuilder::new()
        .with_media_engine(MediaEngine::default())
        .with_setting_engine(settings)
        .build();
    let connection = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let (sessions, incoming) = mpsc::unbounded_channel();
    let owner = connection.clone();
    connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let sessions = sessions.clone();
        let owner = owner.clone();
        Box::pin(async move {
            if channel.label() != CHANNEL_LABEL {
                return;
            }
            let _ = sessions.send(Session::attach(owner, channel));
        })
    }));

    connection
        .set_remote_description(RTCSessionDescription::offer(fabricated_offer(&credential))?)
        .await?;
    let answer = connection.create_answer(None).await?;
    let mut gathered = connection.gathering_complete_promise().await;
    connection.set_local_description(answer).await?;
    let _ = tokio::time::timeout(GATHER_DEADLINE, gathered.recv()).await;

    let local = connection
        .local_description()
        .await
        .ok_or(PeerError::NoLocalDescription)?;

    Ok(Listener {
        address: Address {
            candidates: candidates_of(&local.sdp),
            fingerprint: fingerprint_of(&local.sdp).ok_or(PeerError::NoLocalDescription)?,
            credential,
        },
        _connection: connection,
        incoming: AsyncMutex::new(incoming),
    })
}

impl Listener {
    /// The record to publish. Everything a dialer needs.
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Wait for a dialer to open the channel.
    ///
    /// `None` once the listener is torn down.
    pub async fn accept(&self) -> Option<Session> {
        self.incoming.lock().await.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_published_address_carries_everything_a_dialer_needs() {
        let listener = listen().await.unwrap();
        let address = listener.address();
        assert!(
            address.fingerprint.starts_with("sha-256 "),
            "unexpected fingerprint: {}",
            address.fingerprint
        );
        assert!(
            address.credential.len() >= 22,
            "an ICE password must be at least 22 characters (RFC 5245)"
        );
        assert!(
            !address.candidates.is_empty(),
            "nothing to dial: no host candidates were gathered"
        );
    }

    /// The same-machine case is the one this exists for first, and it
    /// silently has nothing to dial without `set_include_loopback_candidate`.
    #[tokio::test]
    async fn a_loopback_candidate_is_published() {
        let listener = listen().await.unwrap();
        assert!(
            listener
                .address()
                .candidates
                .iter()
                .any(|candidate| candidate.host == "127.0.0.1"),
            "no loopback candidate in {:?}",
            listener.address().candidates
        );
    }

    /// Two listeners must not share a credential; it is a bearer secret.
    #[tokio::test]
    async fn credentials_are_not_reused() {
        let first = listen().await.unwrap();
        let second = listen().await.unwrap();
        assert_ne!(first.address().credential, second.address().credential);
    }

    #[test]
    fn an_address_survives_the_round_trip() {
        let address = Address {
            candidates: vec![Candidate {
                host: "127.0.0.1".into(),
                port: 41794,
            }],
            fingerprint: "sha-256 AB:CD".into(),
            credential: "shared-credential-value".into(),
        };
        assert_eq!(Address::decode(&address.encode()).unwrap(), address);
    }

    #[test]
    fn rtcp_components_do_not_duplicate_a_candidate() {
        // webrtc emits component 1 and component 2 on the same port.
        let sdp = "a=candidate:1 1 udp 2130706431 127.0.0.1 41794 typ host\r\n\
                   a=candidate:1 2 udp 2130706431 127.0.0.1 41794 typ host\r\n";
        assert_eq!(
            candidates_of(sdp),
            vec![Candidate {
                host: "127.0.0.1".into(),
                port: 41794
            }]
        );
    }

    #[test]
    fn non_host_candidates_are_not_published() {
        let sdp = "a=candidate:1 1 udp 1 203.0.113.5 3478 typ srflx raddr 10.0.0.1 rport 4000\r\n";
        assert!(candidates_of(sdp).is_empty());
    }
}
