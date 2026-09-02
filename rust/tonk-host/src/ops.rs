//! Operation event listeners the installed host attaches to
//! `document`.
//!
//! Five events are handled here:
//!
//! - `tonk-query` / `tonk-claim` / `tonk-evaluate` — one-shots;
//!   the handler writes a `Promise` into `detail.result`.
//! - `tonk-subscribe` — streaming; the handler opens the SSE,
//!   inserts a registry entry, writes a subscription handle
//!   into `detail.subscription`, and delivers each frame via
//!   `consumer.reset(...)` (a `Snapshot`) or `consumer.update(...)`
//!   (a `Delta`) by the frame's `kind` (see `deliver_frame`).
//! - `tonk-unsubscribe` — drops all registry entries owned by
//!   the dispatching consumer.
//!
//! Routing context comes from the event detail when a caller
//! pre-filled an explicit route (`consumer::*_with_route`), else
//! from the dispatching element's OWN `with` attribute
//! ([`crate::context::resolve_with`]) at handle time, else the
//! guest's pinned site context.
//!
//! Each listener calls `event.stopPropagation()` so events do
//! not escape the host, and `event.preventDefault()` so the
//! dispatcher knows a provider handled the event (the consumer
//! checks `event.defaultPrevented` after dispatch).

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{ErrorDetail, ErrorKind};
use crate::sse::open_sse;
use ipld_core::ipld::Ipld;
use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{future_to_promise, spawn_local};
use web_sys::{CustomEvent, Element, EventTarget};

use crate::events;
use crate::host::HostState;
use crate::http;
use crate::registry::{Entry, EntryId};
use crate::url::{evaluate_url, query_url, transact_url};

/// One installed listener — the closure is held so it stays
/// attached for the page's lifetime.
pub(crate) struct InstalledListener {
    _closure: Closure<dyn FnMut(CustomEvent)>,
}

/// Attach the host's listeners for every event the protocol
/// defines to `target` (the document). Returns the handles so the
/// caller keeps the closures alive.
pub(crate) fn attach_all(
    target: &EventTarget,
    state: Rc<RefCell<HostState>>,
) -> Vec<InstalledListener> {
    let mut listeners = Vec::new();
    listeners.push(install_listener(target, events::QUERY, {
        move |ev| handle_query(&ev)
    }));
    listeners.push(install_listener(target, events::CLAIM, {
        move |ev| handle_claim(&ev)
    }));
    listeners.push(install_listener(target, events::EVALUATE, {
        move |ev| handle_evaluate(&ev)
    }));
    listeners.push(install_listener(target, events::SUBSCRIBE, {
        let state = state.clone();
        move |ev| handle_subscribe(&ev, &state)
    }));
    listeners.push(install_listener(target, events::UNSUBSCRIBE, {
        move |ev| handle_unsubscribe(&ev, &state)
    }));
    listeners
}

/// Helper used by every listener: stop propagation and mark the
/// event as handled by a provider.
fn claim_event(ev: &CustomEvent) {
    ev.stop_propagation();
    ev.prevent_default();
}

/// Delay before reconnecting a cleanly closed stream: 2s plus up to 3s of
/// per-subscription jitter. The common cause is the SW releasing every
/// in-flight stream at once on update — an immediate, synchronized
/// reconnect re-pins the OLD worker with a fresh wave of streams before
/// the replacement can take over, and the release/reconnect cycle churns
/// until the renderer gives out. The jitter breaks the herd; the base
/// delay gives the waiting worker room to activate.
fn retry_close_ms() -> i32 {
    2_000 + (js_sys::Math::random() * 3_000.0) as i32
}

/// Delay before retrying after a transport error (SW restarting, network
/// blip): 3s plus up to 3s of jitter, longer than the clean-close delay so
/// a hard-down worker isn't hammered.
fn retry_error_ms() -> i32 {
    3_000 + (js_sys::Math::random() * 3_000.0) as i32
}

/// The control frame an intentional drop sends before closing — the
/// worker is about to be replaced; hold the reconnect for the controller
/// change instead of dialing the outgoing worker on the short timer.
const UPDATE_PENDING_FRAME: &str = r#"{"control":"update-pending"}"#;

/// Long-fallback reconnect delay for a held entry: the top page reconnects
/// everything on `controllerchange`, so this only backstops contexts that
/// cannot observe it (sealed guests) or a takeover that never lands.
fn retry_held_ms() -> i32 {
    20_000 + (js_sys::Math::random() * 10_000.0) as i32
}

/// Handle a control frame on a subscription stream. Returns `true` when the
/// frame was a control message (consumed here, not delivered).
fn handle_control_frame(state: &Rc<RefCell<HostState>>, entry_id: EntryId, frame: &str) -> bool {
    if frame.trim() != UPDATE_PENDING_FRAME {
        return false;
    }
    let mut s = state.borrow_mut();
    if let Some(entry) = s.registry.entries_mut().get_mut(&entry_id) {
        entry.awaiting_controller = true;
    }
    true
}

/// The close-side reconnect delay for `entry_id`: held entries wait the
/// long fallback, ordinary closes the short jittered one.
fn close_delay_ms(state: &Rc<RefCell<HostState>>, entry_id: EntryId) -> i32 {
    let held = state
        .borrow()
        .registry
        .get(entry_id)
        .is_some_and(|entry| entry.awaiting_controller);
    if held {
        retry_held_ms()
    } else {
        retry_close_ms()
    }
}

