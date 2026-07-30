//! `<tonk-page>` — surfaces the current location as a `mount` event.
//!
//! On connect it reads `window.location`, parses it into structured
//! fields, and dispatches a bubbling `mount` `CustomEvent` whose `detail`
//! carries the parsed URL. A declarative view binds a command to it with
//! `onmount=<command>`, exactly as it would bind `onclick`/`onsubmit`:
//!
//! ```html
//! <tonk-page onmount=tonk/join>
//!   <tonk-display model=tonk:join/status entity=tonk:join/status></tonk-display>
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
//! Join reads `dom.event.detail/href` as one value. Keeping the query and
//! fragment together avoids reconstructing bearer authority after parsing;
//! the service worker cannot otherwise see the fragment because browsers
//! strip it from requests.
//!
//! The element fires on connect. A detail-free `tonk:join-retry` event asks it
//! to rebuild this in-memory location detail and fire again without navigating
//! or putting the URL in the retry event.

use custom_elements::CustomElement;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    CustomEvent, CustomEventInit, HtmlElement, MutationObserver, MutationObserverInit, Url, window,
};

/// Per-element state. Holds the `MutationObserver` watching the enclosing
/// display's readiness marker (disconnected on disconnect) and its callback.
#[derive(Default)]
pub(crate) struct TonkPage {
    observer: Option<MutationObserver>,
    _callback: Option<Closure<dyn FnMut()>>,
    retry: Option<Closure<dyn FnMut(web_sys::Event)>>,
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
        let retry_host = this.clone();
        let retry = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            if let Some(detail) = location_detail() {
                dispatch_mount(&retry_host, &detail);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let _ = this
            .add_event_listener_with_callback("tonk:join-retry", retry.as_ref().unchecked_ref());
        self.retry = Some(retry);

        // Fire `mount` only once the `<tonk-display>` whose event delegate
        // handles this page's `onmount` binding is actually listening —
        // never on raw connect, when the delegate is still installing
        // asynchronously (after the template renders) and a `mount` would
        // hit no listener.
        //
        // That handler is the nearest ancestor `<tonk-display>`. It marks its
        // delegate installed with a persistent `data-bound` attribute (see
        // `DISPLAY_BOUND_ATTR`). We wait on that MARKER, not on the display's
        // one-shot `tonk-display:bound` event: the event is fragile — it can
        // fire before this element connects, or be missed when a view
        // reconcile swaps the page across the announcement (the exact failure
        // that stranded `onmount` forever). The marker is a persistent fact a
        // `MutationObserver` cannot miss.
        //
        // So: check the marker synchronously on connect (it may already be
        // set — e.g. we reconnected after the delegate installed), and
        // otherwise observe the enclosing display until it appears.
        // `try_fire_mount` fires only while the marker is present, and `fired`
        // makes it exactly once.
        let host = this.clone();
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));

        try_fire_mount(&host, &fired);
        if fired.get() {
            return;
        }

        let Some(display) = host.closest("tonk-display").ok().flatten() else {
            return;
        };
        let callback = {
            let host = host.clone();
            let fired = fired.clone();
            Closure::wrap(Box::new(move || {
                try_fire_mount(&host, &fired);
            }) as Box<dyn FnMut()>)
        };
        let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
            return;
        };
        let init = MutationObserverInit::new();
        init.set_attributes(true);
        init.set_attribute_filter(&js_sys::Array::of1(&JsValue::from_str(DISPLAY_BOUND_ATTR)));
        let _ = observer.observe_with_options(&display, &init);

        self.observer = Some(observer);
        self._callback = Some(callback);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(observer) = self.observer.take() {
            observer.disconnect();
        }
        self._callback = None;
        if let Some(retry) = self.retry.take() {
            let _ = this.remove_event_listener_with_callback(
                "tonk:join-retry",
                retry.as_ref().unchecked_ref(),
            );
        }
    }
}

