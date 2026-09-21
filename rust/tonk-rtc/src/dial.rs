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
//! - **The route is not a bearer grant.** ICE credentials are chosen by
//!   the dialer; knowing the port is enough to attempt a connection.
//!   Neither the route nor the public rendezvous certificate grants
//!   repository access. Local inventory disclosure is a separate policy.
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

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, mpsc};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::dtls_transport::dtls_role::DTLSRole;
use webrtc::ice::udp_mux::{UDPMux, UDPMuxDefault, UDPMuxParams};
use webrtc::ice::udp_network::UDPNetwork;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::identity::Identity;
use crate::mux::Watching;
use crate::peer::{CHANNEL_LABEL, DATAGRAM_LABEL, PeerError, Session};

/// The port a listener binds unless told otherwise.
///
/// The point of a default is that a dialer needs no coordination to
/// find it — the same bargain a local HTTP daemon makes. Chosen from
/// the dynamic/private range (49152–65535), where no registered service
/// will collide with it.
pub const DEFAULT_PORT: u16 = 51247;

/// A default outside the dynamic range risks colliding with a
/// registered service, and a dialer assumes this value — so moving it
/// wrongly would silently break every cached address.
///
/// Superseded for the rendezvous path by
/// [`rendezvous::port`](crate::rendezvous::port), which derives the
/// same kind of value from the published phrase so neither end has to
/// carry a constant. This one remains for a listener given an explicit
/// port.
const _: () = assert!(DEFAULT_PORT >= 49152);

/// How many dials may be in flight or open at once.
///
/// Reachability is not permission here — authorization happens per
/// invocation, above this layer — so anyone who can reach the port can
/// make this side build a peer connection and run a DTLS handshake.
/// This bounds what that costs. Generous next to the handful of tabs a
/// person actually has open, small next to what an unbounded loop would
/// consume.
const MAX_CONCURRENT_DIALS: usize = 32;

pub use crate::address::{Address, Candidate};

/// A dialer that opened a datagram channel.
///
/// Carries the peer connection so it stays alive: dropping it tears the
/// channel down, and the channel is the route.
pub struct Dialer {
    /// The ICE credential this dial announced, which is the only handle
    /// this side has for a browser with no address of its own.
    pub ufrag: String,
    /// The unreliable, unordered channel the datagrams ride.
    pub channel: Arc<RTCDataChannel>,
    _connection: Arc<RTCPeerConnection>,
}

