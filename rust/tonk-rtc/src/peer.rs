//! The native half of the connection: a peer that offers, then talks.
//!
//! Split into two types on purpose. [`Offering`] is the state between
//! "we have an offer" and "the peer answered" — the only thing you can
//! do with it is read the offer or accept an answer. [`Session`] is what
//! you get once the data channel is open, and the only thing that can
//! send. The states are hard to confuse because the compiler will not
//! let you send on an `Offering`.
//!
//! # Why the offer is not trickled
//!
//! Production WebRTC trickles ICE: candidates are discovered
//! asynchronously and shipped to the peer as they appear, so the
//! connection starts forming before gathering finishes. That needs a
//! *live* signalling channel in both directions.
//!
//! This PoC's channel is a pair of browser navigations (see
//! [`crate::loopback`]) — one message each way, nothing left open
//! afterwards. So we wait for gathering to complete and ship a single
//! description with every candidate already in it. On loopback that
//! wait is milliseconds; across a real network with STUN it is a second
//! or two of dead air before the browser can even answer.
//!
//! That tradeoff belongs to the *signalling channel*, not to this
//! module. Once descriptions travel as facts in a replicated space —
//! a channel that stays open — [`Offering::gather`] is the thing to
//! delete, and `on_ice_candidate` becomes the thing to implement.

use std::sync::{Arc, Mutex};

use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
    SettingEngine,
};

use crate::signal::{Description, Role};

/// How long to wait for ICE gathering before shipping what we have.
///
/// This ceremony sends one description and then closes the signalling
/// channel, so candidates found after this point are lost — hence the
/// wait at all. Bounded because an unbounded one is a hang.
const GATHER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

/// The data channel label both peers agree on.
///
/// The browser half opens no channel of its own — it adopts the one
/// this peer creates — so the label only has to be recognisable in a
/// `chrome://webrtc-internals` dump.
pub const CHANNEL_LABEL: &str = "tonk";