/// Build the `mount` event `detail` from `window.location`: a flat,
/// URL-shaped record whose fields mirror the DOM `URL` interface, with
/// `searchParams` as a plain object (`Object.fromEntries`). `None` when
/// there is no window/location to read.
fn location_detail() -> Option<js_sys::Object> {
    // Prefer the host-forwarded location. `<tonk-page>` renders inside a sealed
    // `<tonk-site>` guest whose own `window.location` is `about:srcdoc` — it
    // carries none of the page's `?access`/`#seed`. The host injects its REAL
    // location into `window.tonk.context` ({ origin, path, search, hash }); when
    // present, rebuild the href from it. Fall back to `window.location` for the
    // unsealed/top-page case (and tests).
    let href = context_href().or_else(|| window()?.location().href().ok())?;
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

/// Rebuild the page href from the host-forwarded `window.tonk.context`.
///
/// A sealed guest can't read the real page URL off its `about:srcdoc`
/// location, so the host bridge forwards `{ origin, path, search, hash }` in
/// the `ready` handshake. Reassemble `origin + path + search + hash` into a
/// URL the caller can parse. Returns `None` when there is no guest context
/// (top-page/tests → caller falls back to `window.location`) or when the
/// forwarded origin is empty (a nested guest whose parent is itself an
/// `about:srcdoc` guest has no real location to rebuild).
fn context_href() -> Option<String> {
    let win = window()?;
    let tonk = js_sys::Reflect::get(&win, &JsValue::from_str("tonk")).ok()?;
    let context = js_sys::Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    if context.is_undefined() || context.is_null() {
        return None;
    }
    let field = |key: &str| {
        js_sys::Reflect::get(&context, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
    };
    let origin = field("origin")?;
    if origin.is_empty() {
        return None;
    }
    let path = field("path").unwrap_or_default();
    let search = field("search").unwrap_or_default();
    let hash = field("hash").unwrap_or_default();
    Some(format!("{origin}{path}{search}{hash}"))
}

/// Set a string property on an object.
fn set(object: &js_sys::Object, key: &str, value: &str) {
    let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), &JsValue::from_str(value));
}

/// The persistent readiness marker a `<tonk-display>` stamps on its host
/// once its event delegate is installed. DOM contract shared with
/// `BOUND_ATTR` in the `tonk-display` crate — keep the string in sync.
const DISPLAY_BOUND_ATTR: &str = "data-bound";

/// Whether the nearest ancestor `<tonk-display>` — the one whose delegate
/// handles this page's `onmount` binding — has installed its delegate, i.e.
/// carries the readiness marker. `false` when there is no such ancestor
/// (nothing would handle the command) or it has not bound yet.
fn enclosing_display_ready(host: &HtmlElement) -> bool {
    host.closest("tonk-display")
        .ok()
        .flatten()
        .is_some_and(|display| display.has_attribute(DISPLAY_BOUND_ATTR))
}

