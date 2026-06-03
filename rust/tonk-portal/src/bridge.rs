//! The live-data bridge injected into a portal's iframe.
//!
//! A portal mounts a **same-origin** iframe and prepends one line to
//! its document:
//!
//! ```html
//! <script>window.tonk = window.parent.__tonkConnect(window)</script>
//! ```
//!
//! `__tonkConnect` is a single function this module registers on the
//! top window. Given the inner window it returns the bridge object
//! bound to the portal whose iframe hosts that window — matched by
//! live `iframe.contentWindow` identity, so one function serves every
//! portal and survives reloads (the iframe ref is stable even when a
//! reload swaps its `contentWindow`).
//!
//! The bridge is the consumer: `subscribe` / `query` / `transact`
//! dispatch the existing `tonk-subscribe` / `tonk-query` / `tonk-claim`
//! events on the `<tonk-portal>` element, which bubble through the
//! routing ancestors to `<tonk-host>`. The portal never parses a query
//! or touches the network — it relays. Subscription frames arrive back
//! through the portal's `reset` method (the same seam `<tonk-display>`
//! uses) and are enqueued into the author's `ReadableStream`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use js_sys::{Function, Object, Promise, Reflect};
use tonk_host::consumer::{self as host_consumer, Subscription as HostSubscription};
use tonk_schema::conclusion::Conclusion;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::future_to_promise;
use web_sys::{Element, HtmlIFrameElement, window};

/// Per-portal bridge + iframe state. Held behind `Rc<RefCell<…>>` so
/// it is reachable from the element lifecycle, the prototype `reset`
/// delegate, and the bridge's own closures.
pub(crate) struct PortalState {
    /// The single child iframe. Owned here so attribute callbacks can
    /// reload it and `disconnected_callback` can detach it.
    pub iframe: Option<HtmlIFrameElement>,
    /// Set by `disconnected_callback`; mirrors `<tonk-display>`.
    pub disposed: bool,
    /// Monotonic counter minting unique subscription tags.
    next_tag: u64,
    /// Live subscriptions keyed by tag → stream controller + host
    /// handle. Dropping an entry cancels its host subscription.
    subs: BTreeMap<String, BridgeSub>,
    /// The author-facing `context` object (`{ this, model }`), updated
    /// in place when the scoped entity / model changes.
    context: Option<Object>,
    /// The bridge's method closures, kept alive for its lifetime.
    _callbacks: Vec<Closure<dyn FnMut(JsValue) -> JsValue>>,
}

/// One live subscription: the stream controller methods plus the host
/// subscription handle (whose `Drop` cancels upstream).
struct BridgeSub {
    enqueue: Function,
    error: Function,
    _host_sub: HostSubscription,
    _cancel_cb: Closure<dyn FnMut(JsValue)>,
}

impl PortalState {
    pub(crate) fn new() -> Self {
        Self {
            iframe: None,
            disposed: false,
            next_tag: 0,
            subs: BTreeMap::new(),
            context: None,
            _callbacks: Vec::new(),
        }
    }

    /// Cancel and forget every live subscription. Dropping each
    /// `BridgeSub` cancels its host subscription, so a reload or
    /// teardown never leaves a dangling SSE.
    pub(crate) fn clear_subs(&mut self) {
        self.subs.clear();
    }
}

/// Prepend the bootstrap line that wires `window.tonk` to this portal's
/// bridge. Under same-origin, `window.parent.__tonkConnect` is a real
/// synchronous function, so author code calls `tonk.*` at top level
/// with no `await`.
pub(crate) fn bootstrap_srcdoc(content: &str) -> String {
    format!("<script>window.tonk = window.parent.__tonkConnect(window)</script>{content}")
}

// --- `__tonkConnect` registry -------------------------------------

struct PortalEntry {
    iframe: HtmlIFrameElement,
    bridge: JsValue,
}

thread_local! {
    static REGISTRY: Rc<RefCell<Vec<PortalEntry>>> = Rc::new(RefCell::new(Vec::new()));
    static CONNECT_INSTALLED: RefCell<bool> = const { RefCell::new(false) };
}