/// A listening peer, waiting to be dialed.
///
/// Holds the shared socket and every peer connection built for a dial;
/// dropping it tears all of them down.
pub struct Listener {
    address: Address,
    datagrams: AsyncMutex<mpsc::Receiver<Dialer>>,
    incoming: AsyncMutex<mpsc::Receiver<Session>>,
    /// Kept alive for the listener's life. The accept loop owns the
    /// per-dial connections; this is the handle that stops it.
    _accepting: tokio::task::JoinHandle<()>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

fn expired(state: RTCPeerConnectionState, idle: Duration) -> bool {
    matches!(
        state,
        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed
    ) || (state != RTCPeerConnectionState::Connected && idle >= Duration::from_secs(20))
}

/// The offer this side answers for a given dial.
///
/// Entirely fabricated: the dialer has sent nothing but STUN. Its ICE
/// credentials are the dialer's own ufrag, used for both fields, which
/// is what makes the `USERNAME` this side expects match what the dialer
/// sends. Its fingerprint is a placeholder this side never checks — see
/// the module note on one-way authentication.
///
/// It carries a candidate, and must. An offer with none leaves this
/// side's ICE agent with an empty remote candidate list, and an agent
/// with no remote discards everything that arrives:
///
/// ```text
/// [controlled]: Discarded message, not a valid remote candidate
/// [controlled]: discard success message from (127.0.0.1:59646), no such remote
/// ```
///
/// A browser that publishes a real host candidate survives that anyway,
/// because the address it sends from gets promoted to peer-reflexive.
/// One that anonymises its candidates as `<uuid>.local` — which Chrome
/// does by default, and which nothing here can resolve because the
/// dialer's SDP never crosses — does not, so ICE reports `connected`
/// while DTLS never starts and the channel hangs on `connecting`
/// forever. Naming the loopback address the dialer will arrive from
/// gives the agent something to match against, and costs nothing when
/// the dialer turns out to be elsewhere: an unmatched candidate is
/// simply never used.
fn fabricated_offer(ufrag: &str, from: SocketAddr) -> String {
    let placeholder = ["00"; 32].join(":");
    let host = from.ip();
    let port = from.port();
    let family = if from.is_ipv4() { "IP4" } else { "IP6" };
    format!(
        "v=0\r\n\
         o=- 0 0 IN {family} {host}\r\n\
         s=-\r\n\
         t=0 0\r\n\
         a=fingerprint:sha-256 {placeholder}\r\n\
         a=group:BUNDLE 0\r\n\
         m=application {port} UDP/DTLS/SCTP webrtc-datachannel\r\n\
         c=IN {family} {host}\r\n\
         a=setup:actpass\r\n\
         a=mid:0\r\n\
         a=sendrecv\r\n\
         a=sctp-port:5000\r\n\
         a=max-message-size:65536\r\n\
         a=ice-ufrag:{ufrag}\r\n\
         a=ice-pwd:{ufrag}\r\n\
         a=candidate:1 1 udp 2130706431 {host} {port} typ host\r\n\
         a=end-of-candidates\r\n"
    )
}

/// The addresses this port can be reached on.
///
/// The local-first listener exposes loopback only. Wider listening needs
/// a separate, explicit disclosure and authorization policy.
fn reachable_on(port: u16) -> Vec<Candidate> {
    vec![Candidate {
        host: "127.0.0.1".to_owned(),
        port,
    }]
}

/// Build the peer connection that answers one dial.
async fn answer_dial(
    ufrag: &str,
    from: SocketAddr,
    identity: &Identity,
    mux: Arc<UDPMuxDefault>,
    sessions: mpsc::Sender<Session>,
    datagrams: mpsc::Sender<Dialer>,
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
    // Match the socket's family; do not gather unrelated LAN or IPv6
    // candidates for a loopback-only listener.
    settings.set_network_types(vec![if from.is_ipv4() {
        webrtc::ice::network_type::NetworkType::Udp4
    } else {
        webrtc::ice::network_type::NetworkType::Udp6
    }]);
    // Loopback is not gathered by default, and a dial from 127.0.0.1 has
    // no other pairing available.
    if from.ip().is_loopback() {
        settings.set_include_loopback_candidate(true);
        settings.set_ip_filter(Box::new(|candidate: std::net::IpAddr| {
            candidate.is_loopback()
        }));
    }

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

    // The connection owns this callback: capturing a strong clone here
    // creates a self-cycle and leaks every completed handshake.
    let owner = Arc::downgrade(&connection);
    let dialer = ufrag.to_owned();
    let accepted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    connection.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
        let sessions = sessions.clone();
        let datagrams = datagrams.clone();
        let owner = owner.clone();
        let dialer = dialer.clone();
        let accepted = accepted.clone();
        Box::pin(async move {
            let Some(owner) = owner.upgrade() else {
                return;
            };
            if accepted.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let _ = channel.close().await;
                return;
            }
            let closing = owner.clone();
            match channel.label() {
                CHANNEL_LABEL => {
                    if sessions.try_send(Session::attach(owner, channel)).is_err() {
                        let _ = closing.close().await;
                    }
                }
                // The ufrag names this dial and nothing else: a dialer
                // chose it, used it for both its ICE fields, and the mux
                // routed on it. That makes it the one handle this side
                // has for a browser that has no address of its own.
                DATAGRAM_LABEL => {
                    if channel.ordered() || channel.max_retransmits() != Some(0) {
                        let _ = closing.close().await;
                        return;
                    }
                    if datagrams
                        .try_send(Dialer {
                            ufrag: dialer,
                            channel,
                            _connection: owner,
                        })
                        .is_err()
                    {
                        let _ = closing.close().await;
                    }
                }
                _ => {
                    let _ = closing.close().await;
                }
            }
        })
    }));

    let setup = async {
        connection
            .set_remote_description(RTCSessionDescription::offer(fabricated_offer(ufrag, from))?)
            .await?;
        let answer = connection.create_answer(None).await?;
        connection.set_local_description(answer).await
    };
    match tokio::time::timeout(Duration::from_secs(5), setup).await {
        Ok(Ok(())) => {}
        result => {
            let _ = connection.close().await;
            return Err(match result {
                Ok(Err(error)) => error.into(),
                _ => PeerError::ConnectionFailed,
            });
        }
    }
    Ok(connection)
}

