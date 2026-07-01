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
use web_sys::{CustomEvent, Element, Event, HtmlElement, window};

/// The consumer event names, matching `tonk_host::events`.
const QUERY: &str = "tonk-query";
const CLAIM: &str = "tonk-claim";
const EVALUATE: &str = "tonk-evaluate";
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
        // Bind the per-tab `site` entity: any descendant that opted in with
        // `data-tonk-entity="site"` gets its `entity` set to the host's site
        // (the X-Tonk-Site the host stamps on the HTTP queries it issues on this
        // guest's behalf — `window.tonk.context.site`). That is how the routing
        // shell (`<tonk-display model=tonk:site data-tonk-entity="site">`)
        // resolves its own tab's location/route: the site entity carries the
        // `tonk:site` facts the SW stamped. Deferred until `window.tonk.ready`
        // resolves, since the context (with the site) arrives asynchronously
        // after the host's ready envelope.
        let root = this.clone();
        spawn_local(async move {
            await_tonk_ready().await;
            fill_site_entities(&root);
        });

        let mut installed = Vec::new();
        for name in [QUERY, CLAIM, EVALUATE, SUBSCRIBE, UNSUBSCRIBE] {
            let listener = make_listener(name);
            let _ = this.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
            installed.push((name.to_owned(), listener));
        }
        // Navigation relay: a link click inside the opaque guest can't move
        // the parent, so catch it at the document and post the href over the
        // bridge for the host to perform. Capture phase so it runs before any
        // app handler and before the (blocked-anyway) native navigation.
        if let Some(doc) = window().and_then(|w| w.document()) {
            let listener = make_nav_listener();
            let _ = doc.add_event_listener_with_callback_and_bool(
                "click",
                listener.as_ref().unchecked_ref(),
                true,
            );
            installed.push(("click".to_owned(), listener));
        }
        *self.listeners.borrow_mut() = installed;
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        for (name, listener) in self.listeners.borrow_mut().drain(..) {
            if name == "click" {
                if let Some(doc) = window().and_then(|w| w.document()) {
                    let _ = doc.remove_event_listener_with_callback_and_bool(
                        "click",
                        listener.as_ref().unchecked_ref(),
                        true,
                    );
                }
            } else {
                let _ = this
                    .remove_event_listener_with_callback(&name, listener.as_ref().unchecked_ref());
            }
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
        // Claim the event: stop it bubbling to any OUTER `<tonk-host>` proxy so
        // a single query/subscribe is relayed exactly once. This matters when
        // hosts nest inside the guest — e.g. the FAB menu wraps each row in its
        // own `<tonk-host><tonk-repository name=…>` to read another space's
        // label; without this the outer fab-view host would relay the same
        // operation a second time. Mirrors the real host's `claim_event`.
        custom.stop_propagation();
        match name {
            QUERY => relay_one_shot(&custom, &detail, &tonk, "query", "query"),
            CLAIM => relay_one_shot(&custom, &detail, &tonk, "transact", "request"),
            // Evaluate carries two fields (`document` + `transact`), so the whole
            // detail object is relayed rather than a single keyed arg.
            EVALUATE => relay_evaluate(&custom, &detail, &tonk),
            SUBSCRIBE => relay_subscribe(&custom, &detail, &tonk),
            UNSUBSCRIBE => relay_unsubscribe(&custom, &detail),
            _ => {}
        }
    }) as Box<dyn FnMut(Event)>)
}

/// Build the document click listener that relays in-guest link navigation
/// to the host over `window.tonk.navigate`.
fn make_nav_listener() -> Closure<dyn FnMut(Event)> {
    Closure::wrap(Box::new(move |event: Event| {
        // Leave modified clicks (new tab/window, middle-click) to the
        // browser — though in the sandbox they're inert, honoring them keeps
        // behavior predictable.
        if let Ok(mouse) = event.clone().dyn_into::<web_sys::MouseEvent>()
            && (mouse.meta_key() || mouse.ctrl_key() || mouse.shift_key() || mouse.button() != 0)
        {
            return;
        }
        let Some(href) = event.target().and_then(closest_anchor_href) else {
            return;
        };
        // Only relay in-app navigations: a path (`/…`) or a same-document
        // href. Skip fragments, mailto:, external schemes, etc.
        if !href.starts_with('/') || href.starts_with("//") {
            return;
        }
        event.prevent_default();
        if let Some(tonk) = window_tonk()
            && let Some(navigate) = get_fn(&tonk, "navigate")
        {
            let _ = navigate.call1(&tonk, &JsValue::from_str(&href));
        }
    }) as Box<dyn FnMut(Event)>)
}