/// Install the page-level `window.__tonkConnect(window)` function.
/// Idempotent.
pub(crate) fn install_connect() {
    let already = CONNECT_INSTALLED.with(|c| {
        let was = *c.borrow();
        *c.borrow_mut() = true;
        was
    });
    if already {
        return;
    }
    let Some(win) = window() else {
        return;
    };
    let registry = REGISTRY.with(|r| r.clone());
    let connect: Closure<dyn FnMut(JsValue) -> JsValue> =
        Closure::wrap(Box::new(move |win_arg: JsValue| -> JsValue {
            let reg = registry.borrow();
            for entry in reg.iter() {
                if let Some(cw) = entry.iframe.content_window() {
                    let cw_val: JsValue = cw.into();
                    if cw_val == win_arg {
                        return entry.bridge.clone();
                    }
                }
            }
            JsValue::UNDEFINED
        }) as Box<dyn FnMut(JsValue) -> JsValue>);
    let _ = Reflect::set(
        &win,
        &"__tonkConnect".into(),
        connect.as_ref().unchecked_ref(),
    );
    // Lives for the page's lifetime — there is exactly one.
    connect.forget();
}

/// Register `(iframe, bridge)` so `__tonkConnect` can resolve the
/// bridge from the iframe's live `contentWindow`.
pub(crate) fn register_portal(iframe: &HtmlIFrameElement, bridge: &JsValue) {
    REGISTRY.with(|r| {
        r.borrow_mut().push(PortalEntry {
            iframe: iframe.clone(),
            bridge: bridge.clone(),
        })
    });
}

/// Drop the registry entry for `iframe` on teardown.
pub(crate) fn unregister_portal(iframe: &HtmlIFrameElement) {
    REGISTRY.with(|r| {
        r.borrow_mut()
            .retain(|e| !e.iframe.is_same_node(Some(iframe.as_ref())))
    });
}

// --- Bridge construction ------------------------------------------

/// Build the bridge object for `host` and store its context + closures
/// in `state`. The returned value is what the iframe sees as
/// `window.tonk`.
pub(crate) fn build_bridge(host: &Element, state: &Rc<RefCell<PortalState>>) -> JsValue {
    let bridge = Object::new();

    // context { this, model } — updated in place on rescope.
    let context = Object::new();
    write_context(host, &context);
    let _ = Reflect::set(&bridge, &"context".into(), &context);

    // ready — instant under same-origin (becomes a real handshake
    // await when the transport ratchets to postMessage).
    let _ = Reflect::set(
        &bridge,
        &"ready".into(),
        &Promise::resolve(&JsValue::UNDEFINED),
    );

    let query_cb = make_query_cb(host.clone());
    let _ = Reflect::set(&bridge, &"query".into(), query_cb.as_ref().unchecked_ref());

    let transact_cb = make_transact_cb(host.clone());
    let _ = Reflect::set(
        &bridge,
        &"transact".into(),
        transact_cb.as_ref().unchecked_ref(),
    );

    let subscribe_cb = make_subscribe_cb(host.clone(), state.clone());
    let _ = Reflect::set(
        &bridge,
        &"subscribe".into(),
        subscribe_cb.as_ref().unchecked_ref(),
    );

    {
        let mut s = state.borrow_mut();
        s.context = Some(context);
        s._callbacks = vec![query_cb, transact_cb, subscribe_cb];
    }
    bridge.into()
}

/// Refresh the author-facing `context` from the host's current
/// `entity` / `model` attributes. Called when display re-scopes the
/// portal; the reload then re-runs author code against fresh context.
pub(crate) fn rescope(host: &Element, state: &Rc<RefCell<PortalState>>) {
    if let Some(ctx) = state.borrow().context.as_ref() {
        write_context(host, ctx);
    }
}

