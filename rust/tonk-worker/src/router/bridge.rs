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
