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
//! - **A reachable port is dialable.** Nothing at this layer gates who
//!   may open a channel: the dialer chooses its own ufrag, so there is
//!   no shared secret to withhold. That is deliberate — authorization
//!   is per invocation, where every request carries a signed UCAN and
//!   is verified before any work is done, so a connected peer with no
//!   capability is served nothing. Reachability is not permission.
//!   What an open port does cost is resources: anyone can make this
//!   side perform DTLS handshakes and hold connections, so a cap on
//!   concurrent dials belongs here before it faces a hostile network.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::dtls_transport::dtls_role::DTLSRole;
use webrtc::ice::udp_mux::{UDPMuxDefault, UDPMuxParams};
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::identity::Identity;
use crate::mux::Watching;
use crate::peer::{CHANNEL_LABEL, PeerError, Session};

/// How many dials may be in flight or open at once.
///
/// Reachability is not permission here — authorization happens per
/// invocation, above this layer — so anyone who can reach the port can
/// make this side build a peer connection and run a DTLS handshake.
/// This bounds what that costs. Generous next to the handful of tabs a
/// person actually has open, small next to what an unbounded loop would
/// consume.
const MAX_CONCURRENT_DIALS: usize = 32;

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
    /// `"sha-256 ab:cd:…"`. This is what authenticates this side, and
    /// the reason the certificate is persisted rather than minted per
    /// run: a fingerprint that moves invalidates every address already
    /// handed out.
    pub fingerprint: String,
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
///
/// Holds the shared socket and every peer connection built for a dial;
/// dropping it tears all of them down.
pub struct Listener {
    address: Address,
    incoming: AsyncMutex<mpsc::UnboundedReceiver<Session>>,
    /// Kept alive for the listener's life. The accept loop owns the
    /// per-dial connections; this is the handle that stops it.
    _accepting: tokio::task::JoinHandle<()>,
}

/// The offer this side answers for a given dial.
///
/// Entirely fabricated: the dialer has sent nothing but STUN. Its ICE
/// credentials are the dialer's own ufrag, used for both fields, which
/// is what makes the `USERNAME` this side expects match what the dialer
/// sends. Its fingerprint is a placeholder this side never checks — see
/// the module note on one-way authentication.
fn fabricated_offer(ufrag: &str) -> String {
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
         a=ice-ufrag:{ufrag}\r\n\
         a=ice-pwd:{ufrag}\r\n"
    )
}

/// The addresses this port can be reached on.
///
/// Loopback always, for the same-machine case. Plus whichever local
/// address the routing table would use to reach the outside world,
/// which covers the LAN — found by "connecting" a throwaway UDP socket,
/// which sends nothing and merely asks the kernel to pick a route.
fn reachable_on(port: u16) -> Vec<Candidate> {
    let mut candidates = vec![Candidate {
        host: "127.0.0.1".to_owned(),
        port,
    }];
    if let Ok(probe) = std::net::UdpSocket::bind("0.0.0.0:0")
        && probe.connect("198.51.100.1:9").is_ok()
        && let Ok(local) = probe.local_addr()
        && !local.ip().is_loopback()
    {
        candidates.push(Candidate {
            host: local.ip().to_string(),
            port,
        });
    }
    candidates
}

/// Build the peer connection that answers one dial.
async fn answer_dial(
    ufrag: &str,
    identity: &Identity,
    mux: Arc<UDPMuxDefault>,
    sessions: mpsc::UnboundedSender<Session>,
) -> Result<Arc<RTCPeerConnection>, PeerError> {
    let mut settings = SettingEngine::default();
    // The dialer chose this ufrag and used it for both of its own ICE
    // fields, so matching it here is what makes the USERNAME check pass
    // without anything having been exchanged.
    settings.set_ice_credentials(ufrag.to_owned(), ufrag.to_owned());
    // Every dial rides the one shared socket; this is what routes its
    // packets to this connection rather than another dial's.
    settings.set_udp_network(UDPNetwork::Muxed(mux));
    // A dialer's certificate is minted per page load, so it cannot be
    // known in advance and cannot be checked.
    settings.disable_certificate_fingerprint_verification(true);
    // Answer with `a=setup:active` every time, so a dialer synthesising
    // this side's description can hard-code the role.
    settings.set_answering_dtls_role(DTLSRole::Client)?;
    settings.set_ice_multicast_dns_mode(webrtc::ice::mdns::MulticastDnsMode::QueryOnly);

    let api = APIBuilder::new()
        .with_media_engine(MediaEngine::default())
        .with_setting_engine(settings)
        .build();
    let connection = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            // The published fingerprint names THIS certificate, so every
            // dial must present it. Left to itself each connection would
            // mint its own and every dialer after the first would refuse
            // the one it was promised.
            certificates: vec![identity.certificate()],
            ..Default::default()
        })
        .await?,
    );

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
        .set_remote_description(RTCSessionDescription::offer(fabricated_offer(ufrag))?)
        .await?;
    let answer = connection.create_answer(None).await?;
    connection.set_local_description(answer).await?;
    Ok(connection)
}

