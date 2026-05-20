//! Iframe ↔ service-worker bridge.
//!
//! Handles `message` events from view clients. The iframe loads
//! `/__tonk/bridge.js` (served from dist by the dev server / CDN)
//! and sends `{v:1,type:"hello"}` with a transferred `MessagePort`;
//! the SW enumerates the view's subscription claims, opens reactor
//! subscriptions, and pumps snapshots back over the port.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::sync::atomic::Ordering;

use send_wrapper::SendWrapper;
use tokio::sync::RwLock;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::JsCast;
use web_sys::MessagePort;

use crate::router::ClientId;

/// One per connected iframe. Owns the transferred `MessagePort`
/// plus abort flags keyed by correlation id — one per in-flight
/// subscribe. `query` and `evaluate` are one-shot and don't need
/// to be tracked.
///
/// `MessagePort` is `!Send + !Sync`; wrap it in `SendWrapper` to
/// satisfy the trait bounds the registry imposes. The SW runtime
/// is single-threaded so SendWrapper's panic-on-cross-thread
/// guard never fires.
///
/// Each active subscription is represented by an `Arc<AtomicBool>`
/// abort flag. Setting the flag to `true` causes the pump task to
/// exit on its next iteration. We use this instead of a tokio
/// `AbortHandle` because `wasm_bindgen_futures::spawn_local` (the
/// only spawn primitive available in the WASM service-worker
/// environment) returns `()` rather than a join-handle.
pub struct BridgeSession {
    /// The `MessagePort` end transferred from the iframe's bridge
    /// module on `hello`. Used to send response envelopes back to
    /// the iframe.
    pub port: SendWrapper<MessagePort>,
    /// Active subscriptions keyed by correlation id. Each value is
    /// an abort flag; setting it to `true` causes the pump task to
    /// exit on its next iteration.
    pub subscriptions: HashMap<String, Arc<AtomicBool>>,
    /// Closure attached to `port.onmessage`. Kept here so it stays
    /// alive for the session's lifetime and is dropped (detaching
    /// the listener) when the session is removed from the registry.
    ///
    /// `Closure` is `!Send + !Sync` (holds JS state); `SendWrapper`
    /// makes it acceptable to the single-threaded SW registry.
    pub _on_message:
        SendWrapper<wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>>,
}

impl BridgeSession {
    /// Create a new session wrapping the port and its `onmessage`
    /// closure with no active subscriptions.
    pub fn new(
        port: MessagePort,
        on_message: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>,
    ) -> Self {
        Self {
            port: SendWrapper::new(port),
            subscriptions: HashMap::new(),
            _on_message: SendWrapper::new(on_message),
        }
    }
}

/// Per-SW registry: ClientId → BridgeSession. Held behind an
/// `RwLock` so multiple in-flight message-dispatch tasks can read
/// the port concurrently and only contend when adding/removing
/// sessions.
pub type BridgeRegistry = Arc<RwLock<HashMap<ClientId, BridgeSession>>>;

/// Top-level dispatcher invoked from `worker::on_message` for the SW
/// global `message` event. Only `hello` arrives this way — all
/// subsequent envelopes (`query`, `subscribe`, `evaluate`,
/// `unsubscribe`) travel over the transferred `MessagePort` and are
/// handled by the per-port `onmessage` closure attached in
/// `handle_hello`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn handle_message(
    state: crate::router::AppState,
    client: ClientId,
    envelope: serde_json::Value,
    ports: js_sys::Array,
) {
    let envelope_type = envelope.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match envelope_type {
        "hello" => handle_hello(state, client, ports).await,
        other => {
            log!(
                "bridge: SW global received unexpected envelope type '{other}' from {client:?} \
                 (post-hello messages should go through the port, not the SW global)"
            );
        }
    }
}