/// Anything that can go wrong bringing a peer up.
#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    /// The WebRTC stack refused an operation.
    #[error("webrtc: {0}")]
    WebRtc(#[from] webrtc::error::Error),
    /// A session description could not be read.
    #[error(transparent)]
    Signal(#[from] crate::signal::DecodeError),
    /// ICE gathering finished without producing a local description.
    #[error("the peer produced no local description")]
    NoLocalDescription,
    /// The connection failed or closed before the channel opened.
    #[error("the connection failed before the data channel opened")]
    ConnectionFailed,
}

/// A slot holding a one-shot sender that several callbacks race to fill.
type Trigger<T> = Arc<Mutex<Option<oneshot::Sender<T>>>>;

/// Fire a trigger if nobody has yet. Later attempts are no-ops.
fn fire<T>(trigger: &Trigger<T>, value: T) {
    if let Some(sender) = trigger.lock().ok().and_then(|mut slot| slot.take()) {
        let _ = sender.send(value);
    }
}

/// Watches the connection for the two moments this crate cares about:
/// ICE gathering finishing, and the connection giving up.
struct Watcher {
    gathered: Trigger<()>,
    lost: Trigger<()>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Watcher {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            fire(&self.gathered, ());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if matches!(
            state,
            RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Closed
                | RTCPeerConnectionState::Disconnected
        ) {
            // Also release anyone still waiting on gathering: a
            // connection that died mid-gather will never complete it.
            fire(&self.lost, ());
            fire(&self.gathered, ());
        }
    }
}

/// A peer that has made an offer and is waiting for the answer.
pub struct Offering {
    connection: Arc<dyn PeerConnection>,
    channel: Arc<dyn DataChannel>,
    inbound: mpsc::UnboundedReceiver<String>,
    opened: oneshot::Receiver<Result<(), PeerError>>,
    offer: Description,
}

/// An open data channel to the browser peer.
pub struct Session {
    connection: Arc<dyn PeerConnection>,
    channel: Arc<dyn DataChannel>,
    inbound: AsyncMutex<mpsc::UnboundedReceiver<String>>,
}

/// Build a peer, create the data channel, and produce the offer.
///
/// `ice_servers` may be empty, which is the right setting for two
/// processes on one machine: host candidates alone suffice, and
/// skipping STUN takes a network round-trip out of the ceremony. Supply
/// a STUN URL when the peers are on different networks.
pub async fn offer(ice_servers: Vec<String>) -> Result<Offering, PeerError> {
    let mut configuration = RTCConfigurationBuilder::default();
    if !ice_servers.is_empty() {
        configuration = configuration.with_ice_servers(vec![RTCIceServer {
            urls: ice_servers,
            ..Default::default()
        }]);
    }

    let gathered_trigger: Trigger<()> = Arc::new(Mutex::new(None));
    let lost_trigger: Trigger<()> = Arc::new(Mutex::new(None));
    let (gathered_tx, gathered) = oneshot::channel();
    let (lost_tx, lost) = oneshot::channel();
    *gathered_trigger.lock().expect("fresh mutex") = Some(gathered_tx);
    *lost_trigger.lock().expect("fresh mutex") = Some(lost_tx);

    let connection: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_configuration(configuration.build())
            .with_handler(Arc::new(Watcher {
                gathered: gathered_trigger,
                lost: lost_trigger,
            }))
            // NOT optional, despite looking like a no-op.
            //
            // `PeerConnectionBuilder`'s own default is
            // `MulticastDnsMode::Disabled`, which DISCARDS remote mDNS
            // candidates. Chrome and Safari emit only mDNS host
            // candidates (`<uuid>.local`) by default, to avoid leaking
            // private IPs to a page — so against the builder default a
            // browser offer arrives with every candidate dropped and no
            // connection can ever form. `SettingEngine`'s default is
            // `QueryOnly`: resolve the peer's mDNS names, without
            // publishing our own host IPs as mDNS names.
            //
            // The consequence to remember: a same-machine connection
            // depends on mDNS resolution working on the host. It does on
            // an ordinary desktop; it does not inside a container with
            // no multicast.
            .with_setting_engine(SettingEngine::default())
            // Port 0 on every interface: the OS picks, and every local
            // address becomes a host candidate.
            .with_udp_addrs(vec!["0.0.0.0:0"])
            .build()
            .await?,
    );

    // Creating the channel BEFORE the offer is what puts an
    // `m=application` section in the SDP. Create it after and the offer
    // describes a connection with nothing in it, so the browser has
    // nothing to answer and no channel ever opens.
    let channel = connection.create_data_channel(CHANNEL_LABEL, None).await?;

    let (opened_tx, opened) = oneshot::channel();
    let (messages, inbound) = mpsc::unbounded_channel();
    pump(channel.clone(), messages, opened_tx, lost);

    let local = connection.create_offer(None).await?;
    connection.set_local_description(local).await?;

    // Awaited only after `set_local_description`, which is what starts
    // gathering — and bounded, because "complete" is not guaranteed to
    // arrive promptly (an unreachable STUN server, a host with no
    // multicast). The description already carries every candidate found
    // so far, so a timeout ships those rather than hanging.
    let _ = tokio::time::timeout(GATHER_DEADLINE, gathered).await;

    let offer = connection
        .local_description()
        .await
        .ok_or(PeerError::NoLocalDescription)?;

    Ok(Offering {
        connection,
        channel,
        inbound,
        opened,
        offer: Description::offer(offer.sdp),
    })
}