fn write_context(host: &Element, context: &Object) {
    let this = host.get_attribute("entity").unwrap_or_default();
    let model = host.get_attribute("model").unwrap_or_default();
    let _ = Reflect::set(context, &"this".into(), &JsValue::from_str(&this));
    let _ = Reflect::set(context, &"model".into(), &JsValue::from_str(&model));
}

fn make_query_cb(host: Element) -> Closure<dyn FnMut(JsValue) -> JsValue> {
    Closure::wrap(Box::new(move |arg: JsValue| -> JsValue {
        let body = match query_body(&host, &arg) {
            Ok(b) => b,
            Err(msg) => return Promise::reject(&JsValue::from_str(&msg)).into(),
        };
        let host = host.clone();
        future_to_promise(async move {
            host_consumer::query(&host, &body)
                .await
                .map_err(|e| JsValue::from_str(&e.message))
        })
        .into()
    }) as Box<dyn FnMut(JsValue) -> JsValue>)
}

fn make_transact_cb(host: Element) -> Closure<dyn FnMut(JsValue) -> JsValue> {
    Closure::wrap(Box::new(move |request: JsValue| -> JsValue {
        let host = host.clone();
        future_to_promise(async move {
            host_consumer::claim(&host, &request)
                .await
                .map_err(|e| JsValue::from_str(&e.message))
        })
        .into()
    }) as Box<dyn FnMut(JsValue) -> JsValue>)
}

fn make_subscribe_cb(
    host: Element,
    state: Rc<RefCell<PortalState>>,
) -> Closure<dyn FnMut(JsValue) -> JsValue> {
    Closure::wrap(Box::new(move |arg: JsValue| -> JsValue {
        let body = match query_body(&host, &arg) {
            Ok(b) => b,
            Err(msg) => return errored_stream(&msg),
        };

        let tag = {
            let mut s = state.borrow_mut();
            s.next_tag = s.next_tag.wrapping_add(1);
            format!("portal-sub-{}", s.next_tag)
        };

        // The stream's `cancel` (author cancels, or a pipe aborts)
        // drops our `BridgeSub`, which cancels the host subscription.
        let cancel_state = state.clone();
        let cancel_tag = tag.clone();
        let cancel_cb: Closure<dyn FnMut(JsValue)> = Closure::wrap(Box::new(move |_reason| {
            cancel_state.borrow_mut().subs.remove(&cancel_tag);
        })
            as Box<dyn FnMut(JsValue)>);

        let Some((stream, enqueue, error)) = make_stream(&cancel_cb) else {
            return errored_stream("failed to construct ReadableStream");
        };

        let tag_js = JsValue::from_str(&tag);
        match host_consumer::subscribe(&host, &body, Some(&tag_js)) {
            Ok(host_sub) => {
                state.borrow_mut().subs.insert(
                    tag,
                    BridgeSub {
                        enqueue,
                        error,
                        _host_sub: host_sub,
                        _cancel_cb: cancel_cb,
                    },
                );
            }
            Err(e) => {
                // No host ancestor / dispatch failure: surface to the
                // author through the stream; nothing is tracked.
                let _ = error.call1(&JsValue::NULL, &JsValue::from_str(&e.message));
            }
        }
        stream
    }) as Box<dyn FnMut(JsValue) -> JsValue>)
}

/// Build the query body for a bridge call: no argument streams the
/// scoped entity; an explicit body is forwarded verbatim.
fn query_body(host: &Element, arg: &JsValue) -> Result<JsValue, String> {
    if arg.is_undefined() || arg.is_null() {
        no_arg_entity_query(host)
    } else {
        Ok(arg.clone())
    }
}

fn no_arg_entity_query(host: &Element) -> Result<JsValue, String> {
    let entity = host
        .get_attribute("entity")
        .filter(|s| !s.is_empty())
        .ok_or("tonk.subscribe()/query() with no argument requires a scoped `entity`")?;
    let descriptor = read_descriptor(host)
        .ok_or("tonk.subscribe()/query() with no argument requires a model descriptor")?;
    let query = crate::query::entity_query(&descriptor, &entity)
        .map_err(|e| format!("entity query: {e}"))?;
    serde_wasm_bindgen::to_value(&query).map_err(|e| format!("query body: {e}"))
}

