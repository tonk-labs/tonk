//! Carrying iroh over a WebRTC data channel.
//!
//! A data channel on its own gives 64 KiB messages and nothing else —
//! no multiplexing, no flow control, no way to cancel one exchange
//! without dropping the connection. iroh gives all of that, over QUIC,
//! but in a browser it can only reach a relay, because browsers cannot
//! open UDP sockets.
//!
//! iroh's custom transport closes that gap. A transport supplies
//! datagrams; QUIC is layered on top by iroh. So a WebRTC data channel
//! becomes a route to a peer exactly like an IP address or a relay —
//! `TransportAddr::Custom` sits beside `Relay` and `Ip`, and one
//! `EndpointAddr` holds all three at once. iroh's path selection picks
//! whichever works, which offline means this one.
//!
//! What that buys, beyond streams: the local dial stops being a
//! separate code path. Callers say `endpoint.connect(id, alpn)` and
//! never learn which route carried it.
//!
//! # The channel must be unreliable and unordered
//!
//! QUIC supplies its own reliability, ordering and congestion control.
//! Run it over a reliable ordered channel and the two fight: the
//! channel retransmits a datagram QUIC has already given up on, and
//! head-of-line blocking appears in a protocol designed to avoid it.
//! Quiet networks hide this completely — it shows up under loss, as
//! latency that grows instead of recovering.
//!
//! So channels for this transport are opened with `ordered: false` and
//! `max_retransmits: 0`. [`datagram_channel`] is the only way this
//! module accepts one.
//!
//! # Both targets, one core
//!
//! The transport never holds a data channel. It hands out a [`Port`] —
//! a queue of datagrams to send and an [`Inbound`] to deliver received
//! ones to — and the platform glue owns the channel: [`native`] for
//! `webrtc-rs`, [`web`] for a browser's `RtcDataChannel`.
//!
//! That is not tidiness. `CustomTransport` demands `Send + Sync`, and a
//! browser `RtcDataChannel` is neither. Keeping it outside satisfies
//! the bound honestly, rather than wrapping a JS handle in a
//! `SendWrapper` and asserting a thread-safety that is not there.
//!
//! `tests/iroh_over_webrtc.rs` proves the native path end to end. The
//! browser path was proved separately, out of tree: two peer
//! connections in one page, one unreliable unordered channel between
//! them, two `presets::Empty` endpoints over it — no relay, no
//! discovery, and in a browser no IP transport either, so a completed
//! exchange completed over the channel. Chromium, WebKit and Firefox
//! all pass, with no per-engine accommodation. That harness is not
//! checked in: CI has no browser runner for this crate.
//!
//! # What it took
//!
//! Three things were not obvious and cost time:
//!
//! - `RecvMeta` is `#[non_exhaustive]`, so it is filled field by field
//!   rather than with a struct literal.
//! - `presets::Empty` means **empty** — no relay, no discovery, and no
//!   crypto provider. `Builder::crypto_provider(iroh::tls::default_provider())`
//!   is mandatory with it, and its absence surfaces as
//!   "Missing or incompatible rustls crypto provider", which reads like
//!   a feature-flag problem and is not one.
//! - `poll_send` is synchronous while a data channel's send is not, so
//!   each peer gets a bounded queue and a pump task. A full queue drops,
//!   which is what a socket does and what QUIC expects.
//!
//! # The dependency conflict, and its fix
//!
//! `iroh 1.2` wants `ed25519-dalek 3.0.0-rc.0`, which needs the
//! released `sha2 0.11`. `dialog-remote-s3` used to pull `s3s 0.13`,
//! which pinned `sha2 = 0.11.0-rc.5` exactly, so the two could not share
//! a binary at all.
//!
//! Fixed upstream by bumping dialog-db to `s3s 0.16`, which drops the
//! pin. That bump was free: `s3s` is used there only by test helpers —
//! a local S3 server — never the production path, and three minor
//! versions needed no API change.
//!
//! With it, `tonk-cli` checks clean carrying **both** iroh and
//! `dialog-remote-s3`. Tonk's dialog-db dependencies point at the
//! branch holding that bump until it is tagged.

use std::collections::HashMap;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use iroh::endpoint::transports::{
    CustomEndpoint, CustomSender, CustomTransport, RecvInfo, Transmit,
};
use iroh_base::CustomAddr;
use tokio::sync::mpsc;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;
#[cfg(target_arch = "wasm32")]
pub mod web;