/// Start listening, and produce the address a peer can dial.
///
/// Binding [`DEFAULT_PORT`] is what makes the address predictable: a
/// dialer that knows the fingerprint needs nothing else, the same way a
/// page dials a local daemon on a port it assumes. Pass `0` for an
/// ephemeral port when predictability does not matter — a test, or a
/// second listener on one machine.
///
/// With a restored [`Identity`] and a fixed port the address survives a
/// restart unchanged, so a dialer can cache it indefinitely. Dials may
/// arrive concurrently and repeatedly; each gets its own peer
/// connection over the one shared port.
/// Listen on the first free port a rendezvous spans.
///
/// A fixed port makes a machine hold one listener: the second `tonk`
/// finds it taken and fails. Walking the span means every program on the
/// machine gets its own port and stays findable, because a dialer that
/// knows the phrase knows the whole range.
///
/// Only a port already in use is skipped. Any other bind failure —
/// permissions, an address that cannot be bound at all — is returned as
/// it happens rather than retried fifteen more times, because walking
/// the span would report the last port's error for a problem that has
/// nothing to do with the port.
///
/// Fails with the last [`PeerError::Bind`] when the whole span is taken,
/// which on this design means sixteen listeners are already running.
pub async fn listen_in(
    identity: Identity,
    ports: std::ops::RangeInclusive<u16>,
) -> Result<Listener, PeerError> {
    let mut last = None;
    for port in ports {
        match listen(identity.clone(), port).await {
            Ok(listener) => return Ok(listener),
            Err(PeerError::Bind { port, detail }) if in_use(&detail) => {
                last = Some(PeerError::Bind { port, detail });
            }
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| PeerError::Bind {
        port: 0,
        detail: "the rendezvous span is empty".to_owned(),
    }))
}

/// Whether a bind failure says the port is taken.
///
/// Matched on the message because `std::io::ErrorKind::AddrInUse` is
/// lost by the time the error is a [`PeerError::Bind`] carrying a
/// string. Both spellings appear across platforms.
fn in_use(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("address already in use") || detail.contains("addrinuse")
}