fn read_descriptor(host: &Element) -> Option<String> {
    Reflect::get(host, &"descriptor".into())
        .ok()
        .and_then(|v| v.as_string())
}

/// Construct a `ReadableStream` plus its `enqueue` / `error` controller
/// methods, wiring the stream's `cancel` to `cancel_cb`.
fn make_stream(cancel_cb: &Closure<dyn FnMut(JsValue)>) -> Option<(JsValue, Function, Function)> {
    let helper = Function::new_with_args(
        "cancelCb",
        "const out = {};\
         out.stream = new ReadableStream({\
           start(c) { out.enqueue = (v) => c.enqueue(v); out.error = (e) => c.error(e); },\
           cancel(reason) { cancelCb(reason); }\
         });\
         return out;",
    );
    let out = helper
        .call1(&JsValue::NULL, cancel_cb.as_ref().unchecked_ref())
        .ok()?;
    let stream = Reflect::get(&out, &"stream".into()).ok()?;
    let enqueue = Reflect::get(&out, &"enqueue".into())
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    let error = Reflect::get(&out, &"error".into())
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    Some((stream, enqueue, error))
}

/// A stream that immediately errors — for a `subscribe` that could not
/// be set up. The author's `for await` rejects with the message.
fn errored_stream(message: &str) -> JsValue {
    let helper = Function::new_with_args(
        "msg",
        "return new ReadableStream({ start(c) { c.error(new Error(msg)); } });",
    );
    helper
        .call1(&JsValue::NULL, &JsValue::from_str(message))
        .unwrap_or(JsValue::UNDEFINED)
}

// --- Frame routing (called by the element's reset / error shims) --

/// `reset(conclusions, { tag })` — a subscription frame from the host.
/// The host serializes conclusions with `serde-wasm-bindgen`, which
/// renders maps as JS `Map`s (and integers as `BigInt`). Round-trip
/// through JSON so the author reads `conclusion.fields.x` by dot access
/// and sees the *same* plain shape `tonk.query()` yields (the host
/// `JSON.parse`s one-shot results) — numbers, not `BigInt`s.
pub(crate) fn route_reset(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let conclusions: Vec<Conclusion> = match serde_wasm_bindgen::from_value(payload) {
        Ok(v) => v,
        Err(_) => return,
    };
    let plain = match serde_json::to_string(&conclusions) {
        Ok(json) => js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL),
        Err(_) => return,
    };
    let enqueue = state.borrow().subs.get(&tag).map(|sub| sub.enqueue.clone());
    if let Some(enqueue) = enqueue {
        let _ = enqueue.call1(&JsValue::NULL, &plain);
    }
}

/// `error(detail, { tag })` — a transport error on a subscription.
/// Errors the matching author stream.
pub(crate) fn route_error(state: &Rc<RefCell<PortalState>>, payload: JsValue, opts: JsValue) {
    let Some(tag) = read_tag(&opts) else {
        return;
    };
    let error = state.borrow().subs.get(&tag).map(|sub| sub.error.clone());
    if let Some(error) = error {
        let _ = error.call1(&JsValue::NULL, &payload);
    }
}

fn read_tag(opts: &JsValue) -> Option<String> {
    if !opts.is_object() {
        return None;
    }
    Reflect::get(opts, &"tag".into())
        .ok()
        .and_then(|v| v.as_string())
}

#[cfg(test)]
mod tests {
    use js_sys::{Function, Object, Promise, Reflect};
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen_futures::JsFuture;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::{CustomEvent, Document, Element, HtmlIFrameElement, window};

    wasm_bindgen_test_configure!(run_in_browser);

    fn document() -> Document {
        window().expect("window").document().expect("document")
    }