#[cfg(not(target_arch = "wasm32"))]
pub use native::{attach, datagram_channel};
#[cfg(target_arch = "wasm32")]
pub use web::{attach, datagram_channel};

/// Identifies this transport's address namespace.
///
/// A `CustomAddr` carries an id and opaque bytes; the id is what tells
/// iroh which transport can reach it, so two custom transports in one
/// endpoint do not collide. "tonkrtc1" as bytes.
pub const TRANSPORT_ID: u64 = u64::from_be_bytes(*b"tonkrtc1");

/// How many outbound datagrams may queue for one peer before they are
/// dropped.
///
/// Dropping is correct here: this is a datagram transport, a UDP socket
/// drops too, and QUIC is built to notice and retransmit. Blocking
/// would instead stall iroh's driver behind one slow peer.
const OUTBOUND_QUEUE: usize = 256;

/// One peer's channel, and the queue feeding it.
#[derive(Debug)]
struct Peer {
    outbound: mpsc::Sender<Bytes>,
}

/// Shared state between the transport, its endpoint and its senders.
#[derive(Debug, Default)]
struct Routes {
    peers: Mutex<HashMap<CustomAddr, Peer>>,
}

/// A WebRTC-backed iroh transport.
///
/// Created before the iroh endpoint, because `Builder::add_custom_transport`
/// takes it at build time. Channels are attached later, as peers are
/// dialed or accepted, through [`WebRtcTransport::attach`].
#[derive(Debug)]
pub struct WebRtcTransport {
    local: CustomAddr,
    routes: Arc<Routes>,
    inbound: Mutex<Option<mpsc::Receiver<(CustomAddr, Bytes)>>>,
    announce: mpsc::Sender<(CustomAddr, Bytes)>,
}

impl WebRtcTransport {
    /// Create a transport announcing `local` as its address.
    ///
    /// `local` is what a peer puts in an `EndpointAddr` to reach this
    /// endpoint over WebRTC — for the local-dial case, the encoded
    /// `dial::Address`.
    pub fn new(local: impl AsRef<[u8]>) -> Arc<Self> {
        let (announce, inbound) = mpsc::channel(OUTBOUND_QUEUE);
        Arc::new(Self {
            local: CustomAddr::from_parts(TRANSPORT_ID, local.as_ref()),
            routes: Arc::default(),
            inbound: Mutex::new(Some(inbound)),
            announce,
        })
    }

    /// The address peers use to reach this endpoint over WebRTC.
    pub fn local_addr(&self) -> CustomAddr {
        self.local.clone()
    }

    /// Open a route to `peer`, returning the two ends the caller wires
    /// to an actual data channel.
    ///
    /// The transport deliberately never holds the channel. That keeps
    /// this type `Send + Sync` — which `CustomTransport` demands and a
    /// browser's `RtcDataChannel` is not — so the platform object stays
    /// in the glue that owns it, and no `SendWrapper` is needed here.
    ///
    /// The channel the caller wires up must be **unreliable and
    /// unordered**; see the module note on why a reliable one is
    /// actively harmful rather than merely wasteful.
    pub fn attach(&self, peer: CustomAddr) -> Port {
        let (outbound, queued) = mpsc::channel::<Bytes>(OUTBOUND_QUEUE);
        if let Ok(mut peers) = self.routes.peers.lock() {
            peers.insert(peer.clone(), Peer { outbound });
        }
        Port {
            outbound: queued,
            inbound: Inbound {
                peer,
                announce: self.announce.clone(),
            },
        }
    }

    /// Forget a peer, because its channel closed.
    pub fn detach(&self, peer: &CustomAddr) {
        if let Ok(mut peers) = self.routes.peers.lock() {
            peers.remove(peer);
        }
    }
}

/// The two ends of one peer's route.
///
/// The caller pumps [`Port::outbound`] into its data channel and calls
/// [`Inbound::deliver`] for every message that arrives on it.
#[derive(Debug)]
pub struct Port {
    /// Datagrams iroh wants sent to this peer.
    pub outbound: mpsc::Receiver<Bytes>,
    /// Where datagrams from this peer go.
    pub inbound: Inbound,
}

/// Hands datagrams from one peer to iroh.
#[derive(Debug, Clone)]
pub struct Inbound {
    peer: CustomAddr,
    announce: mpsc::Sender<(CustomAddr, Bytes)>,
}

