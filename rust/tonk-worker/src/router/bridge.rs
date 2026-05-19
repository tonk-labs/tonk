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
        "query" => handle_query(state, client, envelope).await,
        "evaluate" => handle_evaluate(state, client, envelope).await,
        "subscribe" | "unsubscribe" => {
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

/// `query` handler. Looks up the client's `ViewBinding`, runs a
/// one-shot reactor query, and posts `{query-result, id, rows}` (or
/// `{query-error, id, error}`) back over the port.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_query(
    state: crate::router::AppState,
    client: ClientId,
    envelope: serde_json::Value,
) {
    let id = envelope
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    let body = match envelope.get("body") {
        Some(b) => b.clone(),
        None => {
            send_error(&state, &client, "query-error", &id, "missing body").await;
            return;
        }
    };

    let Some(binding) = lookup_binding(&state, &client).await else {
        send_error(&state, &client, "query-error", &id, "no view binding").await;
        return;
    };

    let wire: crate::reactor::Query = match serde_json::from_value(body) {
        Ok(w) => w,
        Err(e) => {
            send_error(
                &state,
                &client,
                "query-error",
                &id,
                &format!("invalid ConceptQuery: {e}"),
            )
            .await;
            return;
        }
    };
    let query: dialog_query::ConceptQuery = wire.into();

    // Run the one-shot path — same as `router::query::query` does
    // when the request did not ask for a stream.
    let conclusions = {
        let tonk = state.read().await;
        let session = match tonk
            .reactor
            .repository(&binding.repo)
            .branch(&binding.branch)
            .acquire(&tonk.operator)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                send_error(
                    &state,
                    &client,
                    "query-error",
                    &id,
                    &format!("reactor acquire: {e}"),
                )
                .await;
                return;
            }
        };
        let terms = query.terms.clone();
        use dialog_query::Output as _;
        match session
            .handle()
            .select(tonk_schema::concept::QueryPlan::from(query))
            .perform(&tonk.operator)
            .try_vec()
            .await
        {
            Ok(rows) => rows
                .iter()
                .map(|c| crate::reactor::Conclusion::project(c, &terms))
                .collect::<Vec<_>>(),
            Err(e) => {
                send_error(
                    &state,
                    &client,
                    "query-error",
                    &id,
                    &format!("query exec: {e}"),
                )
                .await;
                return;
            }
        }
    };

    send_envelope(
        &state,
        &client,
        serde_json::json!({
            "v": 1,
            "type": "query-result",
            "id": id,
            "rows": conclusions,
        }),
    )
    .await;
}

/// `evaluate` handler. Looks up the client's `ViewBinding`, runs
/// the evaluate pipeline against the supplied body, and posts
/// `{evaluate-result, id, result}` (or `{evaluate-error, id, error}`).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_evaluate(
    state: crate::router::AppState,
    client: ClientId,
    envelope: serde_json::Value,
) {
    let id = envelope
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    let body = match envelope.get("body").and_then(|v| v.as_str()) {
        Some(b) => b.to_owned(),
        None => {
            send_error(
                &state,
                &client,
                "evaluate-error",
                &id,
                "missing body string",
            )
            .await;
            return;
        }
    };
    let content_type = envelope
        .get("contentType")
        .and_then(|v| v.as_str())
        .unwrap_or("application/yaml")
        .to_owned();
    let transact = envelope
        .get("transact")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let Some(binding) = lookup_binding(&state, &client).await else {
        send_error(&state, &client, "evaluate-error", &id, "no view binding").await;
        return;
    };

    let result = {
        let tonk = state.read().await;
        crate::router::evaluate::evaluate_body(
            &tonk,
            &binding.repo,
            &binding.branch,
            body,
            &content_type,
            transact,
        )
        .await
    };
    match result {
        Ok(response) => {
            send_envelope(
                &state,
                &client,
                serde_json::json!({
                    "v": 1,
                    "type": "evaluate-result",
                    "id": id,
                    "result": response,
                }),
            )
            .await;
        }
        Err(e) => {
            send_error(&state, &client, "evaluate-error", &id, &format!("{e}")).await;
        }
    }
}

/// Look up the `ViewBinding` for the given client.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn lookup_binding(
    state: &crate::router::AppState,
    client: &ClientId,
) -> Option<crate::router::ViewBinding> {
    let bindings = state.read().await.view_bindings.clone();
    let guard = bindings.read().await;
    guard.get(client).cloned()
}

/// Serialize `envelope` and post it over the client's bridge port.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn send_envelope(
    state: &crate::router::AppState,
    client: &ClientId,
    envelope: serde_json::Value,
) {
    let bridges = state.read().await.bridges.clone();
    let guard = bridges.read().await;
    let Some(session) = guard.get(client) else {
        log!("bridge: tried to send to {client:?} but no session");
        return;
    };
    let js = match serde_wasm_bindgen::to_value(&envelope) {
        Ok(v) => v,
        Err(e) => {
            log!("bridge: envelope serialise failed: {e:?}");
            return;
        }
    };
    if let Err(e) = session.port.post_message(&js) {
        log!("bridge: post_message to {client:?} failed: {e:?}");
    }
}

/// Post an error envelope back to the client.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn send_error(
    state: &crate::router::AppState,
    client: &ClientId,
    error_type: &str,
    id: &str,
    message: &str,
) {
    send_envelope(
        state,
        client,
        serde_json::json!({
            "v": 1,
            "type": error_type,
            "id": id,
            "error": message,
        }),
    )
    .await;
}