/// `hello` handler. Extracts the transferred port (index 0 of the
/// `ports` array), attaches an `onmessage` listener that dispatches
/// subsequent envelopes from the iframe, registers a `BridgeSession`
/// (which keeps the closure alive), and posts a `ready` envelope so
/// the iframe-side `tonk.ready` promise resolves.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_hello(state: crate::router::AppState, client: ClientId, ports: js_sys::Array) {
    use wasm_bindgen::closure::Closure;

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

    // Build the per-port message handler. Each envelope arriving on
    // the port (query / subscribe / evaluate / unsubscribe) is
    // dispatched asynchronously. The closure is stored on
    // BridgeSession to keep it alive; dropping the session drops the
    // closure and detaches the listener.
    let state_for_handler = state.clone();
    let client_for_handler = client.clone();
    let on_message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
        let state = state_for_handler.clone();
        let client = client_for_handler.clone();
        let data = event.data();
        wasm_bindgen_futures::spawn_local(async move {
            let envelope: serde_json::Value = match serde_wasm_bindgen::from_value(data) {
                Ok(v) => v,
                Err(e) => {
                    log!("bridge port: malformed envelope from {client:?}: {e:?}");
                    return;
                }
            };
            let envelope_type = envelope
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match envelope_type {
                "query" => handle_query(state, client, envelope).await,
                "evaluate" => handle_evaluate(state, client, envelope).await,
                "subscribe" => handle_subscribe(state, client, envelope).await,
                "unsubscribe" => handle_unsubscribe(state, client, envelope).await,
                "hello" => {
                    log!(
                        "bridge port: ignoring 'hello' on established port from {client:?}"
                    );
                }
                other => {
                    log!("bridge port: ignoring envelope type '{other}' from {client:?}");
                }
            }
        });
    }) as Box<dyn FnMut(web_sys::MessageEvent)>);

    // Attach the listener. Setting onmessage auto-starts the port,
    // so no explicit port.start() call is needed.
    port.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    // Stash the session (holding port + closure) BEFORE posting
    // `ready` so any immediate query/subscribe that follows finds
    // the binding.
    let bridges = state.read().await.bridges.clone();
    {
        let mut guard = bridges.write().await;
        guard.insert(client.clone(), BridgeSession::new(port.clone(), on_message));
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

/// `subscribe` handler. Opens a reactor subscription for the
/// client's bound branch, spawns a pump task that posts each
/// emission as `subscribe-event` over the port. A shared
/// `Arc<AtomicBool>` abort flag is stored against
/// `BridgeSession.subscriptions` keyed by the correlation `id` so
/// `handle_unsubscribe` can stop the pump.
///
/// Re-subscribing with the same `id` is treated as a programmer
/// error: we log and return an error envelope without disturbing
/// the existing pump. The client should `unsubscribe` first.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_subscribe(
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
            send_error(&state, &client, "subscribe-error", &id, "missing body").await;
            return;
        }
    };

    let Some(binding) = lookup_binding(&state, &client).await else {
        send_error(&state, &client, "subscribe-error", &id, "no view binding").await;
        return;
    };

    let wire: crate::reactor::Query = match serde_json::from_value(body) {
        Ok(w) => w,
        Err(e) => {
            send_error(
                &state,
                &client,
                "subscribe-error",
                &id,
                &format!("invalid ConceptQuery: {e}"),
            )
            .await;
            return;
        }
    };
    let query: dialog_query::ConceptQuery = wire.into();

    // Bail early if this id is already pumping.
    {
        let bridges = state.read().await.bridges.clone();
        let guard = bridges.read().await;
        if let Some(session) = guard.get(&client) {
            if session.subscriptions.contains_key(&id) {
                log!("bridge: subscribe id '{id}' already active for {client:?}");
                drop(guard);
                send_error(&state, &client, "subscribe-error", &id, "id already in use").await;
                return;
            }
        }
    }

    let subscriber = {
        let tonk = state.read().await;
        match tonk
            .reactor
            .repository(&binding.repo)
            .branch(&binding.branch)
            .subscribe(query)
            .perform(&tonk.operator)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                send_error(
                    &state,
                    &client,
                    "subscribe-error",
                    &id,
                    &format!("reactor subscribe: {e}"),
                )
                .await;
                return;
            }
        }
    };

    let mut receiver = subscriber.receiver;
    let pump_state = state.clone();
    let pump_client = client.clone();
    let pump_id = id.clone();

    let abort_flag = Arc::new(AtomicBool::new(false));
    let pump_flag = abort_flag.clone();

    wasm_bindgen_futures::spawn_local(async move {
        while !pump_flag.load(Ordering::Acquire) {
            let Some(bytes) = receiver.recv().await else {
                break;
            };
            if pump_flag.load(Ordering::Acquire) {
                break;
            }
            // The reactor emits a JSON-serialised `Vec<Conclusion>`
            // per frame. Decode and re-wrap as a subscribe-event.
            let rows: serde_json::Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    log!("bridge: subscribe frame for id '{pump_id}' was not JSON: {e}");
                    continue;
                }
            };
            send_envelope(
                &pump_state,
                &pump_client,
                serde_json::json!({
                    "v": 1,
                    "type": "subscribe-event",
                    "id": pump_id,
                    "rows": rows,
                }),
            )
            .await;
        }
    });

    let bridges = state.read().await.bridges.clone();
    let mut guard = bridges.write().await;
    if let Some(session) = guard.get_mut(&client) {
        session.subscriptions.insert(id, abort_flag);
    }
}

