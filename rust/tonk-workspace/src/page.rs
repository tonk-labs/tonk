//! `<tonk-page>` — surfaces the current location as a `mount` event.
//!
//! On connect it reads `window.location`, parses it into structured
//! fields, and dispatches a bubbling `mount` `CustomEvent` whose `detail`
//! carries the parsed URL. A declarative view binds a command to it with
//! `onmount=<command>`, exactly as it would bind `onclick`/`onsubmit`:
//!
//! ```html
//! <tonk-page onmount=tonk/join>
//!   <tonk-display concept=tonk:join/status this=tonk:join/status></tonk-display>
//! </tonk-page>
//! ```
//!
//! The event `detail` is the parsed location, Elm-`Browser.application`
//! style — the URL is *input passed as data*, not an ambient global the
//! command handler reaches for. It is a flat, URL-shaped record whose
//! field names mirror the DOM `URL` interface, so every value is a plain
//! property read (no live `URLSearchParams`, no method calls):
//!
//! ```text
//! detail = {
//!   href:         "https://host/join?access=abc&remote=…#seed",
//!   origin:       "https://host",
//!   pathname:     "/join",
//!   search:       "?access=abc&remote=…",   // faithful to URL.search
//!   hash:         "#seed",                   // faithful to URL.hash (incl. #)
//!   searchParams: { access: "abc", remote: "…" },  // Object.fromEntries
//! }
//! ```
//!
//! So a command reads pieces via plain property paths:
//! `dom.event.detail/hash` (the `#seed`),
//! `dom.event.detail.search-params/access`, etc. — never touching
//! `window`, never re-parsing a raw string. The `#hash` matters because
//! the service worker can't see it (browsers strip fragments from
//! requests); this element is the page-side courier that brings it into
//! the command.
//!
//! Fires once: the element connects once when the view mounts (chrome is
//! not re-created on incremental reconcile), so the `mount` event — and
//! any command bound to it — fires exactly once per page load.

use custom_elements::CustomElement;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CustomEvent, CustomEventInit, Event, HtmlElement, Url, window};

/// Per-element state. Holds the `tonk-display:bound` listener (removed on
/// disconnect) and a fired-once guard.
#[derive(Default)]
pub(crate) struct TonkPage {
    bound_listener: Option<Closure<dyn FnMut(Event)>>,
}

impl CustomElement for TonkPage {
    fn shadow() -> bool {
        // Light DOM: the `mount` event must bubble through the element's
        // descendants to the `<tonk-display>` host whose delegate handles
        // `onmount` bindings.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Fire `mount` only once the host `<tonk-display>`'s event delegate
        // is INSTALLED — not on raw connect. The delegate attaches its
        // listeners asynchronously *after* the template renders (which is
        // what mounts this element), so a `mount` fired on connect would
        // hit no listener. The display announces install with
        // `tonk-display:bound`; we wait for that, then fire once.
        //
        // Chrome mounts before its delegate binds, so this ordering always
        // holds for a `<tonk-page>` inside a view.
        let host = this.clone();
        // Fire-once guard, checked and set *before* dispatching `mount`.
        // The page carries several `<tonk-display>`s, each announcing its
        // own `tonk-display:bound`, and dispatching `mount` can synchronously
        // trigger more renders/binds — so the listener can re-enter. Setting
        // the flag first makes every re-entry an immediate no-op.
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        let listener = Closure::wrap(Box::new(move |_event: Event| {
            if fired.replace(true) {
                return;
            }
            if let Some(detail) = location_detail() {
                dispatch_mount(&host, &detail);
            }
        }) as Box<dyn FnMut(Event)>);

        // Listen on the document during the capture phase so we catch the
        // `tonk-display:bound` event dispatched on the ancestor host (it
        // does not bubble down).
        if let Some(document) = window().and_then(|w| w.document()) {
            let options = web_sys::AddEventListenerOptions::new();
            options.set_capture(true);
            let _ = document.add_event_listener_with_callback_and_add_event_listener_options(
                "tonk-display:bound",
                listener.as_ref().unchecked_ref(),
                &options,
            );
        }
        self.bound_listener = Some(listener);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(listener) = self.bound_listener.take()
            && let Some(document) = window().and_then(|w| w.document())
        {
            let _ = document.remove_event_listener_with_callback_and_bool(
                "tonk-display:bound",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}

/// Build the `mount` event `detail` from `window.location`: a flat,
/// URL-shaped record whose fields mirror the DOM `URL` interface, with
/// `searchParams` as a plain object (`Object.fromEntries`). `None` when
/// there is no window/location to read.
fn location_detail() -> Option<js_sys::Object> {
    let href = window()?.location().href().ok()?;
    // `Url` parses href into the standard URL fields; we copy them onto a
    // plain object so the event-read walk does only property reads.
    let url = Url::new(&href).ok()?;
    let detail = js_sys::Object::new();

    // Faithful to the URL interface — `search`/`hash` keep their leading
    // `?`/`#`; consumers strip if they want the bare value.
    set(&detail, "href", &url.href());
    set(&detail, "origin", &url.origin());
    set(&detail, "pathname", &url.pathname());
    set(&detail, "search", &url.search());
    set(&detail, "hash", &url.hash());

    // `searchParams` as a plain { key: value } object so a command reads
    // `dom.event.detail.search-params/access` as a property walk (no live
    // `URLSearchParams`, no `.get()` call).
    let search_params = js_sys::Object::new();
    let entries = url.search_params().entries();
    while let Ok(next) = entries.next() {
        if next.done() {
            break;
        }
        if let Ok(pair) = next.value().dyn_into::<js_sys::Array>() {
            let key = pair.get(0).as_string().unwrap_or_default();
            let value = pair.get(1).as_string().unwrap_or_default();
            if !key.is_empty() {
                set(&search_params, &key, &value);
            }
        }
    }
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("searchParams"), &search_params);

    Some(detail)
}

/// Set a string property on an object.
fn set(object: &js_sys::Object, key: &str, value: &str) {
    let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), &JsValue::from_str(value));
}