/// Re-issue the subscription for `entry_id` after `delay_ms`, via the same
/// re-resolution path a context refresh uses — so the reconnect picks up
/// the consumer's current `with` ancestry. A cancelled entry or a
/// disconnected consumer ends the retry chain (`refresh_entry` prunes it).
fn schedule_resubscribe(state: &Rc<RefCell<HostState>>, entry_id: EntryId, delay_ms: i32) {
    let state = state.clone();
    spawn_local(async move {
        wait_ms(delay_ms).await;
        refresh_entry(&state, entry_id).await;
    });
}

/// Re-issue after a transport error.
///
/// Every retry path funnels through here rather than deciding
/// individually — the initial `open_sse` rejection, the mid-stream error
/// callback, and the refresh path each hit a different arm.
///
/// There is no "give up" case. An absent repository or branch is no
/// longer an error at all: the worker answers it with an open stream
/// carrying the empty set and parks the subscription in its waiting
/// room, so a repo that arrives later delivers into the stream the page
/// already holds. An earlier version of this retired the entry on a
/// `404`, which destroyed the subscription outright — a space joined in
/// another tab could never reach a tab already sitting on the page,
/// because nothing was left listening.
fn resubscribe_after_error(
    state: &Rc<RefCell<HostState>>,
    entry_id: EntryId,
    _error: &ErrorDetail,
) {
    schedule_resubscribe(state, entry_id, retry_error_ms());
}

/// Re-issue EVERY live subscription. Called on `controllerchange`: a new
/// service worker just took over, the old worker's streams are gone (or
/// serving their final frame), and an immediate refresh beats waiting out
/// each entry's jittered retry timer — the page heals in one pass instead
/// of trickling back over seconds.
pub(crate) fn refresh_all(state: &Rc<RefCell<HostState>>) {
    let ids = state.borrow().registry.ids();
    let state = state.clone();
    spawn_local(async move {
        for id in ids {
            yield_microtask().await;
            refresh_entry(&state, id).await;
        }
    });
}

/// Sleep `ms` milliseconds via `setTimeout`.
pub(crate) async fn wait_ms(ms: i32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        if let Some(win) = web_sys::window() {
            let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Install one event listener on `target` for `name`. Returns the
/// `InstalledListener` so the caller can keep it alive.
fn install_listener<F>(
    target: &EventTarget,
    name: &'static str,
    mut handler: F,
) -> InstalledListener
where
    F: FnMut(CustomEvent) + 'static,
{
    let closure =
        Closure::wrap(Box::new(move |ev: CustomEvent| handler(ev)) as Box<dyn FnMut(CustomEvent)>);
    let _ = target.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
    InstalledListener { _closure: closure }
}

/// The element that actually dispatched the event.
///
/// NOT `ev.target()`. These events are `composed`, so they escape a shadow
/// root — and crossing that boundary RETARGETS `target` to the shadow host.
/// A `<tonk-display>` inside `<tonk-prose>`'s shadow root therefore arrives
/// here reported as `<tonk-prose>`, and every frame this handler routes back
/// by calling `reset` on the "consumer" lands on the host element, which has
/// no such method. The display stays at `data-state="loading"` forever while
/// its data is delivered to something that drops it on the floor.
///
/// `composedPath()[0]` is the un-retargeted origin, so it names the real
/// consumer on both sides of a shadow boundary. Falls back to `target` for
/// an event whose path is empty (one already finished dispatching).
fn event_origin(ev: &CustomEvent) -> Option<Element> {
    let path = ev.composed_path();
    if path.length() > 0
        && let Ok(element) = path.get(0).dyn_into::<Element>()
    {
        return Some(element);
    }
    ev.target().and_then(|t| t.dyn_into::<Element>().ok())
}

/// Resolve the `(space, branch, profile)` route for an operation:
/// an explicit route pre-filled on the detail (by
/// `consumer::*_with_route`) wins; otherwise the dispatching
/// element's nearest `with` ancestor decides; otherwise, inside a
/// sealed guest, the portal's pinned context from the bridge
/// (`window.tonk.context.with`). A malformed `with` attribute is a
/// parse error the caller surfaces to the consumer.
///
/// A `branch` with no `space` outside profile mode is rejected: the
/// repository segment is required and there is no default space to
/// fill it with, so honoring it would silently target the wrong repo.
/// `consumer::apply_route` never emits that shape; hand-authored JS
/// dispatching a raw `detail` can, and gets a parse error.
fn route_from(
    detail: &Object,
    origin: Option<&Element>,
) -> Result<(Option<String>, Option<String>, bool), ErrorDetail> {
    let space = get_string(detail, "space");
    let branch = get_string(detail, "branch");
    let profile = get_bool(detail, "profile");
    if branch.is_some() && space.is_none() && !profile {
        return Err(ErrorDetail::new(
            ErrorKind::Parse,
            "detail.branch requires detail.space (or detail.profile)",
        ));
    }
    // Explicit route on the detail: a repository or the profile flag
    // (both produced by `apply_route`).
    if space.is_some() || profile {
        return Ok((space, branch, profile));
    }
    let Some(origin) = origin else {
        return Ok((None, None, false));
    };
    ambient_route(origin)
}

/// The route implied by an element's surroundings: its nearest `with`
/// ancestor, else the enclosing portal's pinned context (delivered by
/// the bridge as `context.with`), else nothing (the bare endpoint).
fn ambient_route(origin: &Element) -> Result<(Option<String>, Option<String>, bool), ErrorDetail> {
    match crate::context::resolve_with(origin) {
        Ok(Some(location)) => return Ok(crate::context::route_of(&location)),
        Ok(None) => {}
        Err(error) => {
            return Err(ErrorDetail::new(
                ErrorKind::Parse,
                format!("with attribute: {error}"),
            ));
        }
    }
    if let Some(pinned) = crate::bridge::context_field("with")
        && let Ok(location) = pinned.parse::<crate::location::Location>()
    {
        return Ok(crate::context::route_of(&location));
    }
    Ok((None, None, false))
}

/// `tonk-query` handler.
///
/// Resolves the route, POSTs `detail.query` to the structured-query
/// endpoint, and resolves the `Vec<Conclusion>` into `detail.result`.
fn handle_query(ev: &CustomEvent) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return, // No detail object — caller error; nothing we can do.
    };

    let (space, branch, profile) = match route_from(&detail, event_origin(ev).as_ref()) {
        Ok(route) => route,
        Err(error) => return install_rejected_promise(&detail, error),
    };
    let url = query_url(space.as_deref(), branch.as_deref(), profile);

    let query_val = match Reflect::get(&detail, &JsValue::from_str("query")) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v,
        _ => {
            install_rejected_promise(
                &detail,
                ErrorDetail::new(ErrorKind::Parse, "tonk-query: missing detail.query"),
            );
            return;
        }
    };
    let body_str = match js_to_canonical_dag_json(&query_val) {
        Ok(s) => s,
        Err(e) => {
            install_rejected_promise(&detail, e);
            return;
        }
    };

    let promise = future_to_promise(async move {
        match http::post_json(&url, &body_str).await {
            Ok(json_text) => parse_json_response(&json_text),
            Err(e) => Err(error_to_js(&e)),
        }
    });
    let _ = Reflect::set(&detail, &JsValue::from_str("result"), &promise);
}