/// `unsubscribe` handler. Sets the pump task's abort flag and
/// removes the subscription entry. Idempotent — calling it with
/// an unknown id is a no-op (no error envelope sent back).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn handle_unsubscribe(
    state: crate::router::AppState,
    client: ClientId,
    envelope: serde_json::Value,
) {
    let id = envelope
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_default();

    let bridges = state.read().await.bridges.clone();
    let mut guard = bridges.write().await;
    let Some(session) = guard.get_mut(&client) else {
        return;
    };
    if let Some(flag) = session.subscriptions.remove(&id) {
        flag.store(true, Ordering::Release);
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

/// Walk every entry in `BridgeRegistry` and `ViewBindings`, drop any
/// whose client id isn't in the SW's `matchAll()` live set.
///
/// Best-effort — runs opportunistically from `on_fetch` so idle SWs
/// may accumulate stale entries briefly.
///
/// For each dropped `BridgeSession`, abort flags are set so pump
/// tasks exit on their next iteration.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub async fn sweep_stale_clients(state: &crate::router::AppState) {
    use wasm_bindgen::JsCast;

    let global: web_sys::ServiceWorkerGlobalScope = match js_sys::global().dyn_into() {
        Ok(g) => g,
        Err(_) => {
            log!("sweep: not in a service worker scope");
            return;
        }
    };
    let live_promise = global.clients().match_all();
    let live_value = match wasm_bindgen_futures::JsFuture::from(live_promise).await {
        Ok(v) => v,
        Err(e) => {
            log!("sweep: clients.matchAll failed: {e:?}");
            return;
        }
    };
    let live_array = js_sys::Array::from(&live_value);
    let live_ids: std::collections::HashSet<String> = live_array
        .iter()
        .filter_map(|v| v.dyn_into::<web_sys::Client>().ok().map(|c| c.id()))
        .collect();

    let (stale_bridges, stale_views) = {
        let snap = state.read().await;
        let bridges_arc = snap.bridges.clone();
        let views_arc = snap.view_bindings.clone();
        drop(snap);

        let stale_bridges: Vec<ClientId> = {
            let guard = bridges_arc.read().await;
            guard
                .keys()
                .filter(|c| !live_ids.contains(&c.0))
                .cloned()
                .collect()
        };
        let stale_views: Vec<ClientId> = {
            let guard = views_arc.read().await;
            guard
                .keys()
                .filter(|c| !live_ids.contains(&c.0))
                .cloned()
                .collect()
        };
        (stale_bridges, stale_views)
    };

    if stale_bridges.is_empty() && stale_views.is_empty() {
        return;
    }

    let bridges_arc = state.read().await.bridges.clone();
    let views_arc = state.read().await.view_bindings.clone();

    {
        let mut guard = bridges_arc.write().await;
        for client in &stale_bridges {
            if let Some(session) = guard.remove(client) {
                for (_id, flag) in session.subscriptions.iter() {
                    flag.store(true, Ordering::Release);
                }
                // session.port drops here.
            }
        }
    }
    {
        let mut guard = views_arc.write().await;
        for client in &stale_views {
            guard.remove(client);
        }
    }

    log!(
        "sweep: dropped {} bridge sessions, {} view bindings",
        stale_bridges.len(),
        stale_views.len(),
    );
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