/// Dispatch a bubbling `mount` `CustomEvent` carrying `detail` from `host`.
fn dispatch_mount(host: &HtmlElement, detail: &js_sys::Object) {
    let init = CustomEventInit::new();
    init.set_bubbles(true);
    init.set_detail(detail);
    if let Ok(event) = CustomEvent::new_with_event_init_dict("mount", &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// Register `<tonk-page>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkPage::define("tonk-page");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-page").is_undefined()
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    use web_sys::Event;

    wasm_bindgen_test_configure!(run_in_browser);

    /// A connected `<tonk-page>` fires `mount` only after the host's
    /// delegate announces install via `tonk-display:bound` — NOT on raw
    /// connect (the delegate isn't listening yet then). The mount event
    /// bubbles and carries the parsed location.
    #[dialog_common::test]
    async fn it_dispatches_mount_after_the_host_delegate_binds() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        // A wrapper to catch the bubbled event, like a <tonk-display> host.
        let wrapper = document.create_element("div").unwrap();
        body.append_child(&wrapper).unwrap();

        let seen: std::rc::Rc<std::cell::RefCell<Option<js_sys::Object>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let seen_cb = seen.clone();
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: Event| {
            if let Ok(ce) = event.dyn_into::<CustomEvent>()
                && let Ok(obj) = ce.detail().dyn_into::<js_sys::Object>()
            {
                *seen_cb.borrow_mut() = Some(obj);
            }
        }) as Box<dyn FnMut(Event)>);
        wrapper
            .add_event_listener_with_callback("mount", closure.as_ref().unchecked_ref())
            .unwrap();

        let page = document.create_element("tonk-page").unwrap();
        wrapper.append_child(&page).unwrap();

        // Nothing yet — the page waits for the delegate-ready signal.
        assert!(
            seen.borrow().is_none(),
            "tonk-page must NOT fire mount on connect, before the delegate binds"
        );

        // The host's delegate announces it's installed.
        wrapper
            .dispatch_event(&CustomEvent::new("tonk-display:bound").unwrap())
            .unwrap();

        let detail = seen.borrow().clone();
        assert!(
            detail.is_some(),
            "tonk-page must dispatch a bubbling mount event once the delegate binds"
        );
        let detail = detail.unwrap();

        // `href` and `pathname` mirror the URL interface and are always
        // present; `searchParams` is a (possibly empty) plain object.
        let href = get(&detail, "href");
        assert!(
            href.starts_with("http"),
            "detail.href should be the page URL, got {href:?}"
        );
        assert!(
            get(&detail, "pathname").starts_with('/'),
            "detail.pathname should be a path",
        );
        assert!(
            js_sys::Reflect::get(&detail, &JsValue::from_str("searchParams"))
                .ok()
                .and_then(|v| v.dyn_into::<js_sys::Object>().ok())
                .is_some(),
            "detail.searchParams should be a plain object",
        );

        wrapper.remove();
        drop(closure);
    }

    fn get(object: &js_sys::Object, key: &str) -> String {
        js_sys::Reflect::get(object, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    }
}