/// Start listening, and produce the address a peer can dial.
///
/// The address stays valid for as long as this listener lives, and
/// across restarts too when the same [`Identity`] is restored and the
/// port is stable. Dials may arrive concurrently and repeatedly; each
/// gets its own peer connection over the one shared port.
pub async fn listen(identity: Identity) -> Result<Listener, PeerError> {
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|error| PeerError::Identity(error.to_string()))?;
    let port = socket
        .local_addr()
        .map_err(|error| PeerError::Identity(error.to_string()))?
        .port();

    let (watching, mut dials) = Watching::wrap(Arc::new(socket));
    let mux = UDPMuxDefault::new(UDPMuxParams::new(watching));

    let (sessions, incoming) = mpsc::unbounded_channel();
    let accepting = {
        let identity = identity.clone();
        let mux = mux.clone();
        tokio::spawn(async move {
            // Every connection built for a dial is kept here; dropping
            // one would tear down a live channel.
            let mut connections: Vec<Arc<RTCPeerConnection>> = Vec::new();
            while let Some(ufrag) = dials.recv().await {
                // Nothing authenticates a dial at this layer, so a
                // stranger can announce an arbitrary ufrag and each one
                // would otherwise cost a peer connection and a DTLS
                // handshake. Shed closed connections first, then refuse
                // rather than grow without bound.
                connections.retain(|connection| {
                    connection.connection_state() != RTCPeerConnectionState::Closed
                });
                if connections.len() >= MAX_CONCURRENT_DIALS {
                    tracing::warn!(%ufrag, "refusing a dial: too many already open");
                    continue;
                }

                match answer_dial(&ufrag, &identity, mux.clone(), sessions.clone()).await {
                    Ok(connection) => connections.push(connection),
                    // One dial failing is not the listener failing —
                    // anyone can send a packet to an open port.
                    Err(error) => {
                        tracing::debug!(%ufrag, %error, "could not answer a dial");
                    }
                }
            }
        })
    };

    Ok(Listener {
        address: Address {
            candidates: reachable_on(port),
            fingerprint: identity.fingerprint(),
        },
        incoming: AsyncMutex::new(incoming),
        _accepting: accepting,
    })
}

impl Listener {
    /// The record to publish. Everything a dialer needs.
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// Wait for the next dialer to open a channel.
    ///
    /// `None` once the listener is torn down. Called repeatedly: a
    /// listener serves many dials over its life.
    pub async fn accept(&self) -> Option<Session> {
        self.incoming.lock().await.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_published_address_carries_everything_a_dialer_needs() {
        let listener = listen(Identity::generate().unwrap()).await.unwrap();
        let address = listener.address();
        assert!(
            address.fingerprint.starts_with("sha-256 "),
            "unexpected fingerprint: {}",
            address.fingerprint
        );
        assert!(
            !address.candidates.is_empty(),
            "nothing to dial: no addresses were published"
        );
    }

    /// The same-machine case is the one this exists for first, and it
    /// silently has nothing to dial without a loopback address.
    #[tokio::test]
    async fn a_loopback_address_is_published() {
        let listener = listen(Identity::generate().unwrap()).await.unwrap();
        assert!(
            listener
                .address()
                .candidates
                .iter()
                .any(|candidate| candidate.host == "127.0.0.1"),
            "no loopback address in {:?}",
            listener.address().candidates
        );
    }

    /// The published fingerprint is the identity's, not something the
    /// connection minted — which is what lets an address outlive both a
    /// restart and the first dial.
    #[tokio::test]
    async fn the_address_names_the_identity_it_was_given() {
        let identity = Identity::generate().unwrap();
        let listener = listen(identity.clone()).await.unwrap();
        assert_eq!(listener.address().fingerprint, identity.fingerprint());
    }

    /// Restoring the same identity republishes the same fingerprint, so
    /// an address handed out before a restart still authenticates this
    /// side afterwards. Only the port moves.
    #[tokio::test]
    async fn a_restored_identity_republishes_the_same_fingerprint() {
        let identity = Identity::generate().unwrap();
        let before = listen(identity.clone()).await.unwrap();
        let restored = Identity::from_pem(&identity.to_pem()).unwrap();
        let after = listen(restored).await.unwrap();
        assert_eq!(
            before.address().fingerprint,
            after.address().fingerprint,
            "a restart would invalidate every address already published"
        );
    }

    /// Every dial shares one port, so two listeners must not.
    #[tokio::test]
    async fn listeners_do_not_share_a_port() {
        let first = listen(Identity::generate().unwrap()).await.unwrap();
        let second = listen(Identity::generate().unwrap()).await.unwrap();
        assert_ne!(
            first.address().candidates[0].port,
            second.address().candidates[0].port
        );
    }

    #[test]
    fn an_address_survives_the_round_trip() {
        let address = Address {
            candidates: vec![Candidate {
                host: "127.0.0.1".into(),
                port: 41794,
            }],
            fingerprint: "sha-256 ab:cd".into(),
        };
        assert_eq!(Address::decode(&address.encode()).unwrap(), address);
    }

    #[test]
    fn loopback_is_always_reachable_and_carries_the_bound_port() {
        let candidates = reachable_on(4242);
        assert!(candidates.iter().any(|c| c.host == "127.0.0.1"));
        assert!(candidates.iter().all(|c| c.port == 4242));
    }

    /// The dialer chose this ufrag; using it for BOTH fields is what
    /// makes the USERNAME check pass with nothing exchanged.
    #[test]
    fn the_fabricated_offer_answers_as_the_dialer_addressed_us() {
        let offer = fabricated_offer("their-random-ufrag");
        assert!(offer.contains("a=ice-ufrag:their-random-ufrag\r\n"));
        assert!(offer.contains("a=ice-pwd:their-random-ufrag\r\n"));
        assert!(offer.contains("m=application"));
    }
}
