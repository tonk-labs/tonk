//! Wiring a `webrtc-rs` data channel to a transport [`Port`].
//!
//! The transport itself never sees this type; it only knows the port.

use std::sync::Arc;

use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::peer_connection::RTCPeerConnection;

use super::{Port, WebRtcTransport};
use crate::peer::PeerError;
use iroh_base::CustomAddr;

/// Open a data channel suited to carrying QUIC.
///
/// Unreliable and unordered — see the module note on why a reliable
/// ordered channel is actively harmful here.
pub async fn datagram_channel(
    connection: &Arc<RTCPeerConnection>,
    label: &str,
) -> Result<Arc<RTCDataChannel>, PeerError> {
    Ok(connection
        .create_data_channel(
            label,
            Some(RTCDataChannelInit {
                ordered: Some(false),
                max_retransmits: Some(0),
                ..Default::default()
            }),
        )
        .await?)
}

/// Route a peer's datagrams over this channel.
pub fn attach(transport: &Arc<WebRtcTransport>, peer: CustomAddr, channel: Arc<RTCDataChannel>) {
    let Port {
        mut outbound,
        inbound,
    } = transport.attach(peer.clone());

    channel.on_message(Box::new(move |message: DataChannelMessage| {
        let inbound = inbound.clone();
        Box::pin(async move { inbound.deliver(message.data) })
    }));

    // A pump, because `poll_send` is synchronous and this send is not.
    let sending = channel.clone();
    tokio::spawn(async move {
        while let Some(datagram) = outbound.recv().await {
            if sending.send(&datagram).await.is_err() {
                break;
            }
        }
    });

    let transport = transport.clone();
    channel.on_close(Box::new(move || {
        let transport = transport.clone();
        let peer = peer.clone();
        Box::pin(async move { transport.detach(&peer) })
    }));
}