/// Drain the channel's event stream for as long as it lives.
///
/// Reports the channel opening (or the connection dying first) through
/// `opened`, and forwards every text message to `messages`. Dropping
/// the receivers is how a caller unsubscribes: the sends start failing
/// and the task falls out of its loop.
fn pump(
    channel: Arc<dyn DataChannel>,
    messages: mpsc::UnboundedSender<String>,
    opened: oneshot::Sender<Result<(), PeerError>>,
    lost: oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut opened = Some(opened);
        // A connection that fails before the channel opens must release
        // the caller, so the failure signal races the event stream.
        tokio::pin!(lost);

        loop {
            let event = tokio::select! {
                event = channel.poll() => event,
                _ = &mut lost => {
                    if let Some(opened) = opened.take() {
                        let _ = opened.send(Err(PeerError::ConnectionFailed));
                    }
                    return;
                }
            };

            match event {
                Some(DataChannelEvent::OnOpen) => {
                    if let Some(opened) = opened.take() {
                        let _ = opened.send(Ok(()));
                    }
                }
                Some(DataChannelEvent::OnMessage(message)) => {
                    // A PoC chat channel carries text. Anything that is
                    // not valid UTF-8 comes from a peer we do not
                    // understand; drop it rather than guess an encoding.
                    if let Ok(text) = String::from_utf8(message.data.to_vec())
                        && messages.send(text).is_err()
                    {
                        return;
                    }
                }
                Some(DataChannelEvent::OnClose) | None => {
                    if let Some(opened) = opened.take() {
                        let _ = opened.send(Err(PeerError::ConnectionFailed));
                    }
                    return;
                }
                Some(_) => {}
            }
        }
    });
}

impl Offering {
    /// The offer to hand the browser, encoded for a URL.
    pub fn offer(&self) -> String {
        self.offer.encode()
    }

    /// Take the browser's answer and wait for the channel to open.
    ///
    /// Returns once the channel is usable, or once the connection has
    /// failed — it does not wait forever on a peer that answered and
    /// then vanished, because the same signal carries both outcomes.
    pub async fn accept(self, answer: &str) -> Result<Session, PeerError> {
        let answer = Description::decode(answer, Role::Answer)?;
        self.connection
            .set_remote_description(RTCSessionDescription::answer(answer.sdp)?)
            .await?;

        match self.opened.await {
            Ok(Ok(())) => Ok(Session {
                connection: self.connection,
                channel: self.channel,
                inbound: AsyncMutex::new(self.inbound),
            }),
            Ok(Err(error)) => Err(error),
            // The pump dropped its sender without sending, which means
            // the task itself went away.
            Err(_) => Err(PeerError::ConnectionFailed),
        }
    }
}

impl Session {
    /// Send one text message to the browser.
    pub async fn send(&self, text: &str) -> Result<(), PeerError> {
        self.channel.send_text(text).await?;
        Ok(())
    }

    /// Await the next message from the browser.
    ///
    /// `None` once the channel is closed and every message already
    /// received has been handed out.
    pub async fn recv(&self) -> Option<String> {
        self.inbound.lock().await.recv().await
    }

    /// Close the connection.
    pub async fn close(self) -> Result<(), PeerError> {
        self.connection.close().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The offer has to describe a data channel, or the browser has
    /// nothing to answer — the failure mode if `create_data_channel`
    /// ever drifts after `create_offer`.
    #[tokio::test]
    async fn an_offer_describes_a_data_channel() {
        let offering = offer(Vec::new()).await.unwrap();
        let decoded = Description::decode(&offering.offer(), Role::Offer).unwrap();
        assert!(
            decoded.sdp.contains("m=application"),
            "no data channel in the offer:\n{}",
            decoded.sdp
        );
        assert!(decoded.sdp.contains("webrtc-datachannel"));
    }

    /// With no ICE servers configured the offer still carries host
    /// candidates — what makes a same-machine connection work without
    /// STUN, and the thing that silently breaks if gathering is no
    /// longer awaited before the description is read.
    #[tokio::test]
    async fn gathering_completes_with_host_candidates_and_no_stun() {
        let offering = offer(Vec::new()).await.unwrap();
        let decoded = Description::decode(&offering.offer(), Role::Offer).unwrap();
        assert!(
            decoded.sdp.contains("a=candidate:"),
            "no ICE candidates were gathered into the offer:\n{}",
            decoded.sdp
        );
    }

    /// An "answer" that is really an offer is refused at the envelope,
    /// before it can reach `setRemoteDescription`.
    #[tokio::test]
    async fn accepting_the_wrong_half_fails_cleanly() {
        let offering = offer(Vec::new()).await.unwrap();
        let own_offer = offering.offer();
        assert!(matches!(
            offering.accept(&own_offer).await,
            Err(PeerError::Signal(crate::signal::DecodeError::Role { .. }))
        ));
    }
}