/// Convenience: build a rejected `Promise` and install it on
/// `detail.result`.
fn install_rejected_promise(detail: &Object, err: ErrorDetail) {
    let js_err = error_to_js(&err);
    let promise = Promise::reject(&js_err);
    let _ = Reflect::set(detail, &JsValue::from_str("result"), &promise);
}

/// Read a string-valued detail field. Returns `None` for missing,
/// null, undefined, non-string, or empty-string.
fn get_string(detail: &Object, key: &str) -> Option<String> {
    let v = Reflect::get(detail, &JsValue::from_str(key)).ok()?;
    let s = v.as_string()?;
    if s.is_empty() { None } else { Some(s) }
}

/// Read a boolean flag from an event detail. Treats a truthy value
/// (`true`, a non-empty string) as set. Used for the `profile`
/// annotation that a `with="…@profile"` context stamps to target
/// the profile-as-repository endpoint.
fn get_bool(detail: &Object, key: &str) -> bool {
    Reflect::get(detail, &JsValue::from_str(key))
        .ok()
        .map(|v| v.is_truthy())
        .unwrap_or(false)
}

/// Serialize a JS value as canonical DAG-JSON text. Goes
/// `JsValue → Ipld` via `serde-wasm-bindgen` (the only
/// adapter that knows how to project JS `Map`s and typed
/// arrays into the IPLD data model), then `Ipld → bytes`
/// via `serde_ipld_dagjson`. DAG-JSON sorts map keys
/// deterministically and has a stable number/string
/// encoding, so two semantically equal queries always
/// produce the same text.
fn js_to_canonical_dag_json(v: &JsValue) -> Result<String, ErrorDetail> {
    let value: Ipld = serde_wasm_bindgen::from_value(v.clone())
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("detail.query: {e}")))?;
    let bytes = serde_ipld_dagjson::to_vec(&value)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("dag-json: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|e| ErrorDetail::new(ErrorKind::Parse, format!("dag-json utf8: {e}")))
}

/// Parse the worker's JSON response body into a `Vec<Conclusion>`
/// and return it as a JS value the consumer can read.
fn parse_json_response(text: &str) -> Result<JsValue, JsValue> {
    js_sys::JSON::parse(text).map_err(|_| {
        error_to_js(&ErrorDetail::new(
            ErrorKind::Parse,
            "tonk-query: response not JSON",
        ))
    })
}

/// Serialize an `ErrorDetail` into a JS value the caller can
/// `catch`. Uses the `serde-wasm-bindgen` adapter.
fn error_to_js(err: &ErrorDetail) -> JsValue {
    serde_wasm_bindgen::to_value(err).unwrap_or(JsValue::NULL)
}

/// `tonk-claim` handler.
///
/// Resolves the route, POSTs the structured `TransactRequest` to
/// the `/transact` endpoint, resolves the parsed response into
/// `detail.result`.
fn handle_claim(ev: &CustomEvent) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };

    let (space, branch, profile) = match route_from(&detail, event_origin(ev).as_ref()) {
        Ok(route) => route,
        Err(error) => return install_rejected_promise(&detail, error),
    };
    let url = transact_url(space.as_deref(), branch.as_deref(), profile);

    let request_val = match Reflect::get(&detail, &JsValue::from_str("request")) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v,
        _ => {
            install_rejected_promise(
                &detail,
                ErrorDetail::new(ErrorKind::Parse, "tonk-claim: missing detail.request"),
            );
            return;
        }
    };
    let body_str = match js_to_canonical_dag_json(&request_val) {
        Ok(s) => s,
        Err(e) => {
            install_rejected_promise(&detail, e);
            return;
        }
    };

    let promise = future_to_promise(async move {
        match http::post_json(&url, &body_str).await {
            Ok(json_text) => parse_json_response(&json_text),
            Err(e) => Err(error_to_js(&e)),
        }
    });
    let _ = Reflect::set(&detail, &JsValue::from_str("result"), &promise);
}