    /// Sleep `ms` milliseconds, yielding to the event loop so the
    /// iframe can parse + run its bootstrap.
    async fn sleep(ms: i32) {
        let promise = Promise::new(&mut |resolve, _reject| {
            let _ = window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        });
        let _ = JsFuture::from(promise).await;
    }

    /// Poll the iframe's `contentWindow.tonk` until the bootstrap has
    /// installed it (or give up). Same-origin access is required for
    /// this read to succeed; with an opaque origin it throws and we
    /// time out.
    async fn await_bridge(iframe: &HtmlIFrameElement) -> JsValue {
        for _ in 0..200 {
            if let Some(win) = iframe.content_window()
                && let Ok(tonk) = Reflect::get(&win, &"tonk".into())
                && !tonk.is_undefined()
                && !tonk.is_null()
            {
                return tonk;
            }
            sleep(5).await;
        }
        JsValue::UNDEFINED
    }

    fn get_str(obj: &JsValue, key: &str) -> Option<String> {
        Reflect::get(obj, &key.into())
            .ok()
            .and_then(|v| v.as_string())
    }

    /// A minimal stand-in for `<tonk-host>`: a container element that
    /// answers the consumer events the bridge dispatches with canned
    /// data, captures the live subscription consumer + tag so a test
    /// can push frames, and records cancellation.
    struct FakeHost {
        container: Element,
        state: Rc<RefCell<FakeState>>,
        // Kept alive for the listeners' lifetime.
        _listeners: Vec<Closure<dyn FnMut(CustomEvent)>>,
    }

    #[derive(Default)]
    struct FakeState {
        /// Canned `Vec<Conclusion>`-shaped JS array returned by query.
        query_result: Option<JsValue>,
        /// Canned receipt JS value returned by claim.
        claim_result: Option<JsValue>,
        /// The last query body passed to a `tonk-query`.
        last_query_body: Option<JsValue>,
        /// The last request body passed to a `tonk-claim`.
        last_claim_body: Option<JsValue>,
        /// The consumer + tag captured from the last `tonk-subscribe`,
        /// so the test can push frames via `consumer.reset(...)`.
        sub_consumer: Option<Element>,
        sub_tag: Option<JsValue>,
        last_subscribe_body: Option<JsValue>,
        /// Set true when the host-side `cancel()` is invoked.
        cancelled: bool,
    }

    impl FakeHost {
        fn install() -> FakeHost {
            let container = document().create_element("div").expect("div");
            document()
                .body()
                .expect("body")
                .append_child(&container)
                .expect("attach container");
            let state = Rc::new(RefCell::new(FakeState::default()));
            let mut listeners = Vec::new();

            // tonk-query
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        state.borrow_mut().last_query_body = Some(query);
                        let result = state
                            .borrow()
                            .query_result
                            .clone()
                            .unwrap_or(JsValue::from(js_sys::Array::new()));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-query", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            // tonk-claim
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let request = Reflect::get(&detail, &"request".into()).unwrap();
                        state.borrow_mut().last_claim_body = Some(request);
                        let result = state
                            .borrow()
                            .claim_result
                            .clone()
                            .unwrap_or(JsValue::from_str("ok"));
                        let _ = Reflect::set(&detail, &"result".into(), &Promise::resolve(&result));
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container
                    .add_event_listener_with_callback("tonk-claim", cb.as_ref().unchecked_ref());
                listeners.push(cb);
            }
            // tonk-subscribe
            {
                let state = state.clone();
                let cb: Closure<dyn FnMut(CustomEvent)> =
                    Closure::wrap(Box::new(move |ev: CustomEvent| {
                        ev.stop_propagation();
                        ev.prevent_default();
                        let detail: Object = ev.detail().dyn_into().unwrap();
                        let query = Reflect::get(&detail, &"query".into()).unwrap();
                        let tag = Reflect::get(&detail, &"tag".into()).ok();
                        let consumer: Element = ev.target().unwrap().dyn_into().unwrap();
                        {
                            let mut s = state.borrow_mut();
                            s.last_subscribe_body = Some(query);
                            s.sub_consumer = Some(consumer);
                            s.sub_tag = tag;
                        }
                        // detail.subscription = { cancel }
                        let sub = Object::new();
                        let state_for_cancel = state.clone();
                        let cancel: Closure<dyn FnMut()> = Closure::wrap(Box::new(move || {
                            state_for_cancel.borrow_mut().cancelled = true;
                        })
                            as Box<dyn FnMut()>);
                        let cancel_fn: Function = cancel.into_js_value().unchecked_into();
                        let _ = Reflect::set(&sub, &"cancel".into(), &cancel_fn);
                        let _ = Reflect::set(&detail, &"subscription".into(), &sub);
                    }) as Box<dyn FnMut(CustomEvent)>);
                let _ = container.add_event_listener_with_callback(
                    "tonk-subscribe",
                    cb.as_ref().unchecked_ref(),
                );
                listeners.push(cb);
            }