/// Dispatch `mount` exactly once, and only after the enclosing display's
/// delegate is ready to receive it. A no-op if already fired, if that
/// display has not bound yet, or if the location can't be read — the marker
/// observer will call again when the display's `data-bound` next changes.
fn try_fire_mount(host: &HtmlElement, fired: &std::rc::Rc<std::cell::Cell<bool>>) {
    if fired.get() {
        return;
    }
    if !enclosing_display_ready(host) {
        return;
    }
    if fired.replace(true) {
        return;
    }
    // `mount` is a lifecycle signal — fire it once the enclosing display's
    // delegate is ready. The location detail is best-effort: the top page and
    // the `/join` flow carry a real URL, but inside a deeply nested sealed
    // guest (the space view is `blob:null` inside `about:srcdoc`) the location
    // is opaque and the host forwards no origin, so there is nothing to parse.
    // The command bound here (e.g. `tonk:invite`) reads no `dom.event.detail`
    // fields, so fire with an empty detail rather than stranding it — never
    // firing was the bug that hung the "generating link" canvas.
    let detail = location_detail().unwrap_or_default();
    dispatch_mount(host, &detail);
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

    /// Let queued `MutationObserver` microtasks flush before asserting.
    async fn tick() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let _ = window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0);
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }

    /// A `<tonk-page>` fires `mount` once its enclosing `<tonk-display>` marks
    /// its delegate installed (`data-bound`) — not on raw connect, when the
    /// marker is still absent. The observer picks up the marker landing after
    /// connect; the event bubbles and carries the parsed location.
    #[dialog_common::test]
    async fn it_fires_when_the_display_marks_bound_after_connect() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        // The enclosing display whose delegate would handle `onmount`.
        let wrapper = document.create_element("tonk-display").unwrap();
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

        // Not on connect: the display hasn't marked its delegate ready.
        assert!(
            seen.borrow().is_none(),
            "tonk-page must NOT fire mount before its display marks bound"
        );

        // The display installs its delegate: the marker lands. The page's
        // observer picks it up on the next microtask.
        wrapper.set_attribute(DISPLAY_BOUND_ATTR, "").unwrap();
        tick().await;

        let detail = seen.borrow().clone();
        assert!(
            detail.is_some(),
            "tonk-page must dispatch a bubbling mount once its display marks bound"
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

    /// When the enclosing display is *already* marked bound at connect — the
    /// marker predated this element, or a prior instance set it before a view
    /// reconcile replaced the page — the page fires `mount` synchronously on
    /// connect. This is the case the old event-only wait stranded.
    #[dialog_common::test]
    async fn it_fires_on_connect_when_the_display_is_already_bound() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let wrapper = document.create_element("tonk-display").unwrap();
        wrapper.set_attribute(DISPLAY_BOUND_ATTR, "").unwrap();
        body.append_child(&wrapper).unwrap();

        let seen = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let seen_cb = seen.clone();
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |_event: Event| {
            seen_cb.set(seen_cb.get() + 1);
        }) as Box<dyn FnMut(Event)>);
        wrapper
            .add_event_listener_with_callback("mount", closure.as_ref().unchecked_ref())
            .unwrap();

        let page = document.create_element("tonk-page").unwrap();
        wrapper.append_child(&page).unwrap();

        assert_eq!(
            seen.get(),
            1,
            "page must fire mount on connect when its display is already bound"
        );

        wrapper.remove();
        drop(closure);
    }

    /// The page never fires while the marker is absent (a benign attribute
    /// change is not the readiness signal), fires once the marker lands, and
    /// then exactly once even as the marker toggles across a delegate rebuild.
    #[dialog_common::test]
    async fn it_fires_exactly_once_and_never_while_unmarked() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let wrapper = document.create_element("tonk-display").unwrap();
        body.append_child(&wrapper).unwrap();

        let seen = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let seen_cb = seen.clone();
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |_event: Event| {
            seen_cb.set(seen_cb.get() + 1);
        }) as Box<dyn FnMut(Event)>);
        wrapper
            .add_event_listener_with_callback("mount", closure.as_ref().unchecked_ref())
            .unwrap();

        let page = document.create_element("tonk-page").unwrap();
        wrapper.append_child(&page).unwrap();

        // A non-readiness attribute change must not fire mount.
        wrapper.set_attribute("data-state", "loading").unwrap();
        tick().await;
        assert_eq!(
            seen.get(),
            0,
            "no fire while the readiness marker is absent"
        );

        // Marker lands -> fires once.
        wrapper.set_attribute(DISPLAY_BOUND_ATTR, "").unwrap();
        tick().await;
        assert_eq!(seen.get(), 1, "fires once the display marks bound");

        // Marker clears then re-lands (a delegate rebuild) -> still once.
        wrapper.remove_attribute(DISPLAY_BOUND_ATTR).unwrap();
        tick().await;
        wrapper.set_attribute(DISPLAY_BOUND_ATTR, "").unwrap();
        tick().await;
        assert_eq!(seen.get(), 1, "mount must fire exactly once");

        wrapper.remove();
        drop(closure);
    }

    fn get(object: &js_sys::Object, key: &str) -> String {
        js_sys::Reflect::get(object, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default()
    }

    /// Inside a sealed guest, `window.location` is `about:srcdoc`, so the join
    /// trigger must read the host-forwarded location from `window.tonk.context`
    /// instead — otherwise the invite's `?access`/`#seed` never reach the
    /// command. With that context set, `location_detail` reconstructs the real
    /// `search`, `hash`, and `searchParams`.
    #[dialog_common::test]
    async fn it_reads_the_forwarded_location_from_guest_context() {
        let win = window().unwrap();

        // Simulate the host `ready` handshake: `window.tonk.context` carries the
        // parent page's real location (the guest's own is `about:srcdoc`).
        let tonk = js_sys::Object::new();
        let context = js_sys::Object::new();
        set(&context, "origin", "https://hub.example");
        set(&context, "path", "/join");
        set(&context, "search", "?access=abc&remote=xyz");
        set(&context, "hash", "#seed123");
        js_sys::Reflect::set(&tonk, &JsValue::from_str("context"), &context).unwrap();
        js_sys::Reflect::set(&win, &JsValue::from_str("tonk"), &tonk).unwrap();

        let detail = location_detail().expect("detail rebuilt from guest context");
        assert_eq!(
            get(&detail, "search"),
            "?access=abc&remote=xyz",
            "search comes from the forwarded context, not about:srcdoc",
        );
        assert_eq!(
            get(&detail, "hash"),
            "#seed123",
            "the #seed fragment is recovered from the forwarded context",
        );
        let params = js_sys::Reflect::get(&detail, &JsValue::from_str("searchParams"))
            .ok()
            .and_then(|v| v.dyn_into::<js_sys::Object>().ok())
            .expect("searchParams is a plain object");
        assert_eq!(
            get(&params, "access"),
            "abc",
            "searchParams is reconstructed from the forwarded search",
        );

        // Restore so other tests fall back to `window.location` as usual.
        js_sys::Reflect::set(&win, &JsValue::from_str("tonk"), &JsValue::UNDEFINED).unwrap();
    }
}
