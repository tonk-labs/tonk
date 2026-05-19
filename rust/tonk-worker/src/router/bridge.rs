//! Iframe ↔ service-worker bridge.
//!
//! Two responsibilities in Increment 1:
//!
//! 1. Serve the iframe-side bridge module at `/__tonk/bridge.js`.
//!    The wrapper injected by `host::wrap_html_body` loads it as
//!    an ES module.
//! 2. Handle `message` events from view clients. The iframe sends
//!    `{v:1,type:"hello"}` with a transferred `MessagePort`; the
//!    SW enumerates the view's subscription claims, opens reactor
//!    subscriptions, and pumps snapshots back over the port.
//!
//! Increment 1 ships only the serve route; the message handler
//! lives in later tasks.

use std::collections::HashMap;
use std::sync::Arc;

use ::axum::body::Body;
use ::axum::http::{HeaderValue, StatusCode, header};
use ::axum::response::{IntoResponse, Response};
use send_wrapper::SendWrapper;
use tokio::sync::RwLock;
use tokio::task::AbortHandle;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::JsCast;
use web_sys::MessagePort;

use crate::router::ClientId;

const BRIDGE_JS: &str = include_str!("../../assets/bridge.js");

/// `GET /__tonk/bridge.js`.
///
/// Serves the compiled bridge module as a JavaScript ES module.
/// Bytes are bundled into the wasm artefact at build time so the
/// route resolves without touching the network even when the SW
/// is running cold.
pub async fn serve_bridge_js() -> Response {
    let mut response = (StatusCode::OK, Body::from(BRIDGE_JS)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript"),
    );
    response
}

/// One per connected iframe. Owns the transferred `MessagePort`
/// plus abort handles keyed by correlation id — one per in-flight
/// subscribe. `query` and `evaluate` are one-shot and don't need
/// to be tracked.
///
/// `MessagePort` is `!Send + !Sync`; wrap it in `SendWrapper` to
/// satisfy the trait bounds the registry imposes. The SW runtime
/// is single-threaded so SendWrapper's panic-on-cross-thread
/// guard never fires.
pub(crate) struct BridgeSession {
    pub port: SendWrapper<MessagePort>,
    pub subscriptions: HashMap<String, AbortHandle>,
}

impl BridgeSession {
    pub fn new(port: MessagePort) -> Self {
        Self {
            port: SendWrapper::new(port),
            subscriptions: HashMap::new(),
        }
    }
}

/// Per-SW registry: ClientId → BridgeSession. Held behind an
/// `RwLock` so multiple in-flight message-dispatch tasks can read
/// the port concurrently and only contend when adding/removing
/// sessions.
pub type BridgeRegistry = Arc<RwLock<HashMap<ClientId, BridgeSession>>>;

/// Top-level dispatcher invoked from `worker::on_message`. Routes
/// the envelope to the right per-type handler.
///
/// Increment 1 ships `hello` (port handoff) only. The other types
/// log "not yet implemented" — they'll be filled in by follow-up
/// tasks in the same PR.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn handle_message(
    state: crate::router::AppState,
    client: ClientId,
    envelope: serde_json::Value,
    ports: js_sys::Array,
) {
    let envelope_type = envelope
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match envelope_type {
        "hello" => handle_hello(state, client, ports).await,
        "query" | "subscribe" | "unsubscribe" | "evaluate" => {
            log!("bridge: envelope type '{envelope_type}' is not yet wired");
        }
        other => {
            log!("bridge: ignoring envelope type '{other}'");
        }
    }
}

/// `hello` handler. Extracts the transferred port (index 0 of the
/// `ports` array), registers a `BridgeSession` against the client
/// id, and posts a `ready` envelope back so the iframe-side
/// `tonk.ready` promise resolves.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_hello(
    state: crate::router::AppState,
    client: ClientId,
    ports: js_sys::Array,
) {
    if ports.length() == 0 {
        log!("bridge: hello from {client:?} had no transferred port; dropping");
        return;
    }
    let port_value = ports.get(0);
    let port: MessagePort = match port_value.dyn_into() {
        Ok(p) => p,
        Err(_) => {
            log!("bridge: hello from {client:?} transferred a non-MessagePort; dropping");
            return;
        }
    };

    // Stash the session BEFORE posting `ready` so any immediate
    // query/subscribe that follows finds the binding.
    let bridges = state.read().await.bridges.clone();
    {
        let mut guard = bridges.write().await;
        guard.insert(client.clone(), BridgeSession::new(port.clone()));
    }

    let envelope = serde_json::json!({
        "v": 1,
        "type": "ready",
    });
    let js = match serde_wasm_bindgen::to_value(&envelope) {
        Ok(v) => v,
        Err(e) => {
            log!("bridge: ready envelope serialise failed: {e:?}");
            return;
        }
    };
    if let Err(e) = port.post_message(&js) {
        log!("bridge: failed to post ready to {client:?}: {e:?}");
    }
}
