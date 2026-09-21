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
use std::{cell::Cell, rc::Rc};

use bytes::Bytes;
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, MessagePort};

use super::{Inbound, Port, WebRtcTransport};
use iroh_base::CustomAddr;

/// Route a peer's datagrams over a message port.
///
/// The far end of `port` is expected to own the carrier — a data
/// channel opened with [`super::web::datagram_channel`] — and to relay
/// in both directions without interpreting anything.
pub fn attach(transport: &Arc<WebRtcTransport>, peer: CustomAddr, port: MessagePort) -> Inbound {
    let Port {
        mut outbound,
        inbound,
    } = transport.attach(peer);

    let pumping = inbound.clone();
    let lease = inbound.clone();
    let outstanding = Rc::new(Cell::new(0_usize));
    let acknowledged = outstanding.clone();
    let last_pong = Rc::new(Cell::new(js_sys::Date::now()));
    let seen_pong = last_pong.clone();
    let replying = port.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let data = event.data();
        // The far end closes by posting null rather than by closing the
        // port: a closed `MessagePort` fires no event, so a silent
        // close would leave iroh with a route to nowhere.
        if data.is_null() {
            inbound.detach();
            return;
        }
        if let Some(control) = data.as_string() {
            match control.as_str() {
                "ack" => acknowledged.set(acknowledged.get().saturating_sub(1)),
                "pong" => seen_pong.set(js_sys::Date::now()),
                "ping" => {
                    let _ = replying.post_message(&JsValue::from_str("pong"));
                }
                _ => {}
            }
            return;
        }
        if let Ok(buffer) = data.dyn_into::<js_sys::ArrayBuffer>() {
            let _ = replying.post_message(&JsValue::from_str("ack"));
            inbound.deliver(Bytes::from(Uint8Array::new(&buffer).to_vec()));
        }
    });
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    port.start();

    // A closed/frozen document does not notify a MessagePort. A lease
    // also bounds the time an abandoned pump keeps its closures alive.
    let watching = lease.clone();
    let heartbeat_port = port.clone();
    let heartbeat = gloo_timers::callback::Interval::new(5000, move || {
        if js_sys::Date::now() - last_pong.get() >= 15_000.0
            || heartbeat_port
                .post_message(&JsValue::from_str("ping"))
                .is_err()
        {
            watching.detach();
        }
    });

    // A pump, because `poll_send` is synchronous. `spawn_local` because
    // a `MessagePort` is not `Send`.
    let sending = port.clone();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(datagram) = outbound.recv().await {
            if !pumping.is_current() {
                break;
            }
            if outstanding.get() >= 64 {
                continue;
            }
            outstanding.set(outstanding.get() + 1);
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
        pumping.detach();
        let _ = sending.post_message(&JsValue::NULL);
        sending.set_onmessage(None);
        sending.close();
        drop((on_message, heartbeat));
    });
    lease
}