/// `tonk-subscribe` handler.
///
/// Resolves the route, inserts an entry in the registry, opens an
/// upstream SSE, routes each frame to the consumer's
/// `reset(conclusions, opts)` method (v1 SW emits only `reset`).
/// Writes `{ cancel }` into `detail.subscription` for caller-side
/// teardown.
fn handle_subscribe(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };
    let consumer = match event_origin(ev) {
        Some(el) => el,
        None => return,
    };

    let tag_js = Reflect::get(&detail, &JsValue::from_str("tag"))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null());

    let (space, branch, profile) = match route_from(&detail, Some(&consumer)) {
        Ok(route) => route,
        Err(error) => {
            // No promise on a subscribe — surface the malformed `with` via
            // the consumer's error method instead.
            invoke_method(&consumer, "error", &error_to_js(&error), tag_js.as_ref());
            return;
        }
    };
    // A routeless subscription (no own `with`, no pinned context) would hit
    // the bare `/query` endpoint — a 404 the reconnect retries forever.
    // Refuse it: the consumer needs a routing context. (One-shots may
    // legitimately hit the bare endpoint from the top page; a live
    // subscription that reconnects must not.)
    if space.is_none() && branch.is_none() && !profile {
        invoke_method(
            &consumer,
            "error",
            &error_to_js(&ErrorDetail::new(
                ErrorKind::Network,
                format!(
                    "tonk-subscribe: no routing context for <{}> (set a with= attribute)",
                    consumer.local_name()
                ),
            )),
            tag_js.as_ref(),
        );
        return;
    }
    let url = query_url(space.as_deref(), branch.as_deref(), profile);

    let query_val = match Reflect::get(&detail, &JsValue::from_str("query")) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v,
        _ => return,
    };
    let query_ipld: Ipld = match serde_wasm_bindgen::from_value(query_val.clone()) {
        Ok(v) => v,
        Err(_) => return,
    };
    let depth = Reflect::get(&detail, &JsValue::from_str("depth"))
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as u32;

    let entry_id = {
        let mut s = state.borrow_mut();
        s.registry.insert(Entry {
            consumer: consumer.clone(),
            space: space.clone(),
            branch: branch.clone(),
            query: query_val.clone(),
            tag: tag_js.clone(),
            depth,
            abort: None,
            awaiting_controller: false,
        })
    };

    // Spawn the SSE open. When the future completes, install the
    // abort handle on the registry entry (or drop it if the entry
    // has been removed in the meantime).
    let state_for_spawn = state.clone();
    let consumer_for_spawn = consumer.clone();
    let tag_for_spawn = tag_js.clone();
    spawn_local(async move {
        let body = query_ipld;
        let consumer_frame = consumer_for_spawn.clone();
        let tag_frame = tag_for_spawn.clone();
        let state_frame = state_for_spawn.clone();
        let consumer_err = consumer_for_spawn.clone();
        let tag_err = tag_for_spawn.clone();
        let state_err = state_for_spawn.clone();
        let state_close = state_for_spawn.clone();
        let abort = open_sse(
            &url,
            &body,
            move |frame: &str| {
                if handle_control_frame(&state_frame, entry_id, frame) {
                    return;
                }
                if !consumer_frame.is_connected() {
                    return;
                }
                // `JSON.parse`, NOT a serde round-trip: the frame is already
                // the JSON text of `Vec<Conclusion>`, and consumers read the
                // result by property access (`conclusion.fields.status`).
                // `serde_wasm_bindgen` would serialize the `fields` map as a
                // JS `Map`, whose entries `Reflect::get` cannot see.
                let frame_js = match js_sys::JSON::parse(frame) {
                    Ok(v) => v,
                    Err(e) => {
                        invoke_method(
                            &consumer_frame,
                            "error",
                            &error_to_js(&ErrorDetail::new(
                                ErrorKind::Parse,
                                format!("subscribe frame: {e:?}"),
                            )),
                            tag_frame.as_ref(),
                        );
                        return;
                    }
                };
                deliver_frame(&consumer_frame, &frame_js, tag_frame.as_ref(), false);
            },
            move |err: ErrorDetail| {
                if consumer_err.is_connected() {
                    invoke_method(&consumer_err, "error", &error_to_js(&err), tag_err.as_ref());
                }
                // Keep the entry and retry: a transport error usually means
                // the SW is restarting/updating, and a successful reconnect
                // heals the consumer with its next `reset`.
                resubscribe_after_error(&state_err, entry_id, &err);
            },
            move || {
                // Clean server close (the SW releasing in-flight streams on
                // update): reconnect silently — the subscription isn't over,
                // its transport is. A close announced as INTENTIONAL holds
                // for the controller change instead.
                let delay = close_delay_ms(&state_close, entry_id);
                schedule_resubscribe(&state_close, entry_id, delay);
            },
        )
        .await;
        match abort {
            Ok(handle) => {
                let mut s = state_for_spawn.borrow_mut();
                s.registry.install_abort(entry_id, handle);
            }
            Err(err) => {
                if consumer_for_spawn.is_connected() {
                    invoke_method(
                        &consumer_for_spawn,
                        "error",
                        &error_to_js(&err),
                        tag_for_spawn.as_ref(),
                    );
                }
                // The 404 path: a missing repo/branch fails the initial
                // POST, so the stream never opens and `on_error` above
                // never runs — this arm is the one that looped.
                resubscribe_after_error(&state_for_spawn, entry_id, &err);
            }
        }
    });

    install_subscription_handle(&detail, state.clone(), entry_id);
}

/// Build a `{ cancel: () => host.cancel(entryId) }` JS object and
/// install it as `detail.subscription`.
fn install_subscription_handle(detail: &Object, state: Rc<RefCell<HostState>>, entry_id: EntryId) {
    let cancel_closure: Closure<dyn FnMut()> = Closure::wrap(Box::new(move || {
        let mut s = state.borrow_mut();
        s.registry.remove(entry_id);
    }) as Box<dyn FnMut()>);
    let cancel_fn: Function = cancel_closure.into_js_value().unchecked_into::<Function>();

    let sub = Object::new();
    let _ = Reflect::set(&sub, &JsValue::from_str("cancel"), &cancel_fn);
    let _ = Reflect::set(detail, &JsValue::from_str("subscription"), &sub);
}

