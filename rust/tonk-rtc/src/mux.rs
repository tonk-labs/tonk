//! Sharing one UDP port across every dial.
//!
//! A dialer picks its own ICE ufrag, so a listener cannot know it in
//! advance — and ICE separates peers *by* ufrag, which is why a single
//! peer connection with a fixed credential serves exactly one dialer
//! ever, concurrently or sequentially.
//!
//! The way out is upstream's `UDPMuxDefault`, which routes packets to a
//! peer connection registered under a ufrag. What it does not do is
//! tell anyone about a ufrag it has never seen — an unregistered
//! packet is simply dropped, so a dial nobody was expecting goes
//! nowhere.
//!
//! Rather than reimplement the mux to add that (libp2p's own is some
//! six hundred lines), this wraps the socket the mux reads from.
//! [`Watching`] is an ordinary [`Conn`] that hands every packet
//! straight through and, on the way past, notices binding requests
//! carrying a ufrag it has not reported and announces it. The listener
//! registers a peer connection for that ufrag, and ICE retransmission
//! — every few tens of milliseconds — means the next packet lands on a
//! route that now exists. A handful of dropped packets at the start of
//! a dial costs nothing; ICE is built to expect loss.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;

use tokio::sync::mpsc;
use webrtc::stun::attributes::ATTR_USERNAME;
use webrtc::stun::message::Message;
use webrtc::util::Conn;

/// Where a dial came from, and what it called itself.
///
/// The address matters as much as the ufrag. This side fabricates the
/// dialer's offer — nothing of the dialer's SDP ever crosses — so the
/// remote candidate in that offer can only be the address its packets
/// actually arrive from. Inventing one leaves the ICE agent with a
/// remote list that matches nothing, and it discards every packet that
/// arrives as `no such remote`.
#[derive(Debug, Clone)]
pub(crate) struct Dial {
    /// The ufrag the dial is addressed to.
    pub ufrag: String,
    /// The socket address the dial arrived from.
    pub from: SocketAddr,
}

/// A socket that reports the ufrags of dials it has not seen before.
pub(crate) struct Watching {
    socket: Arc<dyn Conn + Send + Sync>,
    announce: mpsc::UnboundedSender<Dial>,
    /// Ufrags already announced. Without this every retransmission
    /// during a dial would announce again, and the listener would build
    /// a peer connection per packet.
    announced: Mutex<HashSet<String>>,
}

impl Watching {
    /// Wrap a socket, announcing new dials on the returned channel.
    pub(crate) fn wrap(
        socket: Arc<dyn Conn + Send + Sync>,
    ) -> (Self, mpsc::UnboundedReceiver<Dial>) {
        let (announce, dials) = mpsc::unbounded_channel();
        (
            Self {
                socket,
                announce,
                announced: Mutex::new(HashSet::new()),
            },
            dials,
        )
    }

    /// Announce a ufrag the first time it is seen, with where it came
    /// from.
    fn notice(&self, ufrag: String, from: SocketAddr) {
        let fresh = self
            .announced
            .lock()
            .map(|mut seen| seen.insert(ufrag.clone()))
            .unwrap_or(false);
        if fresh {
            let _ = self.announce.send(Dial { ufrag, from });
        }
    }
}

/// The ufrag a STUN binding request is addressed TO.
///
/// `USERNAME` is `<destination ufrag>:<source ufrag>`, so the first
/// half is the one this side is being asked to answer as. A dialer
/// following this design sets both halves to the same value, but
/// taking the first is what the field means and stays correct if that
/// ever changes.
fn destination_ufrag(packet: &[u8]) -> Option<String> {
    let mut message = Message::new();
    message.raw = packet.to_vec();
    message.decode().ok()?;
    let username = message.get(ATTR_USERNAME).ok()?;
    let text = String::from_utf8(username).ok()?;
    let (destination, _source) = text.split_once(':')?;
    (!destination.is_empty()).then(|| destination.to_owned())
}

#[async_trait::async_trait]
impl Conn for Watching {
    async fn connect(&self, addr: SocketAddr) -> webrtc::util::Result<()> {
        self.socket.connect(addr).await
    }

    /// Deliberately announces nothing: a connected `recv` yields no peer
    /// address, and a dial announced without one would put this side
    /// back to fabricating a remote candidate it cannot know. The mux
    /// reads through `recv_from`, so this is not the path a dial takes.
    async fn recv(&self, buf: &mut [u8]) -> webrtc::util::Result<usize> {
        self.socket.recv(buf).await
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc::util::Result<(usize, SocketAddr)> {
        let (read, from) = self.socket.recv_from(buf).await?;
        // The packet is handed on unchanged whether or not it parses:
        // this only watches, it never filters. Anything it fails to
        // understand is upstream's business.
        if let Some(ufrag) = destination_ufrag(&buf[..read]) {
            self.notice(ufrag, from);
        }
        Ok((read, from))
    }

    async fn send(&self, buf: &[u8]) -> webrtc::util::Result<usize> {
        self.socket.send(buf).await
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc::util::Result<usize> {
        self.socket.send_to(buf, target).await
    }

    fn local_addr(&self) -> webrtc::util::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.socket.remote_addr()
    }

    async fn close(&self) -> webrtc::util::Result<()> {
        self.socket.close().await
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webrtc::stun::attributes::ATTR_USERNAME;
    use webrtc::stun::message::{BINDING_REQUEST, Message};
    use webrtc::stun::textattrs::TextAttribute;

    fn binding_request(username: &str) -> Vec<u8> {
        let mut message = Message::new();
        message
            .build(&[
                Box::new(BINDING_REQUEST),
                Box::new(TextAttribute::new(ATTR_USERNAME, username.to_owned())),
            ])
            .unwrap();
        message.raw
    }

    /// `USERNAME` is `<destination>:<source>`; the destination is the
    /// ufrag this side must answer as.
    #[test]
    fn the_destination_half_of_the_username_is_taken() {
        let packet = binding_request("theirs-for-us:their-own");
        assert_eq!(destination_ufrag(&packet).as_deref(), Some("theirs-for-us"));
    }

    #[test]
    fn a_shared_credential_reads_back_as_itself() {
        let packet = binding_request("shared-value:shared-value");
        assert_eq!(destination_ufrag(&packet).as_deref(), Some("shared-value"));
    }

    /// Anything that is not a well-formed binding request with a
    /// username is simply not a dial announcement. It must not panic
    /// and must not be filtered out of the stream.
    #[test]
    fn packets_that_are_not_dials_are_ignored_quietly() {
        assert_eq!(destination_ufrag(b""), None);
        assert_eq!(destination_ufrag(b"not stun at all, just bytes"), None);
        assert_eq!(destination_ufrag(&binding_request("")), None);
        assert_eq!(destination_ufrag(&binding_request("no-colon-here")), None);
        // A plausible STUN header with nothing after it.
        assert_eq!(destination_ufrag(&[0u8; 20]), None);
    }
}
