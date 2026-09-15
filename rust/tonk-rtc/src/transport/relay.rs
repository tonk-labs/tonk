//! Wiring a transport [`Port`] to a [`MessagePort`], for a worker that
//! holds iroh but cannot hold a peer connection.
//!
//! `RTCPeerConnection` is `[Exposed=Window]`, so in a browser the peer
//! connection lives in a page. The iroh endpoint does not have to: it
//! can sit in the worker beside the replica, with the page acting as a
//! pipe. This module is the worker end of that pipe, and it knows
//! nothing about WebRTC — it moves datagrams over a `MessagePort` and
//! lets the page decide what carries them.
//!
//! That is the whole reason the transport hands out a [`Port`] rather
//! than holding a channel: [`super::web`] and this module are peers,
//! not layers.
//!
//! # Transferred, not cloned
//!
//! Every datagram crosses `postMessage`, which is far more traffic than
//! an application-level bridge carries. Each one is sent as an
//! `ArrayBuffer` in the transfer list, so ownership moves and the bytes
//! are not copied. A structured clone per packet would put a memcpy and
//! an allocation in the middle of the data path.
//!
//! Transferable *streams* would be the other shape, and are deliberately
//! not used: support is the newest of anything here, and a pair of
//! streams buys ordering this transport does not want anyway — QUIC
//! supplies its own.

use std::sync::Arc;

use bytes::Bytes;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

use super::{Port, WebRtcTransport};
use iroh_base::CustomAddr;

/// Route a peer's datagrams over a message port.
///
/// The far end of `port` is expected to own the carrier — a data
/// channel opened with [`super::web::datagram_channel`] — and to relay
/// in both directions without interpreting anything.
pub fn attach(transport: &Arc<WebRtcTransport>, peer: CustomAddr, port: MessagePort) {
    let Port {
        mut outbound,
        inbound,
    } = transport.attach(peer.clone());

    let transport_for_close = transport.clone();
    let peer_for_close = peer.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        // The far end closes by posting null rather than by closing the
        // port: a closed `MessagePort` fires no event, so a silent
        // close would leave iroh with a route to nowhere.
        if data.is_null() {
            transport_for_close.detach(&peer_for_close);
            return;
        }
        if let Ok(buffer) = data.dyn_into::<js_sys::ArrayBuffer>() {
            inbound.deliver(Bytes::from(Uint8Array::new(&buffer).to_vec()));
        }
    });
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The port owns the closure from here.
    on_message.forget();
    port.start();

    // A pump, because `poll_send` is synchronous. `spawn_local` because
    // a `MessagePort` is not `Send`.
    let sending = port.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(datagram) = outbound.recv().await {
            // Allocate in the transferable heap and move it: the buffer
            // is neutered here rather than copied into the page.
            let view = Uint8Array::new_with_length(datagram.len() as u32);
            view.copy_from(&datagram);
            let buffer = view.buffer();
            let transfer = js_sys::Array::of1(&buffer);
            if sending
                .post_message_with_transferable(&buffer, &transfer)
                .is_err()
            {
                break;
            }
        }
    });
}