/// Refresh every subscription whose consumer sits under `target` —
/// called by the host's `with` observer when a routing context
/// changes. The affected entries are grouped by recorded `depth`
/// and re-issued shallowest first.
///
/// Between depth groups, the refresh yields to the microtask
/// queue so the synchronous iteration diffs the consumer
/// triggers (in its `reset`) have a chance to fire
/// `disconnectedCallback` → `tonk-unsubscribe`, which prunes
/// doomed entries from the registry before the next depth runs.
pub(crate) fn refresh_under(state: &Rc<RefCell<HostState>>, target: &Element) {
    // Snapshot the affected ids grouped by depth. Holding the
    // borrow only for this scan keeps it cheap.
    let mut by_depth: std::collections::BTreeMap<u32, Vec<crate::registry::EntryId>> =
        std::collections::BTreeMap::new();
    {
        let s = state.borrow();
        for id in s.registry.ids_under(target) {
            if let Some(entry) = s.registry.get(id) {
                by_depth.entry(entry.depth).or_default().push(id);
            }
        }
    }
    if by_depth.is_empty() {
        return;
    }

    let state_for_refresh = state.clone();
    spawn_local(async move {
        for (_depth, ids) in by_depth {
            // Yield to the microtask queue before each depth so
            // any iteration-diff-driven detachments from the
            // previous depth's `reset` calls have fired their
            // `disconnectedCallback` → `tonk-unsubscribe`, and
            // their registry entries are gone.
            yield_microtask().await;
            for id in ids {
                refresh_entry(&state_for_refresh, id).await;
            }
        }
    });
}