            FakeHost {
                container,
                state,
                _listeners: listeners,
            }
        }

        fn set_query_result(&self, value: JsValue) {
            self.state.borrow_mut().query_result = Some(value);
        }

        fn set_claim_result(&self, value: JsValue) {
            self.state.borrow_mut().claim_result = Some(value);
        }

        fn last_query_body(&self) -> Option<JsValue> {
            self.state.borrow().last_query_body.clone()
        }

        fn last_claim_body(&self) -> Option<JsValue> {
            self.state.borrow().last_claim_body.clone()
        }

        /// Push a subscription frame to the captured consumer, mirroring
        /// how the real host calls `consumer.reset(conclusions, { tag })`.
        fn push_frame(&self, conclusions: &JsValue) {
            let (consumer, tag) = {
                let s = self.state.borrow();
                (s.sub_consumer.clone(), s.sub_tag.clone())
            };
            let Some(consumer) = consumer else { return };
            let opts = Object::new();
            if let Some(t) = tag {
                let _ = Reflect::set(&opts, &"tag".into(), &t);
            }
            let reset = Reflect::get(&consumer, &"reset".into()).unwrap();
            let reset: Function = reset.dyn_into().expect("reset method");
            let _ = reset.call2(&consumer, conclusions, &opts);
        }