/// Listen for direct dials on IPv4 loopback. Port zero allocates a free port.
pub async fn listen(identity: Identity, port: u16) -> Result<Listener, PeerError> {
    let socket = tokio::net::UdpSocket::bind(("127.0.0.1", port))
        .await
        .map_err(|error| PeerError::Bind {
            port,
            detail: error.to_string(),
        })?;
    let port = socket
        .local_addr()
        .map_err(|error| PeerError::Identity(error.to_string()))?
        .port();

    let (watching, mut dials) = Watching::wrap(Arc::new(socket));
    let mux = UDPMuxDefault::new(UDPMuxParams::new(watching));

    let (sessions, incoming) = mpsc::channel(MAX_CONCURRENT_DIALS);
    let (datagrams, dialers) = mpsc::channel(MAX_CONCURRENT_DIALS);
    let (shutdown, mut stopping) = tokio::sync::oneshot::channel();
    let accepting = {
        let datagrams = datagrams.clone();
        let identity = identity.clone();
        let mux = mux.clone();
        tokio::spawn(async move {
            // Every connection built for a dial is kept here; dropping
            // one would tear down a live channel.
            let mut connections: Vec<(Arc<RTCPeerConnection>, crate::mux::Dial, Instant)> =
                Vec::new();
            let mut cleanup = tokio::time::interval(Duration::from_secs(1));
            loop {
                let dial = tokio::select! {
                    _ = &mut stopping => break,
                    _ = cleanup.tick() => {
                        let mut index = 0;
                        while index < connections.len() {
                            let (connection, _, seen) = &mut connections[index];
                            if connection.connection_state() == RTCPeerConnectionState::Connected {
                                *seen = Instant::now();
                            }
                            if expired(connection.connection_state(), seen.elapsed()) {
                                let (connection, dial, _) = connections.swap_remove(index);
                                let _ = connection.close().await;
                                mux.remove_conn_by_ufrag(&dial.ufrag).await;
                            } else { index += 1; }
                        }
                        continue;
                    }
                    dial = dials.recv() => match dial { Some(dial) => dial, None => break },
                };
                let crate::mux::Dial {
                    ref ufrag, from, ..
                } = dial;
                // Nothing authenticates a dial at this layer, so a
                // stranger can announce an arbitrary ufrag and each one
                // would otherwise cost a peer connection and a DTLS
                // handshake. Shed closed connections first, then refuse
                // rather than grow without bound.
                if connections.len() >= MAX_CONCURRENT_DIALS {
                    tracing::warn!(%ufrag, "refusing a dial: too many already open");
                    continue;
                }

                match answer_dial(
                    ufrag,
                    from,
                    &identity,
                    mux.clone(),
                    sessions.clone(),
                    datagrams.clone(),
                )
                .await
                {
                    Ok(connection) => connections.push((connection, dial, Instant::now())),
                    // One dial failing is not the listener failing —
                    // anyone can send a packet to an open port.
                    Err(error) => {
                        tracing::debug!(%ufrag, %error, "could not answer a dial");
                    }
                }
            }
            for (connection, _, _) in connections {
                let _ = connection.close().await;
            }
            let _ = mux.close().await;
        })
    };

    Ok(Listener {
        address: Address {
            version: crate::address::VERSION,
            candidates: reachable_on(port),
            fingerprint: identity.fingerprint(),
        },
        incoming: AsyncMutex::new(incoming),
        datagrams: AsyncMutex::new(dialers),
        _accepting: accepting,
        shutdown: Some(shutdown),
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

    /// Wait for the next dialer to open a *datagram* channel.
    ///
    /// The other half of [`Self::accept`]: one address answers both, and
    /// the label the dialer chose decides which queue its channel lands
    /// in. `None` once the listener is torn down.
    pub async fn accept_datagram(&self) -> Option<Dialer> {
        self.datagrams.lock().await.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_and_stalled_handshakes_do_not_hold_listener_slots_forever() {
        assert!(expired(RTCPeerConnectionState::Failed, Duration::ZERO));
        assert!(expired(RTCPeerConnectionState::Closed, Duration::ZERO));
        assert!(!expired(
            RTCPeerConnectionState::Connecting,
            Duration::from_secs(19)
        ));
        assert!(expired(
            RTCPeerConnectionState::Connecting,
            Duration::from_secs(20)
        ));
        assert!(expired(
            RTCPeerConnectionState::Disconnected,
            Duration::from_secs(20)
        ));
        assert!(!expired(
            RTCPeerConnectionState::Connected,
            Duration::from_secs(3600)
        ));
    }

    #[tokio::test]
    async fn dropping_a_listener_releases_its_loopback_port() {
        let listener = listen(Identity::generate().unwrap(), 0).await.unwrap();
        let port = listener.address().candidates[0].port;
        assert!(listener.address().is_loopback());
        drop(listener);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if tokio::net::UdpSocket::bind(("127.0.0.1", port))
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    /// The offer has to name where the dial came from.
    ///
    /// An offer with no candidate leaves this side's ICE agent with an
    /// empty remote list, and it discards everything that arrives as
    /// `no such remote`. A browser that publishes a real host candidate
    /// survives that by peer-reflexive promotion; one that anonymises
    /// its candidates as `<uuid>.local` — Chrome's default — does not,
    /// so the channel hangs on `connecting` forever while ICE claims to
    /// be connected.
    #[test]
    fn it_offers_the_address_the_dial_arrived_from() {
        let from: SocketAddr = "127.0.0.1:50502".parse().unwrap();
        let offer = fabricated_offer("Ufrag123", from);

        assert!(
            offer.contains("a=candidate:1 1 udp 2130706431 127.0.0.1 50502 typ host"),
            "the dialer's own address must be the remote candidate:\n{offer}"
        );
        assert!(
            offer.contains("a=end-of-candidates"),
            "an unterminated candidate list leaves the agent waiting:\n{offer}"
        );
        assert!(
            offer.contains("c=IN IP4 127.0.0.1"),
            "the connection line must agree with the candidate:\n{offer}"
        );
    }

    /// IPv6 dials describe themselves as IPv6.
    #[test]
    fn it_describes_an_ipv6_dial_as_ipv6() {
        let from: SocketAddr = "[::1]:50502".parse().unwrap();
        let offer = fabricated_offer("Ufrag123", from);

        assert!(offer.contains("c=IN IP6 ::1"), "wrong family:\n{offer}");
        assert!(
            offer.contains("a=candidate:1 1 udp 2130706431 ::1 50502 typ host"),
            "wrong candidate:\n{offer}"
        );
    }

    /// Two listeners on one machine, which a fixed port made impossible.
    ///
    /// This is the whole reason for a span: the second `tonk` used to
    /// fail with "could not start the WebRTC listener" because the first
    /// held the only port a phrase derived.
    #[tokio::test]
    async fn it_gives_each_listener_its_own_port_in_the_span() {
        let span = crate::rendezvous::ports(crate::rendezvous::RENDEZVOUS);

        let first = listen_in(Identity::generate().unwrap(), span.clone())
            .await
            .expect("the first listener takes a port");
        let second = listen_in(Identity::generate().unwrap(), span.clone())
            .await
            .expect("the second listener takes the next free one");

        let (a, b) = (
            first.address().candidates[0].port,
            second.address().candidates[0].port,
        );
        assert_ne!(a, b, "two listeners must not claim one port");
        assert!(
            span.contains(&a) && span.contains(&b),
            "both are in the span: {a}, {b}"
        );
    }

    /// The span starts where the single-port derivation pointed, so a
    /// dialer that only knows `port()` still finds the first listener.
    #[test]
    fn it_starts_the_span_at_the_derived_port() {
        let phrase = crate::rendezvous::RENDEZVOUS;
        let span = crate::rendezvous::ports(phrase);
        assert_eq!(*span.start(), crate::rendezvous::port(phrase));
        assert_eq!(
            span.count(),
            crate::rendezvous::SPAN as usize,
            "the span is as wide as it claims"
        );
    }

    #[tokio::test]
    async fn the_published_address_carries_everything_a_dialer_needs() {
        let listener = listen(Identity::generate().unwrap(), 0).await.unwrap();
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
        let listener = listen(Identity::generate().unwrap(), 0).await.unwrap();
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
        let listener = listen(identity.clone(), 0).await.unwrap();
        assert_eq!(listener.address().fingerprint, identity.fingerprint());
    }

    /// Restoring the same identity republishes the same fingerprint, so
    /// an address handed out before a restart still authenticates this
    /// side afterwards. Only the port moves.
    #[tokio::test]
    async fn a_restored_identity_republishes_the_same_fingerprint() {
        let identity = Identity::generate().unwrap();
        let before = listen(identity.clone(), 0).await.unwrap();
        let restored = Identity::from_pem(&identity.to_pem()).unwrap();
        let after = listen(restored, 0).await.unwrap();
        assert_eq!(
            before.address().fingerprint,
            after.address().fingerprint,
            "a restart would invalidate every address already published"
        );
    }

    /// Every dial shares one port, so two listeners must not.
    #[tokio::test]
    async fn listeners_do_not_share_a_port() {
        let first = listen(Identity::generate().unwrap(), 0).await.unwrap();
        let second = listen(Identity::generate().unwrap(), 0).await.unwrap();
        assert_ne!(
            first.address().candidates[0].port,
            second.address().candidates[0].port
        );
    }

    #[test]
    fn an_address_survives_the_round_trip() {
        let address = Address {
            version: crate::address::VERSION,
            candidates: vec![Candidate {
                host: "127.0.0.1".into(),
                port: 41794,
            }],
            fingerprint: format!("sha-256 {}", ["AB"; 32].join(":")),
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
        let offer = fabricated_offer("their-random-ufrag", "127.0.0.1:1234".parse().unwrap());
        assert!(offer.contains("a=ice-ufrag:their-random-ufrag\r\n"));
        assert!(offer.contains("a=ice-pwd:their-random-ufrag\r\n"));
        assert!(offer.contains("m=application"));
    }
}
