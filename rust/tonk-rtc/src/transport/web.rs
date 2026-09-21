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
    MessageEvent, MessagePort, RtcDataChannel, RtcDataChannelInit, RtcDataChannelType,
    RtcPeerConnection,
};

use super::{Port, WebRtcTransport};
use iroh_base::CustomAddr;

/// Open a data channel suited to carrying QUIC.
///
/// Unreliable and unordered, and set to hand back `ArrayBuffer` rather
/// than `Blob` — a Blob would have to be read asynchronously, which
/// turns every inbound datagram into a task and reorders them.
pub fn datagram_channel(connection: &RtcPeerConnection, label: &str) -> RtcDataChannel {
    let options = RtcDataChannelInit::new();
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
    } = transport.attach(peer);

    let closing = inbound.clone();
    let pumping = inbound.clone();

    channel.set_binary_type(RtcDataChannelType::Arraybuffer);

    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // Anything that is not an ArrayBuffer is not a datagram this
        // transport sent, so it is ignored rather than guessed at.
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            inbound.deliver(Bytes::from(Uint8Array::new(&buffer).to_vec()));
        }
    });
    channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    let on_close = Closure::<dyn FnMut()>::new(move || {
        closing.detach();
    });
    channel.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    // A pump, because `poll_send` is synchronous and this send may
    // throw when the channel's buffer is full. `spawn_local` because
    // nothing here is `Send`.
    let sending = channel.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(datagram) = outbound.recv().await {
            if !pumping.is_current() || sending.ready_state() != web_sys::RtcDataChannelState::Open
            {
                break;
            }
            // A throw here means the send buffer is full: drop the
            // datagram, as a socket would, and let QUIC notice.
            let _ = sending.send_with_u8_array(&datagram);
        }
        pumping.detach();
        sending.set_onmessage(None);
        sending.set_onclose(None);
        sending.close();
        drop((on_message, on_close));
    });
}

/// Relay a data channel to a [`MessagePort`], and back.
///
/// The page half of [`super::relay`]. The worker holds the transport and
/// the iroh endpoint; the page holds the peer connection, because
/// `RTCPeerConnection` is `[Exposed=Window]` and does not exist in a
/// worker. This is the pipe between them, and it interprets nothing —
/// datagrams cross in both directions and the page never learns what
/// they mean.
///
/// # Transferred, not copied
///
/// Every datagram crosses `postMessage`, which is far more traffic than
/// an application-level bridge carries, so each one moves as an
/// `ArrayBuffer` in the transfer list. The buffer is neutered here
/// rather than cloned; a structured clone per packet would put a memcpy
/// and an allocation in the middle of the data path.
///
/// # Closing is explicit
///
/// A closed channel posts `null` rather than closing the port. A closed
/// `MessagePort` fires no event, so a silent close would leave iroh
/// holding a route to nowhere — the worker end reads that `null` as the
/// signal to detach.
pub fn relay(channel: RtcDataChannel, port: MessagePort) {
    channel.set_binary_type(RtcDataChannelType::Arraybuffer);

    // Channel -> port.
    let to_worker = port.clone();
    let inbound = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            let transfer = js_sys::Array::of1(&buffer);
            let _ = to_worker.post_message_with_transferable(&buffer, &transfer);
        }
    });
    channel.set_onmessage(Some(inbound.as_ref().unchecked_ref()));
    inbound.forget();

    // Port -> channel. A send can fail while the channel is closing,
    // which is ordinary: QUIC treats a lost datagram as loss and
    // retransmits, so there is nothing to report here.
    let sending = channel.clone();
    let outbound = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Ok(buffer) = event.data().dyn_into::<ArrayBuffer>() {
            let _ = sending.send_with_array_buffer(&buffer);
        }
    });
    port.set_onmessage(Some(outbound.as_ref().unchecked_ref()));
    outbound.forget();
    port.start();

    // Tell the worker when the carrier goes away, since the port will
    // not.
    let closing = port.clone();
    let on_close = Closure::<dyn FnMut()>::new(move || {
        let _ = closing.post_message(&JsValue::NULL);
    });
    channel.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();
}