impl Inbound {
    /// Deliver one datagram received from this peer.
    ///
    /// Dropped when iroh is not draining, which is what a socket does
    /// and what QUIC is built to notice.
    pub fn deliver(&self, datagram: Bytes) {
        let _ = self.announce.try_send((self.peer.clone(), datagram));
    }
}

impl CustomTransport for WebRtcTransport {
    fn bind(&self) -> io::Result<Box<dyn CustomEndpoint>> {
        let inbound = self
            .inbound
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .ok_or_else(|| {
                io::Error::other("this WebRTC transport is already bound to an endpoint")
            })?;
        let addrs = n0_watcher::Watchable::new(vec![self.local.clone()]);
        Ok(Box::new(Endpoint {
            routes: self.routes.clone(),
            inbound,
            addrs,
        }))
    }
}

/// The bound half: receives datagrams and hands out senders.
#[derive(Debug)]
struct Endpoint {
    routes: Arc<Routes>,
    inbound: mpsc::Receiver<(CustomAddr, Bytes)>,
    addrs: n0_watcher::Watchable<Vec<CustomAddr>>,
}

impl CustomEndpoint for Endpoint {
    fn watch_local_addrs(&self) -> n0_watcher::Direct<Vec<CustomAddr>> {
        self.addrs.watch()
    }

    fn create_sender(&self) -> Arc<dyn CustomSender> {
        Arc::new(Sender {
            routes: self.routes.clone(),
        })
    }

    fn poll_recv(
        &mut self,
        cx: &mut Context,
        bufs: &mut [io::IoSliceMut<'_>],
        metas: &mut [noq_udp::RecvMeta],
        recv_infos: &mut [RecvInfo],
    ) -> Poll<io::Result<usize>> {
        let mut filled = 0;
        while filled < bufs.len() {
            match self.inbound.poll_recv(cx) {
                Poll::Ready(Some((from, datagram))) => {
                    let len = datagram.len().min(bufs[filled].len());
                    bufs[filled][..len].copy_from_slice(&datagram[..len]);
                    // `RecvMeta` is non-exhaustive, so it is filled
                    // field by field rather than by struct literal.
                    let mut meta = noq_udp::RecvMeta::default();
                    // A WebRTC peer has no meaningful socket address;
                    // iroh routes on the custom address in `recv_infos`,
                    // so this only has to be present and stable.
                    meta.addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));
                    meta.len = len;
                    meta.stride = len;
                    metas[filled] = meta;
                    recv_infos[filled] = RecvInfo::new(from, None);
                    filled += 1;
                }
                // Nothing more right now; hand over what we have rather
                // than parking with datagrams already in hand.
                Poll::Pending => break,
                // Every channel is gone and the transport was dropped.
                Poll::Ready(None) => {
                    return if filled > 0 {
                        Poll::Ready(Ok(filled))
                    } else {
                        Poll::Ready(Err(io::Error::other("the WebRTC transport is closed")))
                    };
                }
            }
        }

        if filled > 0 {
            Poll::Ready(Ok(filled))
        } else {
            Poll::Pending
        }
    }
}

/// The sending half. Cheap to clone; iroh makes several.
#[derive(Debug)]
struct Sender {
    routes: Arc<Routes>,
}

impl CustomSender for Sender {
    fn is_valid_send_addr(&self, addr: &CustomAddr) -> bool {
        addr.id() == TRANSPORT_ID
            && self
                .routes
                .peers
                .lock()
                .map(|peers| peers.contains_key(addr))
                .unwrap_or(false)
    }

    fn poll_send(
        &self,
        _cx: &mut Context,
        dst: &CustomAddr,
        _src: Option<&CustomAddr>,
        transmit: &Transmit<'_>,
    ) -> Poll<io::Result<()>> {
        let Ok(peers) = self.routes.peers.lock() else {
            return Poll::Ready(Err(io::Error::other("the WebRTC route table is poisoned")));
        };
        let Some(peer) = peers.get(dst) else {
            return Poll::Ready(Err(io::Error::other("no WebRTC channel for that peer")));
        };
        // Queue full means the peer is not keeping up. Report success
        // and drop, exactly as a socket would: QUIC notices the loss and
        // slows down, whereas surfacing an error here would tear down a
        // connection that is merely congested.
        let _ = peer
            .outbound
            .try_send(Bytes::copy_from_slice(transmit.contents));
        Poll::Ready(Ok(()))
    }
}
