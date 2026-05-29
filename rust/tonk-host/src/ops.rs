//! Operation event listeners on `<tonk-host>`.
//!
//! Six events are handled here:
//!
//! - `tonk-query` / `tonk-claim` / `tonk-evaluate` — one-shots;
//!   the handler writes a `Promise` into `detail.result`.
//! - `tonk-subscribe` — streaming; the handler opens the SSE,
//!   inserts a registry entry, writes a subscription handle
//!   into `detail.subscription`, and delivers per-frame data
//!   via `consumer.reset(...)` method calls.
//! - `tonk-unsubscribe` — drops all registry entries owned by
//!   the dispatching consumer.
//! - `tonk-context-refresh` — fired by a routing element when
//!   its `name` attribute changes; the handler finds affected
//!   subscriptions, groups them by depth, and re-issues them
//!   shallowest first.
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
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{future_to_promise, spawn_local};
use web_sys::{CustomEvent, Element, HtmlElement};

use crate::events;
use crate::host::HostState;
use crate::http;
use crate::registry::{Entry, EntryId};
use crate::url::{evaluate_url, query_url, transact_url};

/// One installed listener — closure plus the event name it
/// listens to. Held so the host can detach on disconnect.
pub(crate) struct InstalledListener {
    name: &'static str,
    closure: Closure<dyn FnMut(CustomEvent)>,
}

/// Attach the host's listeners for every event the protocol
/// defines. Returns the handles so the caller can detach later.
pub(crate) fn attach_all(
    this: &HtmlElement,
    state: Rc<RefCell<HostState>>,
) -> Vec<InstalledListener> {
    let host: Element = this.clone().into();
    let mut listeners = Vec::new();
    listeners.push(install_listener(&host, events::QUERY, {
        let state = state.clone();
        move |ev| handle_query(&ev, &state)
    }));
    listeners.push(install_listener(&host, events::CLAIM, {
        let state = state.clone();
        move |ev| handle_claim(&ev, &state)
    }));
    listeners.push(install_listener(&host, events::EVALUATE, {
        let state = state.clone();
        move |ev| handle_evaluate(&ev, &state)
    }));
    listeners.push(install_listener(&host, events::SUBSCRIBE, {
        let state = state.clone();
        move |ev| handle_subscribe(&ev, &state)
    }));
    listeners.push(install_listener(&host, events::UNSUBSCRIBE, {
        let state = state.clone();
        move |ev| handle_unsubscribe(&ev, &state)
    }));
    listeners.push(install_listener(&host, events::CONTEXT_REFRESH, {
        let state = state.clone();
        move |ev| handle_context_refresh(&ev, &state)
    }));
    listeners
}

/// Detach all listeners installed by `attach_all`.
pub(crate) fn detach_all(this: &HtmlElement, listeners: &[InstalledListener]) {
    let host: Element = this.clone().into();
    for l in listeners {
        let _ =
            host.remove_event_listener_with_callback(l.name, l.closure.as_ref().unchecked_ref());
    }
}

/// Helper used by every listener: stop propagation and mark the
/// event as handled by a provider.
fn claim_event(ev: &CustomEvent) {
    ev.stop_propagation();
    ev.prevent_default();
}