/// Walk up from an event target to the nearest `<a>` and read its `href`
/// attribute (the raw attribute, not the resolved `.href` which an opaque
/// origin mangles to `null/…`).
fn closest_anchor_href(target: web_sys::EventTarget) -> Option<String> {
    let element = target.dyn_into::<web_sys::Element>().ok()?;
    let anchor = element.closest("a[href]").ok()??;
    anchor.get_attribute("href").filter(|h| !h.is_empty())
}

/// `window.tonk` if the portal bootstrap installed it.
fn window_tonk() -> Option<Object> {
    let win = window()?;
    Reflect::get(&win, &JsValue::from_str("tonk"))
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
}

/// The host's per-tab `site` entity (`window.tonk.context.site`), the
/// `X-Tonk-Site` the SW keys this tab's `tonk:site` facts on. Lives on `context`
/// (not `tonk` directly) because it is the HOST's id, delivered with the rest of
/// the context in the `ready` envelope.
fn site_id() -> Option<String> {
    let tonk: JsValue = window_tonk()?.into();
    let context = Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    Reflect::get(&context, &JsValue::from_str("site"))
        .ok()?
        .as_string()
        .filter(|s| !s.is_empty())
}

/// Await `window.tonk.ready` (resolves once the host's `ready` envelope, with
/// the context, has arrived). Resolves immediately if the bridge is absent.
async fn await_tonk_ready() {
    let Some(tonk) = window_tonk() else {
        return;
    };
    let Ok(ready) = Reflect::get(&tonk, &JsValue::from_str("ready")) else {
        return;
    };
    if let Ok(promise) = ready.dyn_into::<Promise>() {
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

/// Set `entity` to the host's `site` entity on every descendant of `root` that
/// opted in with `data-tonk-entity="site"`. Idempotent: `<tonk-display>`
/// observes `entity`, so a re-set after upgrade just re-resolves to the same
/// value.
fn fill_site_entities(root: &HtmlElement) {
    let Some(site) = site_id() else {
        return;
    };
    let Ok(matches) = root.query_selector_all("[data-tonk-entity=\"site\"]") else {
        return;
    };
    for i in 0..matches.length() {
        if let Some(el) = matches.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
            let _ = el.set_attribute("entity", &site);
        }
    }
}

/// Build the optional per-call routing context (`{space, branch}`) from a
/// consumer event's detail — the values its in-guest `<tonk-repository name=…>`
/// / `<tonk-branch name=…>` ancestors annotated during bubble phase. Forwarded
/// as the second argument to `window.tonk.query` / `.subscribe` so the host can
/// route the query at that repository. The host honors it ONLY for a privileged
/// (cross-repo) portal — the FAB — and ignores it otherwise, so forwarding is
/// always safe. Empty when no `<tonk-repository>` scoped the query (the common
/// case), leaving the host on its handshake context.
fn route_ctx(detail: &JsValue) -> JsValue {
    let ctx = Object::new();
    for key in ["space", "branch"] {
        if let Some(value) = Reflect::get(detail, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
        {
            let _ = Reflect::set(&ctx, &JsValue::from_str(key), &JsValue::from_str(&value));
        }
    }
    // Forward the `profile` flag (a `<tonk-repository profile>` ancestor) so the
    // host routes the query at the profile-as-repository endpoint. Like `space`,
    // the host honors it only for a privileged portal.
    if Reflect::get(detail, &JsValue::from_str("profile"))
        .map(|v| v.is_truthy())
        .unwrap_or(false)
    {
        let _ = Reflect::set(&ctx, &JsValue::from_str("profile"), &JsValue::TRUE);
    }
    ctx.into()
}

/// Call `tonk[method](detail[arg_key], routeCtx)`, expect a Promise, write it
/// into `detail.result`, and preventDefault so the consumer awaits it.
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
    let Ok(result) = func.call2(tonk, &arg, &route_ctx(detail)) else {
        return;
    };
    // result is a Promise<rows|receipt>; that's exactly what detail.result
    // must be.
    let _ = Reflect::set(detail, &JsValue::from_str("result"), &result);
    event.prevent_default();
}

/// Relay a `tonk-evaluate` consumer event to `window.tonk.evaluate(detail)`.
/// Unlike query/claim, evaluate carries two fields (`document`, `transact`), so
/// the whole detail object is passed; the bridge forwards it to the host's real
/// `<tonk-host>` consumer, which performs the typed evaluate and returns the
/// promise the consumer awaits as `detail.result`.
fn relay_evaluate(event: &CustomEvent, detail: &JsValue, tonk: &Object) {
    let Some(func) = get_fn(tonk, "evaluate") else {
        return;
    };
    let Ok(result) = func.call1(tonk, detail) else {
        return;
    };
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
    let Ok(stream) = subscribe.call2(tonk, &query, &route_ctx(detail)) else {
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
