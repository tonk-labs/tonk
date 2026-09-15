//! Wiring a browser `RTCDataChannel` to a transport [`Port`].
//!
//! The mirror of [`super::native`], and the reason the transport core
//! holds no channel: an `RtcDataChannel` is neither `Send` nor `Sync`,
//! while `CustomTransport` requires both. Keeping the platform object
//! here — owned by closures and a `spawn_local` task that never cross a
//! thread — means the bound is satisfied without a `SendWrapper` and
//! without pretending a JS handle is thread safe.

use std::sync::Arc;

use bytes::Bytes;
use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    MessageEvent, RtcDataChannel, RtcDataChannelInit, RtcDataChannelType, RtcPeerConnection,
};

use super::{Port, WebRtcTransport};
use iroh_base::CustomAddr;

/// Open a data channel suited to carrying QUIC.
///
/// Unreliable and unordered, and set to hand back `ArrayBuffer` rather
/// than `Blob` — a Blob would have to be read asynchronously, which
/// turns every inbound datagram into a task and reorders them.
pub fn datagram_channel(connection: &RtcPeerConnection, label: &str) -> RtcDataChannel {
    let mut options = RtcDataChannelInit::new();
    options.set_ordered(false);
    options.set_max_retransmits(0);
    let channel = connection.create_data_channel_with_data_channel_dict(label, &options);
    channel.set_binary_type(RtcDataChannelType::Arraybuffer);
    channel
}

/// Route a peer's datagrams over this channel.
pub fn attach(transport: &Arc<WebRtcTransport>, peer: CustomAddr, channel: RtcDataChannel) {
    let Port {
        mut outbound,
        inbound,
    } = transport.attach(peer.clone());

    channel.set_binary_type(RtcDataChannelType::Arraybuffer);

    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // Anything that is not an ArrayBuffer is not a datagram this
        // transport sent, so it is ignored rather than guessed at.
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            inbound.deliver(Bytes::from(Uint8Array::new(&buffer).to_vec()));
        }
    });
    channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The closure must outlive this call; the channel owns it from here.
    on_message.forget();

    let transport_for_close = transport.clone();
    let peer_for_close = peer.clone();
    let on_close = Closure::<dyn FnMut()>::new(move || {
        transport_for_close.detach(&peer_for_close);
    });
    channel.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    // A pump, because `poll_send` is synchronous and this send may
    // throw when the channel's buffer is full. `spawn_local` because
    // nothing here is `Send`.
    let sending = channel.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(datagram) = outbound.recv().await {
            if sending.ready_state() != web_sys::RtcDataChannelState::Open {
                break;
            }
            // A throw here means the send buffer is full: drop the
            // datagram, as a socket would, and let QUIC notice.
            let _ = sending.send_with_u8_array(&datagram);
        }
    });
}