/// Yield once to the microtask queue. Allows synchronous
/// reconciliation work in the consumer's `reset` to complete
/// (including `disconnectedCallback` for detached children)
/// before the next depth's refresh runs.
async fn yield_microtask() {
    let promise = Promise::resolve(&JsValue::UNDEFINED);
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Re-issue the subscription for `entry_id` against the current
/// context, re-resolving the consumer's `with` ancestry. If the
/// consumer is no longer in the DOM, drops the entry without
/// re-issuing.
async fn refresh_entry(state: &Rc<RefCell<HostState>>, entry_id: crate::registry::EntryId) {
    // Snapshot what we need under one borrow.
    let snapshot = {
        let s = state.borrow();
        s.registry
            .get(entry_id)
            .map(|e| (e.consumer.clone(), e.query.clone(), e.tag.clone()))
    };
    let Some((consumer, query, tag)) = snapshot else {
        return;
    };
    if !consumer.is_connected() {
        let mut s = state.borrow_mut();
        s.registry.remove(entry_id);
        return;
    }
    // Read fresh context from the consumer's own `with`, else the portal's
    // pinned context.
    let (space, branch, profile) = match ambient_route(&consumer) {
        Ok(route) => route,
        Err(error) => {
            invoke_method(&consumer, "error", &error_to_js(&error), tag.as_ref());
            let mut s = state.borrow_mut();
            s.registry.remove(entry_id);
            return;
        }
    };
    // A routeless reconnect would hit the bare `/query` endpoint and fail;
    // drop the entry instead of retrying forever (the context vanished —
    // e.g. a `with` attribute was removed). See `handle_subscribe`.
    if space.is_none() && branch.is_none() && !profile {
        invoke_method(
            &consumer,
            "error",
            &error_to_js(&ErrorDetail::new(
                ErrorKind::Network,
                "tonk-subscribe: routing context lost on reconnect",
            )),
            tag.as_ref(),
        );
        let mut s = state.borrow_mut();
        s.registry.remove(entry_id);
        return;
    }
    // Abort the existing upstream and clear its handle so the
    // refresh's new subscription is the only live one.
    let url = query_url(space.as_deref(), branch.as_deref(), profile);
    {
        let mut s = state.borrow_mut();
        if s.registry.get(entry_id).is_some() {
            if let Some(e) = s.registry.entries_mut().get_mut(&entry_id) {
                e.abort.take();
                e.space = space.clone();
                e.branch = branch.clone();
                // A fresh issue starts unheld; a new intentional-drop
                // signal re-marks it.
                e.awaiting_controller = false;
            }
        } else {
            return;
        }
    }

    // Re-open the SSE. Same shape as `handle_subscribe` but
    // against the new url, reusing the stored query + tag.
    let body_ipld: Ipld = match serde_wasm_bindgen::from_value(query.clone()) {
        Ok(v) => v,
        Err(_) => return,
    };
    let consumer_frame = consumer.clone();
    let tag_frame = tag.clone();
    let state_err = state.clone();
    let state_close = state.clone();
    let state_frame = state.clone();
    let consumer_err = consumer.clone();
    let tag_err = tag.clone();
    // The first frame after this reopen carries `reconnect: true` — see
    // `invoke_method_marked`.
    let first_frame = std::cell::Cell::new(true);
    let abort_result = open_sse(
        &url,
        &body_ipld,
        move |frame: &str| {
            if handle_control_frame(&state_frame, entry_id, frame) {
                return;
            }
            if !consumer_frame.is_connected() {
                return;
            }
            // `JSON.parse` for plain objects — see the subscribe handler.
            let frame_js = match js_sys::JSON::parse(frame) {
                Ok(v) => v,
                Err(e) => {
                    invoke_method(
                        &consumer_frame,
                        "error",
                        &error_to_js(&ErrorDetail::new(
                            ErrorKind::Parse,
                            format!("refresh frame: {e:?}"),
                        )),
                        tag_frame.as_ref(),
                    );
                    return;
                }
            };
            deliver_frame(
                &consumer_frame,
                &frame_js,
                tag_frame.as_ref(),
                first_frame.replace(false),
            );
        },
        move |err: ErrorDetail| {
            if consumer_err.is_connected() {
                invoke_method(&consumer_err, "error", &error_to_js(&err), tag_err.as_ref());
            }
            resubscribe_after_error(&state_err, entry_id, &err);
        },
        move || {
            let delay = close_delay_ms(&state_close, entry_id);
            schedule_resubscribe(&state_close, entry_id, delay);
        },
    )
    .await;
    match abort_result {
        Ok(handle) => {
            let mut s = state.borrow_mut();
            s.registry.install_abort(entry_id, handle);
        }
        Err(err) => {
            if consumer.is_connected() {
                invoke_method(&consumer, "error", &error_to_js(&err), tag.as_ref());
            }
            resubscribe_after_error(state, entry_id, &err);
        }
    }
}

/// `tonk-unsubscribe` handler. Drops all registry entries owned
/// by `event.target`.
fn handle_unsubscribe(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let consumer = match event_origin(ev) {
        Some(el) => el,
        None => return,
    };
    let mut s = state.borrow_mut();
    let ids = s.registry.ids_for_consumer(&consumer);
    for id in ids {
        s.registry.remove(id);
    }
}

/// Invoke a method on a consumer element with one positional
/// payload + an `{ tag }` opts object.
fn invoke_method(consumer: &Element, method: &str, payload: &JsValue, tag: Option<&JsValue>) {
    invoke_method_marked(consumer, method, payload, tag, false);
}

/// Route one parsed subscription frame to the consumer's `reset` /
/// `update` method by its `kind`.
///
/// The reactor emits a tagged `Frame` (`tonk_schema::conclusion::Frame`):
/// - `{"kind":"snapshot","conclusions":[…]}` → `reset(conclusions, opts)`
///   — the full current set, for a first frame or a reconnect.
/// - `{"kind":"delta","asserted":[…],"retracted":[…]}` →
///   `update({asserted, retracted}, opts)` — the change since the last
///   frame, applied to the consumer's retained set.
///
/// Backward-compatible: a frame with no `kind` (a bare `[…]` array, the
/// pre-delta wire) is delivered as a `reset` unchanged, so a consumer or
/// worker that hasn't cut over still works.
fn deliver_frame(consumer: &Element, frame_js: &JsValue, tag: Option<&JsValue>, reconnect: bool) {
    let kind = Reflect::get(frame_js, &JsValue::from_str("kind"))
        .ok()
        .and_then(|v| v.as_string());
    match kind.as_deref() {
        Some("snapshot") => {
            let conclusions = Reflect::get(frame_js, &JsValue::from_str("conclusions"))
                .unwrap_or(JsValue::UNDEFINED);
            invoke_method_marked(consumer, "reset", &conclusions, tag, reconnect);
        }
        Some("delta") => {
            // Hand the consumer the `{asserted, retracted}` object as-is;
            // it applies both to its retained set keyed by conclusion
            // identity. A delta is never a reconnect's first frame — that
            // is always a snapshot — so the marker is not forwarded.
            invoke_method(consumer, "update", frame_js, tag);
        }
        // No `kind`: legacy bare-array frame → treat as a full snapshot.
        _ => invoke_method_marked(consumer, "reset", frame_js, tag, reconnect),
    }
}

/// Like [`invoke_method`], with a `reconnect` marker on the opts: `true`
/// flags the FIRST frame after a re-opened subscription, whose content may
/// briefly reflect state the worker lost (an overlay-backed record before
/// its owner re-asserts it). Consumers may hold their rendered content
/// through such a frame instead of treating it as settled truth.
fn invoke_method_marked(
    consumer: &Element,
    method: &str,
    payload: &JsValue,
    tag: Option<&JsValue>,
    reconnect: bool,
) {
    let opts = Object::new();
    if let Some(tag_val) = tag {
        let _ = Reflect::set(&opts, &JsValue::from_str("tag"), tag_val);
    }
    if reconnect {
        let _ = Reflect::set(&opts, &JsValue::from_str("reconnect"), &JsValue::TRUE);
    }
    let fn_val = match Reflect::get(consumer, &JsValue::from_str(method)) {
        Ok(v) => v,
        Err(_) => return,
    };
    let Ok(func) = fn_val.dyn_into::<Function>() else {
        return;
    };
    let _ = func.call2(consumer, payload, &opts.into());
}

/// `tonk-evaluate` handler.
///
/// Resolves the route, POSTs the raw asserted-notation text
/// (`detail.document`) to the `/evaluate` endpoint (with
/// `?transact=false` for a dry run), resolves the parsed response
/// into `detail.result`.
fn handle_evaluate(ev: &CustomEvent) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };

    // Absent `transact` defaults to a committing evaluate, matching
    // the worker's own `?transact=` default.
    let transact = match Reflect::get(&detail, &JsValue::from_str("transact")) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v.is_truthy(),
        _ => true,
    };
    let (space, branch, profile) = match route_from(&detail, event_origin(ev).as_ref()) {
        Ok(route) => route,
        Err(error) => return install_rejected_promise(&detail, error),
    };
    let url = evaluate_url(space.as_deref(), branch.as_deref(), profile, transact);

    let document = match Reflect::get(&detail, &JsValue::from_str("document"))
        .ok()
        .and_then(|v| v.as_string())
    {
        Some(s) => s,
        None => {
            install_rejected_promise(
                &detail,
                ErrorDetail::new(
                    ErrorKind::Parse,
                    "tonk-evaluate: detail.document must be a string",
                ),
            );
            return;
        }
    };

    let promise = future_to_promise(async move {
        match http::post_text(&url, &document, "application/x-tonk-notation").await {
            Ok(json_text) => parse_json_response(&json_text),
            Err(e) => Err(error_to_js(&e)),
        }
    });
    let _ = Reflect::set(&detail, &JsValue::from_str("result"), &promise);
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Array;
    use wasm_bindgen::prelude::Closure;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{CustomEventInit, ShadowRootInit, ShadowRootMode, window};

    wasm_bindgen_test_configure!(run_in_browser);

    /// A fake consumer element whose `reset`/`update`/`error` methods each
    /// push a `{ method, payload, opts }` record into a shared JS array the
    /// test reads back. The closures are kept alive by the returned struct
    /// so they outlive the `deliver_frame` call.
    struct FakeConsumer {
        element: Element,
        calls: Array,
        _closures: Vec<Closure<dyn FnMut(JsValue, JsValue)>>,
    }

    impl FakeConsumer {
        fn new() -> FakeConsumer {
            let element = window()
                .expect("window")
                .document()
                .expect("document")
                .create_element("tonk-fake-consumer")
                .expect("create_element");

            let calls = Array::new();
            let mut closures = Vec::new();
            for method in ["reset", "update", "error"] {
                let calls_for_method = calls.clone();
                let method_name = method.to_owned();
                let closure = Closure::wrap(Box::new(move |payload: JsValue, opts: JsValue| {
                    let record = Object::new();
                    let _ = Reflect::set(
                        &record,
                        &JsValue::from_str("method"),
                        &JsValue::from_str(&method_name),
                    );
                    let _ = Reflect::set(&record, &JsValue::from_str("payload"), &payload);
                    let _ = Reflect::set(&record, &JsValue::from_str("opts"), &opts);
                    calls_for_method.push(&record);
                }) as Box<dyn FnMut(JsValue, JsValue)>);
                let _ = Reflect::set(
                    &element,
                    &JsValue::from_str(method),
                    closure.as_ref().unchecked_ref(),
                );
                closures.push(closure);
            }

            FakeConsumer {
                element,
                calls,
                _closures: closures,
            }
        }

        /// Number of recorded method calls.
        fn len(&self) -> u32 {
            self.calls.length()
        }

        /// The recorded call at `index` as `{ method, payload, opts }`.
        fn call(&self, index: u32) -> Object {
            self.calls.get(index).dyn_into::<Object>().expect("record")
        }
    }

    fn field(object: &Object, key: &str) -> JsValue {
        Reflect::get(object, &JsValue::from_str(key)).expect("field")
    }

    fn method_of(record: &Object) -> String {
        field(record, "method").as_string().expect("method string")
    }

    fn opts_of(record: &Object) -> Object {
        field(record, "opts")
            .dyn_into::<Object>()
            .expect("opts object")
    }

    fn payload_of(record: &Object) -> JsValue {
        field(record, "payload")
    }

    /// Build a `{ kind: "snapshot", conclusions }` frame with `conclusions`
    /// as a JS array (its contents are irrelevant to routing).
    fn snapshot_frame() -> JsValue {
        let frame = Object::new();
        let _ = Reflect::set(
            &frame,
            &JsValue::from_str("kind"),
            &JsValue::from_str("snapshot"),
        );
        let conclusions = Array::new();
        conclusions.push(&JsValue::from_str("c0"));
        let _ = Reflect::set(&frame, &JsValue::from_str("conclusions"), &conclusions);
        frame.into()
    }

    /// Build a `{ kind: "delta", asserted, retracted }` frame.
    fn delta_frame() -> JsValue {
        let frame = Object::new();
        let _ = Reflect::set(
            &frame,
            &JsValue::from_str("kind"),
            &JsValue::from_str("delta"),
        );
        let _ = Reflect::set(&frame, &JsValue::from_str("asserted"), &Array::new());
        let _ = Reflect::set(&frame, &JsValue::from_str("retracted"), &Array::new());
        frame.into()
    }

    /// Build a bare `[…]` array frame (the pre-delta, kind-less wire).
    fn bare_array_frame() -> JsValue {
        let frame = Array::new();
        frame.push(&JsValue::from_str("c0"));
        frame.into()
    }

    /// A snapshot frame routes to `reset` with the tag forwarded on opts.
    #[dialog_common::test]
    async fn it_routes_a_snapshot_frame_to_reset_with_the_tag() {
        let consumer = FakeConsumer::new();
        let tag = JsValue::from_str("tonk:sheet");

        deliver_frame(&consumer.element, &snapshot_frame(), Some(&tag), false);

        assert_eq!(consumer.len(), 1, "exactly one method call");
        let record = consumer.call(0);
        assert_eq!(method_of(&record), "reset", "snapshot must route to reset");

        let opts = opts_of(&record);
        assert_eq!(
            field(&opts, "tag"),
            tag,
            "the tag must be forwarded on the reset opts"
        );

        // The payload is the frame's `conclusions`, not the whole frame.
        let payload = payload_of(&record)
            .dyn_into::<Array>()
            .expect("conclusions array");
        assert_eq!(payload.length(), 1, "reset receives the conclusions array");
    }

    /// A delta frame routes to `update` with the tag and NO reconnect marker.
    #[dialog_common::test]
    async fn it_routes_a_delta_frame_to_update_with_the_tag_and_no_reconnect() {
        let consumer = FakeConsumer::new();
        let tag = JsValue::from_str("tonk:sheet");

        // Even with reconnect=true, a delta must never carry the marker.
        deliver_frame(&consumer.element, &delta_frame(), Some(&tag), true);

        assert_eq!(consumer.len(), 1, "exactly one method call");
        let record = consumer.call(0);
        assert_eq!(method_of(&record), "update", "delta must route to update");

        let opts = opts_of(&record);
        assert_eq!(
            field(&opts, "tag"),
            tag,
            "the tag must be forwarded on the update opts"
        );
        assert!(
            field(&opts, "reconnect").is_undefined(),
            "a delta must never carry a reconnect marker"
        );
    }

    /// A bare-array (kind-less) frame routes to `reset` (legacy wire).
    #[dialog_common::test]
    async fn it_routes_a_bare_array_frame_to_reset() {
        let consumer = FakeConsumer::new();
        let tag = JsValue::from_str("tonk:sheet");

        deliver_frame(&consumer.element, &bare_array_frame(), Some(&tag), false);

        assert_eq!(consumer.len(), 1, "exactly one method call");
        let record = consumer.call(0);
        assert_eq!(
            method_of(&record),
            "reset",
            "a kind-less bare-array frame must route to reset"
        );

        // The whole array is handed through unchanged as the reset payload.
        let payload = payload_of(&record)
            .dyn_into::<Array>()
            .expect("array payload");
        assert_eq!(payload.length(), 1, "the bare array is the reset payload");
    }

    /// Each tag is forwarded verbatim, so a frame carrying the model tag can
    /// never land in the view or entity retained bucket. We deliver three
    /// frames on the same consumer, one per tag, and assert each recorded
    /// opts.tag matches the tag it was dispatched with.
    #[dialog_common::test]
    async fn it_forwards_the_tag_to_the_matching_retained_bucket() {
        let consumer = FakeConsumer::new();

        for tag_name in ["model", "view", "entity"] {
            let tag = JsValue::from_str(tag_name);
            deliver_frame(&consumer.element, &snapshot_frame(), Some(&tag), false);
        }

        assert_eq!(consumer.len(), 3, "three frames, three calls");
        for (index, tag_name) in ["model", "view", "entity"].iter().enumerate() {
            let record = consumer.call(index as u32);
            let opts = opts_of(&record);
            assert_eq!(
                field(&opts, "tag"),
                JsValue::from_str(tag_name),
                "frame {index} must carry its own tag, not another bucket's"
            );
        }
    }

    /// A reconnect snapshot is marked `reconnect: true`; the very next delta
    /// on the same consumer is not. Guards the invariant that only a
    /// snapshot ever flags the first post-reconnect frame.
    #[dialog_common::test]
    async fn it_marks_a_reconnect_snapshot_but_never_a_delta() {
        let consumer = FakeConsumer::new();
        let tag = JsValue::from_str("tonk:sheet");

        // First: the reconnect snapshot.
        deliver_frame(&consumer.element, &snapshot_frame(), Some(&tag), true);
        // Then: a following delta (also flagged reconnect at the call site,
        // which `deliver_frame` must ignore for deltas).
        deliver_frame(&consumer.element, &delta_frame(), Some(&tag), true);

        assert_eq!(consumer.len(), 2, "snapshot then delta");

        let snapshot = consumer.call(0);
        assert_eq!(method_of(&snapshot), "reset");
        assert_eq!(
            field(&opts_of(&snapshot), "reconnect"),
            JsValue::TRUE,
            "a reconnect snapshot must be marked reconnect:true"
        );

        let delta = consumer.call(1);
        assert_eq!(method_of(&delta), "update");
        assert!(
            field(&opts_of(&delta), "reconnect").is_undefined(),
            "a delta must never be marked reconnect, even on reconnect"
        );
    }

    /// A consumer inside a shadow root must be recognized as ITSELF.
    ///
    /// The operation events are `composed`, so they escape a shadow root —
    /// and crossing that boundary retargets `event.target` to the shadow
    /// HOST. Reading `target` therefore named `<tonk-prose>` for a
    /// `<tonk-display>` mounted in its shadow root, and every frame was
    /// then delivered by calling `reset` on the prose element, which has no
    /// such method. The display sat at `data-state="loading"` forever with
    /// its data going to an element that dropped it.
    #[dialog_common::test]
    async fn it_finds_a_consumer_inside_a_shadow_root() {
        let document = window().expect("window").document().expect("document");
        let outer = document.create_element("tonk-shadow-outer").expect("outer");
        document
            .body()
            .expect("body")
            .append_child(&outer)
            .expect("attach");
        let root = outer
            .attach_shadow(&ShadowRootInit::new(ShadowRootMode::Open))
            .expect("shadow root");
        let inner = document.create_element("tonk-shadow-inner").expect("inner");
        root.append_child(&inner).expect("append");

        // Listen where the real host listens: outside the shadow boundary.
        let seen = Rc::new(RefCell::new(Option::<String>::None));
        let seen_for_closure = seen.clone();
        let closure = Closure::wrap(Box::new(move |ev: CustomEvent| {
            *seen_for_closure.borrow_mut() = event_origin(&ev).map(|element| element.local_name());
        }) as Box<dyn FnMut(CustomEvent)>);
        let _ = document
            .add_event_listener_with_callback(events::SUBSCRIBE, closure.as_ref().unchecked_ref());

        let init = CustomEventInit::new();
        init.set_bubbles(true);
        init.set_composed(true);
        let ev = CustomEvent::new_with_event_init_dict(events::SUBSCRIBE, &init).expect("event");
        let _ = inner.dispatch_event(&ev);

        assert_eq!(
            seen.borrow().as_deref(),
            Some("tonk-shadow-inner"),
            "the dispatching element, not the shadow host `target` retargets to"
        );
        outer.remove();
    }
}
