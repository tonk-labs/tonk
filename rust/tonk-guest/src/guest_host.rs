//! `<tonk-host>` proxy for the sealed guest iframe.
//!
//! Inside the sealed (opaque-origin) guest, a real `<tonk-display>` still
//! dispatches its `tonk-query` / `tonk-subscribe` / `tonk-claim` consumer
//! events expecting a `<tonk-host>` ancestor to service them (see
//! `tonk-host/src/consumer.rs`). But the guest has no service-worker
//! access — its only channel out is `window.tonk`, the MessageChannel
//! bridge the portal bootstrap installs (`tonk-portal/src/bridge.rs`).
//!
//! This element IS that ancestor. It catches the four consumer events and
//! relays them to `window.tonk`:
//!
//! - `tonk-query`     → `window.tonk.query(detail.query)`   → `detail.result`
//! - `tonk-claim`     → `window.tonk.transact(detail.request)` → `detail.result`
//! - `tonk-subscribe` → `window.tonk.subscribe(detail.query)`; each stream
//!   frame is delivered by calling `consumer.reset(rows, {tag})` back on the
//!   dispatching element; writes `detail.subscription = { cancel }`.
//! - `tonk-unsubscribe` → cancels the matching stream reader.
//!
//! It is the mirror image of the portal's *parent* relay: the parent turns
//! `window.tonk` envelopes into consumer events bubbling to the real outer
//! `<tonk-host>`; this turns consumer events into `window.tonk` calls. With
//! it present, a real `<tonk-display>` renders in the guest unchanged.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{CustomEvent, Event, HtmlElement, window};

/// The four consumer event names, matching `tonk_host::events`.
const QUERY: &str = "tonk-query";
const CLAIM: &str = "tonk-claim";
const SUBSCRIBE: &str = "tonk-subscribe";
const UNSUBSCRIBE: &str = "tonk-unsubscribe";

/// An installed event listener: the event name it's bound to and the
/// closure kept alive for as long as it's attached.
type Listener = (String, Closure<dyn FnMut(Event)>);

/// Per-element listeners, dropped on disconnect.
#[derive(Default)]
pub(crate) struct GuestHost {
    listeners: RefCell<Vec<Listener>>,
}

impl CustomElement for GuestHost {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        let mut installed = Vec::new();
        for name in [QUERY, CLAIM, SUBSCRIBE, UNSUBSCRIBE] {
            let listener = make_listener(name);
            let _ = this.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
            installed.push((name.to_owned(), listener));
        }
        *self.listeners.borrow_mut() = installed;
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        for (name, listener) in self.listeners.borrow_mut().drain(..) {
            let _ =
                this.remove_event_listener_with_callback(&name, listener.as_ref().unchecked_ref());
        }
    }
}

/// Build the listener for one event name. It claims the event
/// (`preventDefault`, so the consumer sees it as handled) and relays to
/// `window.tonk`.
fn make_listener(name: &'static str) -> Closure<dyn FnMut(Event)> {
    Closure::wrap(Box::new(move |event: Event| {
        let Ok(custom) = event.clone().dyn_into::<CustomEvent>() else {
            return;
        };
        let detail = custom.detail();
        let Some(tonk) = window_tonk() else {
            // No bridge — leave the event unhandled so the consumer
            // surfaces "no <tonk-host> ancestor".
            return;
        };
        match name {
            QUERY => relay_one_shot(&custom, &detail, &tonk, "query", "query"),
            CLAIM => relay_one_shot(&custom, &detail, &tonk, "transact", "request"),
            SUBSCRIBE => relay_subscribe(&custom, &detail, &tonk),
            UNSUBSCRIBE => relay_unsubscribe(&custom, &detail),
            _ => {}
        }
    }) as Box<dyn FnMut(Event)>)
}

/// `window.tonk` if the portal bootstrap installed it.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
}

/// Call `tonk[method](detail[arg_key])`, expect a Promise, write it into
/// `detail.result`, and preventDefault so the consumer awaits it.
fn relay_one_shot(
    event: &CustomEvent,
    detail: &JsValue,
    tonk: &Object,
    method: &str,
    arg_key: &str,
) {
    let Some(func) = get_fn(tonk, method) else {
        return;
    };
    let arg = Reflect::get(detail, &JsValue::from_str(arg_key)).unwrap_or(JsValue::UNDEFINED);
    let Ok(result) = func.call1(tonk, &arg) else {
        return;
    };
    // result is a Promise<rows|receipt>; that's exactly what detail.result
    // must be.
    let _ = Reflect::set(detail, &JsValue::from_str("result"), &result);
    event.prevent_default();
}