/// Install one event listener on `host` for `name`. Returns the
/// `InstalledListener` so the caller can hold it for later
/// detach.
fn install_listener<F>(host: &Element, name: &'static str, mut handler: F) -> InstalledListener
where
    F: FnMut(CustomEvent) + 'static,
{
    let closure =
        Closure::wrap(Box::new(move |ev: CustomEvent| handler(ev)) as Box<dyn FnMut(CustomEvent)>);
    let _ = host.add_event_listener_with_callback(name, closure.as_ref().unchecked_ref());
    InstalledListener { name, closure }
}

/// `tonk-query` handler.
///
/// Reads `detail.space`, `detail.branch`, `detail.query`. POSTs
/// `detail.query` to the structured-query endpoint, or returns a
/// cached response from the host's query LRU. Resolves the
/// `Vec<Conclusion>` into `detail.result`.
fn handle_query(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return, // No detail object — caller error; nothing we can do.
    };

    let space = get_string(&detail, "space");
    let branch = get_string(&detail, "branch");
    let url = query_url(space.as_deref(), branch.as_deref());

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

    // `body_str` is already canonical dag-json, so the same
    // semantic query always produces the same cache key.
    let cache_key = crate::query_cache::Key {
        space: space.clone(),
        branch: branch.clone(),
        body: body_str.clone(),
    };

    // Cache hit (resolved response): resolve the promise
    // synchronously with the previously-fetched body. We still
    // go through `future_to_promise` so the consumer's `await`
    // semantics are uniform.
    if !state.borrow().disposed
        && let Some(cached) = state.borrow_mut().query_cache.get(&cache_key)
    {
        let promise = future_to_promise(async move { parse_json_response(&cached) });
        let _ = Reflect::set(&detail, &JsValue::from_str("result"), &promise);
        return;
    }

    // In-flight hit: a previous query with the same key is
    // already on the wire. Reuse its Promise so multiple
    // displays mounting in parallel share one HTTP round-trip
    // instead of stampeding the worker.
    if !state.borrow().disposed
        && let Some(pending) = state.borrow().query_cache.get_pending(&cache_key)
    {
        let _ = Reflect::set(&detail, &JsValue::from_str("result"), &pending);
        return;
    }

    let state_for_cache = state.clone();
    let cache_key_for_async = cache_key.clone();
    let promise = future_to_promise(async move {
        let result = http::post_json(&url, &body_str).await;
        // Always clear the in-flight entry once the network
        // settles — success populates the resolved cache below,
        // failure leaves nothing behind so a retry can try
        // again.
        if !state_for_cache.borrow().disposed {
            state_for_cache
                .borrow_mut()
                .query_cache
                .clear_pending(&cache_key_for_async);
        }
        match result {
            Ok(json_text) => {
                if !state_for_cache.borrow().disposed {
                    state_for_cache
                        .borrow_mut()
                        .query_cache
                        .put(cache_key_for_async, json_text.clone());
                }
                parse_json_response(&json_text)
            }
            Err(e) => Err(error_to_js(&e)),
        }
    });
    // Stash the in-flight Promise so concurrent dispatches see
    // it before the network settles.
    if !state.borrow().disposed {
        state
            .borrow_mut()
            .query_cache
            .put_pending(cache_key, JsValue::from(promise.clone()));
    }
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

/// Serialize a JS value as canonical DAG-JSON text. Goes
/// `JsValue → Ipld` via `serde-wasm-bindgen` (the only
/// adapter that knows how to project JS `Map`s and typed
/// arrays into the IPLD data model), then `Ipld → bytes`
/// via `serde_ipld_dagjson`. DAG-JSON sorts map keys
/// deterministically and has a stable number/string
/// encoding, so two semantically equal queries always
/// produce the same text — usable both as the HTTP body
/// and as the cache key without a second canonicalisation
/// pass.
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
/// Reads `detail.space`, `detail.branch`, `detail.request`.
/// POSTs the structured `TransactRequest` to the `/transact`
/// endpoint. Resolves the parsed response into `detail.result`.
/// Invalidates the query cache for the affected branch so a
/// claim that changes concept descriptors doesn't leave stale
/// phase-1 results in memory.
fn handle_claim(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };

    let space = get_string(&detail, "space");
    let branch = get_string(&detail, "branch");
    let url = transact_url(space.as_deref(), branch.as_deref());

    if !state.borrow().disposed {
        state
            .borrow_mut()
            .query_cache
            .invalidate_branch(space.as_deref(), branch.as_deref());
    }

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
/// Reads `detail.space`, `detail.branch`, `detail.query`, and
/// optional `detail.tag`. Inserts an entry in the registry, opens
/// an upstream SSE, routes each frame to the consumer's
/// `reset(conclusions, opts)` method (v1 SW emits only `reset`).
/// Writes `{ cancel }` into `detail.subscription` for caller-side
/// teardown.
fn handle_subscribe(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };
    let consumer = match ev.target().and_then(|t| t.dyn_into::<Element>().ok()) {
        Some(el) => el,
        None => return,
    };

    let space = get_string(&detail, "space");
    let branch = get_string(&detail, "branch");
    let url = query_url(space.as_deref(), branch.as_deref());

    let query_val = match Reflect::get(&detail, &JsValue::from_str("query")) {
        Ok(v) if !v.is_undefined() && !v.is_null() => v,
        _ => return,
    };
    let query_ipld: Ipld = match serde_wasm_bindgen::from_value(query_val.clone()) {
        Ok(v) => v,
        Err(_) => return,
    };
    let tag_js = Reflect::get(&detail, &JsValue::from_str("tag"))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null());
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
        let consumer_err = consumer_for_spawn.clone();
        let tag_err = tag_for_spawn.clone();
        let state_err = state_for_spawn.clone();
        let abort = open_sse(
            &url,
            &body,
            move |frame: &str| {
                if !consumer_frame.is_connected() {
                    return;
                }
                let conclusions: Vec<Conclusion> = match serde_json::from_str(frame) {
                    Ok(v) => v,
                    Err(e) => {
                        invoke_method(
                            &consumer_frame,
                            "error",
                            &error_to_js(&ErrorDetail::new(
                                ErrorKind::Parse,
                                format!("subscribe frame: {e}"),
                            )),
                            tag_frame.as_ref(),
                        );
                        return;
                    }
                };
                let conclusions_js =
                    serde_wasm_bindgen::to_value(&conclusions).unwrap_or(JsValue::NULL);
                invoke_method(
                    &consumer_frame,
                    "reset",
                    &conclusions_js,
                    tag_frame.as_ref(),
                );
            },
            move |err: ErrorDetail| {
                if !consumer_err.is_connected() {
                    return;
                }
                invoke_method(&consumer_err, "error", &error_to_js(&err), tag_err.as_ref());
                // Drop the registry entry on transport error so the
                // consumer can re-subscribe cleanly.
                let mut s = state_err.borrow_mut();
                s.registry.remove(entry_id);
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
                let mut s = state_for_spawn.borrow_mut();
                s.registry.remove(entry_id);
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

/// `tonk-context-refresh` handler. A routing element's
/// `attribute_changed_callback` dispatches this when its `name`
/// changes. The host finds every subscription whose consumer is
/// a DOM descendant of the changed routing element, groups them
/// by recorded `depth`, and re-issues them shallowest first.
///
/// Between depth groups, the handler yields to the microtask
/// queue so the synchronous iteration diffs the consumer
/// triggers (in its `reset`) have a chance to fire
/// `disconnectedCallback` → `tonk-unsubscribe`, which prunes
/// doomed entries from the registry before the next depth runs.
fn handle_context_refresh(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    ev.stop_propagation();
    let Some(target) = ev.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
        return;
    };
    // Snapshot the affected ids grouped by depth. Holding the
    // borrow only for this scan keeps it cheap.
    let mut by_depth: std::collections::BTreeMap<u32, Vec<crate::registry::EntryId>> =
        std::collections::BTreeMap::new();
    {
        let s = state.borrow();
        for id in s.registry.ids_under(&target) {
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
                if !crate::host::is_alive(&state_for_refresh) {
                    return;
                }
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
/// `(space, branch)` context, reading fresh annotations from
/// the consumer's routing-element ancestors. If the consumer is
/// no longer in the DOM, drops the entry without re-issuing.
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
    // Read fresh context from the consumer's ancestors. Walk up
    // the DOM looking for the nearest `<tonk-repository>` and
    // `<tonk-branch>` element; their `name` attributes are the
    // current context.
    let (space, branch) = read_context_from_ancestors(&consumer);
    // Abort the existing upstream and clear its handle so the
    // refresh's new subscription is the only live one.
    let url = query_url(space.as_deref(), branch.as_deref());
    {
        let mut s = state.borrow_mut();
        if let Some(entry) = s.registry.get(entry_id) {
            // Re-borrow as mutable to clear abort. The above is
            // just a guard against the entry having vanished.
            let _ = entry;
            if let Some(e) = s.registry.entries_mut().get_mut(&entry_id) {
                e.abort.take();
                e.space = space.clone();
                e.branch = branch.clone();
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
    let consumer_err = consumer.clone();
    let tag_err = tag.clone();
    let abort_result = open_sse(
        &url,
        &body_ipld,
        move |frame: &str| {
            if !consumer_frame.is_connected() {
                return;
            }
            let conclusions: Vec<Conclusion> = match serde_json::from_str(frame) {
                Ok(v) => v,
                Err(e) => {
                    invoke_method(
                        &consumer_frame,
                        "error",
                        &error_to_js(&ErrorDetail::new(
                            ErrorKind::Parse,
                            format!("refresh frame: {e}"),
                        )),
                        tag_frame.as_ref(),
                    );
                    return;
                }
            };
            let conclusions_js =
                serde_wasm_bindgen::to_value(&conclusions).unwrap_or(JsValue::NULL);
            invoke_method(
                &consumer_frame,
                "reset",
                &conclusions_js,
                tag_frame.as_ref(),
            );
        },
        move |err: ErrorDetail| {
            if !consumer_err.is_connected() {
                return;
            }
            invoke_method(&consumer_err, "error", &error_to_js(&err), tag_err.as_ref());
            let mut s = state_err.borrow_mut();
            s.registry.remove(entry_id);
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
            let mut s = state.borrow_mut();
            s.registry.remove(entry_id);
        }
    }
}

/// Walk up from `consumer` looking for the nearest
/// `<tonk-repository>` and `<tonk-branch>` ancestor; return their
/// `name` attributes. Inner-most-wins — the first one found on
/// the way up.
fn read_context_from_ancestors(consumer: &Element) -> (Option<String>, Option<String>) {
    let mut space: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut node: Option<Element> = consumer.parent_element();
    while let Some(el) = node {
        let tag = el.tag_name().to_ascii_lowercase();
        if branch.is_none() && tag == "tonk-branch" {
            branch = el.get_attribute("name").filter(|s| !s.is_empty());
        } else if space.is_none() && tag == "tonk-repository" {
            space = el.get_attribute("name").filter(|s| !s.is_empty());
        }
        if space.is_some() && branch.is_some() {
            break;
        }
        node = el.parent_element();
    }
    (space, branch)
}

/// `tonk-unsubscribe` handler. Drops all registry entries owned
/// by `event.target`.
fn handle_unsubscribe(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let consumer = match ev.target().and_then(|t| t.dyn_into::<Element>().ok()) {
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
    let opts = Object::new();
    if let Some(tag_val) = tag {
        let _ = Reflect::set(&opts, &JsValue::from_str("tag"), tag_val);
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
/// Reads `detail.space`, `detail.branch`, `detail.document`.
/// POSTs the raw asserted-notation text to the `/evaluate`
/// endpoint. Resolves the parsed response into `detail.result`.
/// Invalidates the query cache for the affected branch since an
/// evaluate document can introduce or mutate concepts.
fn handle_evaluate(ev: &CustomEvent, state: &Rc<RefCell<HostState>>) {
    claim_event(ev);
    let detail = match ev.detail().dyn_into::<Object>() {
        Ok(o) => o,
        Err(_) => return,
    };

    let space = get_string(&detail, "space");
    let branch = get_string(&detail, "branch");
    let url = evaluate_url(space.as_deref(), branch.as_deref());

    if !state.borrow().disposed {
        state
            .borrow_mut()
            .query_cache
            .invalidate_branch(space.as_deref(), branch.as_deref());
    }

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