        fn cancelled(&self) -> bool {
            self.state.borrow().cancelled
        }
    }

    /// Mount a `<tonk-portal>` under the fake host's container with the
    /// given attributes + optional descriptor JSON property.
    fn mount_portal(
        host: &FakeHost,
        content: &str,
        entity: Option<&str>,
        model: Option<&str>,
        descriptor: Option<&str>,
    ) -> Element {
        crate::register();
        let portal = document()
            .create_element("tonk-portal")
            .expect("tonk-portal");
        portal.set_attribute("content", content).expect("content");
        if let Some(e) = entity {
            portal.set_attribute("entity", e).expect("entity");
        }
        if let Some(m) = model {
            portal.set_attribute("model", m).expect("model");
        }
        if let Some(d) = descriptor {
            let _ = Reflect::set(portal.as_ref(), &"descriptor".into(), &JsValue::from_str(d));
        }
        host.container.append_child(&portal).expect("attach portal");
        portal
    }

    fn iframe_of(portal: &Element) -> HtmlIFrameElement {
        portal
            .query_selector("iframe")
            .expect("query")
            .expect("iframe mounted")
            .dyn_into()
            .expect("HtmlIFrameElement")
    }

    #[dialog_common::test]
    async fn it_exposes_context_from_entity_and_model_attributes() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            None,
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;
        assert!(
            !tonk.is_undefined(),
            "window.tonk should be defined inside the same-origin iframe",
        );
        let ctx = Reflect::get(&tonk, &"context".into()).expect("context");
        assert_eq!(get_str(&ctx, "this").as_deref(), Some("id:demo-counter"));
        assert_eq!(get_str(&ctx, "model").as_deref(), Some("counter"));
    }

    const DESCRIPTOR: &str = r#"{"with":{
        "count": { "the": "counter/count", "as": "UnsignedInteger", "cardinality": "one" }
    }}"#;

    /// Look up a method on the bridge and call it with the given args.
    fn call_method(tonk: &JsValue, method: &str, args: &[JsValue]) -> JsValue {
        let f: Function = Reflect::get(tonk, &method.into())
            .expect("method")
            .dyn_into()
            .expect("method is a function");
        let arr = js_sys::Array::new();
        for a in args {
            arr.push(a);
        }
        Reflect::apply(&f, tonk, &arr).expect("call")
    }

    async fn await_promise(value: JsValue) -> Result<JsValue, JsValue> {
        let promise: Promise = value.dyn_into().expect("a Promise");
        JsFuture::from(promise).await
    }

    /// A host-shaped subscription frame: `Vec<Conclusion>` serialized
    /// with `serde-wasm-bindgen` (which renders maps as JS `Map`s),
    /// exactly as `<tonk-host>` delivers them — so the test exercises
    /// the bridge's conversion to dot-accessible objects.
    fn host_frame(this: &str, count: i128) -> JsValue {
        use ipld_core::ipld::Ipld;
        use std::collections::BTreeMap;
        use tonk_schema::conclusion::Conclusion;
        let mut fields: BTreeMap<String, Ipld> = BTreeMap::new();
        fields.insert("count".to_owned(), Ipld::Integer(count));
        let conclusions = vec![Conclusion {
            this: this.to_owned(),
            fields,
        }];
        serde_wasm_bindgen::to_value(&conclusions).expect("serialize frame")
    }

    #[dialog_common::test]
    async fn it_builds_the_no_arg_query_from_descriptor_and_entity() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;
        let _ = await_promise(call_method(&tonk, "query", &[])).await;

        let body = host.last_query_body().expect("query dispatched");
        let terms = Reflect::get(&body, &"terms".into()).expect("terms");
        let this = Reflect::get(&terms, &"this".into()).expect("this term");
        // `serde-wasm-bindgen` renders the body as nested `Map`s, so
        // read `terms` as a Map to reach `this`.
        let this = if this.is_undefined() {
            let map: js_sys::Map = terms.dyn_into().expect("terms is a Map");
            map.get(&"this".into())
        } else {
            this
        };
        assert_eq!(this.as_string().as_deref(), Some("id:demo-counter"));
    }

    #[dialog_common::test]
    async fn it_resolves_query_to_the_canned_conclusions() {
        let host = FakeHost::install();
        let canned = js_sys::Array::new();
        canned.push(&JsValue::from_str("row"));
        host.set_query_result(canned.into());
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;
        let result = await_promise(call_method(&tonk, "query", &[]))
            .await
            .expect("query resolves");
        let arr: js_sys::Array = result.dyn_into().expect("array");
        assert_eq!(arr.length(), 1);
        assert_eq!(arr.get(0).as_string().as_deref(), Some("row"));
    }

    #[dialog_common::test]
    async fn it_routes_transact_to_tonk_claim() {
        let host = FakeHost::install();
        host.set_claim_result(JsValue::from_str("receipt"));
        let portal = mount_portal(&host, "<p>hi</p>", Some("id:x"), Some("m"), None);
        let tonk = await_bridge(&iframe_of(&portal)).await;

        let request = Object::new();
        let _ = Reflect::set(&request, &"assert".into(), &JsValue::from_str("something"));
        let result = await_promise(call_method(&tonk, "transact", &[request.into()]))
            .await
            .expect("transact resolves");
        assert_eq!(result.as_string().as_deref(), Some("receipt"));

        let body = host.last_claim_body().expect("claim dispatched");
        assert_eq!(
            get_str(&body, "assert").as_deref(),
            Some("something"),
            "the structured request is forwarded verbatim",
        );
    }

    #[dialog_common::test]
    async fn it_yields_pushed_subscribe_frames_as_dot_accessible_objects() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;

        let stream = call_method(&tonk, "subscribe", &[]);
        let get_reader: Function = Reflect::get(&stream, &"getReader".into())
            .expect("getReader")
            .dyn_into()
            .expect("fn");
        let reader = get_reader.call0(&stream).expect("reader");
        let read: Function = Reflect::get(&reader, &"read".into())
            .expect("read")
            .dyn_into()
            .expect("fn");

        // Start the read (pending), then push a host-shaped frame.
        let pending = read.call0(&reader).expect("read()");
        host.push_frame(&host_frame("id:demo-counter", 5));
        let result = await_promise(pending).await.expect("frame");

        let value = Reflect::get(&result, &"value".into()).expect("value");
        let rows: js_sys::Array = value.dyn_into().expect("Conclusion[]");
        let me = rows.get(0);
        // Dot access must work — the host delivers Maps/BigInts, the
        // bridge converts them to the same plain shape `query()` yields.
        assert_eq!(get_str(&me, "this").as_deref(), Some("id:demo-counter"));
        let fields = Reflect::get(&me, &"fields".into()).expect("fields");
        let count = Reflect::get(&fields, &"count".into()).expect("count");
        assert_eq!(count.as_f64(), Some(5.0), "integer field is a plain number");
    }

    #[dialog_common::test]
    async fn it_propagates_stream_cancel_to_the_host_subscription() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;

        let stream = call_method(&tonk, "subscribe", &[]);
        assert!(!host.cancelled(), "not cancelled before the author asks");

        let cancel: Function = Reflect::get(&stream, &"cancel".into())
            .expect("cancel")
            .dyn_into()
            .expect("fn");
        let _ = await_promise(cancel.call0(&stream).expect("cancel()")).await;
        sleep(5).await;

        assert!(
            host.cancelled(),
            "cancelling the stream must cancel the host subscription",
        );
    }

    #[dialog_common::test]
    async fn it_makes_tonk_available_synchronously_at_top_level() {
        let host = FakeHost::install();
        host.set_query_result(js_sys::Array::new().into());
        // Author code runs `tonk.query()` at the top level — no await
        // to reach `tonk`. We stash the returned promise for the test.
        let portal = mount_portal(
            &host,
            "<script>window.__q = tonk.query()</script>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let iframe = iframe_of(&portal);
        let _ = await_bridge(&iframe).await;

        // The stash is set synchronously when the author script runs;
        // poll briefly for the document to have executed it.
        let mut stashed = JsValue::UNDEFINED;
        for _ in 0..200 {
            if let Some(win) = iframe.content_window()
                && let Ok(v) = Reflect::get(&win, &"__q".into())
                && !v.is_undefined()
            {
                stashed = v;
                break;
            }
            sleep(5).await;
        }
        assert!(
            !stashed.is_undefined(),
            "author top-level `tonk.query()` should have produced a promise",
        );
        let result = await_promise(stashed).await.expect("query resolves");
        assert!(js_sys::Array::is_array(&result));
    }

    #[dialog_common::test]
    async fn it_cancels_subscriptions_when_content_changes() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;
        let _ = call_method(&tonk, "subscribe", &[]);
        assert!(!host.cancelled());

        portal
            .set_attribute("content", "<p>new</p>")
            .expect("change content");
        sleep(5).await;

        assert!(
            host.cancelled(),
            "a content reload must cancel the prior window's subscriptions",
        );
    }

    #[dialog_common::test]
    async fn it_cancels_subscriptions_and_unregisters_on_disconnect() {
        let host = FakeHost::install();
        let portal = mount_portal(
            &host,
            "<p>hi</p>",
            Some("id:demo-counter"),
            Some("counter"),
            Some(DESCRIPTOR),
        );
        let tonk = await_bridge(&iframe_of(&portal)).await;
        let _ = call_method(&tonk, "subscribe", &[]);

        portal.remove();
        sleep(5).await;

        assert!(
            host.cancelled(),
            "disconnect must cancel live subscriptions"
        );
        assert!(
            portal.query_selector("iframe").unwrap().is_none(),
            "disconnect detaches the iframe",
        );
    }
}