/// `tonk.subscribe(detail.query)` → a ReadableStream. Read it in a loop,
/// delivering each frame via `consumer.reset(rows, {tag})`. Writes
/// `detail.subscription = { cancel }`.
fn relay_subscribe(event: &CustomEvent, detail: &JsValue, tonk: &Object) {
    let Some(subscribe) = get_fn(tonk, "subscribe") else {
        return;
    };
    let query = Reflect::get(detail, &JsValue::from_str("query")).unwrap_or(JsValue::UNDEFINED);
    let Ok(stream) = subscribe.call1(tonk, &query) else {
        return;
    };
    // The element the event was dispatched on — frames are delivered back
    // to it via its `reset` method.
    let Some(consumer) = event
        .target()
        .and_then(|t| t.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    let tag = Reflect::get(detail, &JsValue::from_str("tag")).unwrap_or(JsValue::UNDEFINED);

    // Get a reader off the ReadableStream and pump frames.
    let cancelled = Rc::new(RefCell::new(false));
    let reader = match Reflect::get(&stream, &JsValue::from_str("getReader"))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
        .and_then(|f| f.call0(&stream).ok())
    {
        Some(r) => r,
        None => return,
    };
    pump(reader.clone(), consumer, tag, cancelled.clone());

    // detail.subscription = { cancel }
    let subscription = Object::new();
    let cancel_flag = cancelled.clone();
    let reader_for_cancel = reader.clone();
    let cancel = Closure::wrap(Box::new(move || {
        *cancel_flag.borrow_mut() = true;
        if let Some(cancel_fn) = Reflect::get(&reader_for_cancel, &JsValue::from_str("cancel"))
            .ok()
            .and_then(|v| v.dyn_into::<Function>().ok())
        {
            let _ = cancel_fn.call0(&reader_for_cancel);
        }
    }) as Box<dyn FnMut()>);
    let _ = Reflect::set(
        &subscription,
        &JsValue::from_str("cancel"),
        cancel.as_ref().unchecked_ref(),
    );
    cancel.forget();
    let _ = Reflect::set(detail, &JsValue::from_str("subscription"), &subscription);
    event.prevent_default();
}

/// Read frames off `reader` and call `consumer.reset(value, {tag})` for
/// each until done or cancelled.
fn pump(reader: JsValue, consumer: HtmlElement, tag: JsValue, cancelled: Rc<RefCell<bool>>) {
    spawn_local(async move {
        loop {
            if *cancelled.borrow() {
                return;
            }
            let read = match Reflect::get(&reader, &JsValue::from_str("read"))
                .ok()
                .and_then(|v| v.dyn_into::<Function>().ok())
            {
                Some(f) => f,
                None => return,
            };
            let Ok(promise) = read.call0(&reader) else {
                return;
            };
            let Ok(promise) = promise.dyn_into::<Promise>() else {
                return;
            };
            let result = match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(r) => r,
                Err(_) => return,
            };
            let done = Reflect::get(&result, &JsValue::from_str("done"))
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if done {
                return;
            }
            let value = Reflect::get(&result, &JsValue::from_str("value")).unwrap_or(JsValue::NULL);
            // consumer.reset(value, { tag })
            if let Some(reset) = Reflect::get(&consumer, &JsValue::from_str("reset"))
                .ok()
                .and_then(|v| v.dyn_into::<Function>().ok())
            {
                let opts = Object::new();
                let _ = Reflect::set(&opts, &JsValue::from_str("tag"), &tag);
                let _ = reset.call2(&consumer, &value, &opts);
            }
        }
    });
}

/// `tonk-unsubscribe` — the consumer's Subscription.cancel already calls
/// the reader's cancel via the closure we stored, so this is a no-op
/// backstop. (The consumer may also drop the handle, which calls cancel.)
fn relay_unsubscribe(event: &CustomEvent, _detail: &JsValue) {
    event.prevent_default();
}

/// Read `tonk[name]` as a Function.
fn get_fn(tonk: &Object, name: &str) -> Option<Function> {
    Reflect::get(tonk, &JsValue::from_str(name))
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
}

/// Register `<tonk-host>` (the guest proxy). Idempotent. NOTE: this defines
/// `tonk-host` to the GUEST'S proxy, so it must run in the guest only —
/// never alongside the real `tonk_host::register()`.
pub fn register() {
    if already_registered() {
        return;
    }
    GuestHost::define("tonk-host");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-host").is_undefined()
}
