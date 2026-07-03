//! `<tonk-default-remote>` — fills a sibling form input with this server's
//! UCAN access-service URL.
//!
//! The create / enable-sync forms collect a remote URL the user can
//! type. Most of the time they want *this* server, but notation has no
//! `window.origin` and no inline JS, so a static template can't encode
//! `origin + /ucan/`. This dumb element bridges that gap: by default it
//! renders a button and, on click, writes the URL into the form control
//! named by its `field` attribute (default `remote`), resolved within the
//! closest `<form>`. With the `auto` attribute, it writes the URL on connect
//! and renders no button, for flows where the default server is policy.
//!
//! Like [`super::share`], it holds no app policy — just the
//! origin-resolution a static template can't do. The button's label is
//! the element's own text content (defaulting to "Use this server").

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Element, Event, HtmlElement, window};

/// A retained click-listener closure, kept alive for the element's
/// lifetime so the listener stays valid.
type ClickClosure = Rc<RefCell<Option<Closure<dyn FnMut(Event)>>>>;

/// CSS class the consuming view styles.
const BUTTON: &str = "workspace__default-remote";

/// Path of the UCAN access service, resolved against the page origin —
/// the same path `tonk-ui`'s `init()` wires for the `home` space.
const ACCESS_SERVICE_PATH: &str = "/ucan/";

/// The form-control `name` filled when the element sets no `field`.
const DEFAULT_FIELD: &str = "remote";

/// Attribute that makes the element silently fill the target on connect.
const AUTO_ATTR: &str = "auto";

/// Fallback button label when the element carries no text.
const DEFAULT_LABEL: &str = "Use this server";

/// Per-element state. Holds the click closure so it lives as long as
/// the element and drops on disconnect.
#[derive(Default)]
pub(crate) struct TonkDefaultRemote {
    click: ClickClosure,
}

impl CustomElement for TonkDefaultRemote {
    fn shadow() -> bool {
        // Light DOM: the consuming view styles the button and the
        // element must see its `<form>` ancestor via `closest`.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        if this.has_attribute(AUTO_ATTR) {
            fill_default(this);
            return;
        }
        ensure_button(this);
        install_click(this, &self.click);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.click.borrow_mut().take();
    }
}

/// The default-service URL for this page: `origin + /ucan/`.
fn default_remote_url() -> Option<String> {
    let origin = page_origin()?;
    Some(format!("{origin}{ACCESS_SERVICE_PATH}"))
}

/// The real page origin. Inside a sealed `<tonk-portal>` guest the
/// document is `about:srcdoc`, so `location.origin` is the opaque string
/// `"null"`; the portal bridge injects the true origin at
/// `window.tonk.context.origin`. Prefer that, and fall back to
/// `location.origin` at the top-level shell (where `window.tonk` is
/// absent). Returns `None` if neither yields a real origin.
fn page_origin() -> Option<String> {
    let win = window()?;
    if let Some(origin) = tonk_context_origin(&win) {
        return Some(origin);
    }
    let origin = win.location().origin().ok()?;
    (origin != "null" && !origin.is_empty()).then_some(origin)
}

/// Read `window.tonk.context.origin` — the host-supplied real origin the
/// portal bridge sets on the guest. `None` when any hop is missing/empty.
fn tonk_context_origin(win: &web_sys::Window) -> Option<String> {
    let tonk = js_sys::Reflect::get(win, &JsValue::from_str("tonk")).ok()?;
    let context = js_sys::Reflect::get(&tonk, &JsValue::from_str("context")).ok()?;
    let origin = js_sys::Reflect::get(&context, &JsValue::from_str("origin")).ok()?;
    origin.as_string().filter(|s| !s.is_empty() && s != "null")
}

/// Find or create the button as the element's only child. The label is
/// the element's own text content (moved onto the button so the host
/// carries no stray text node), defaulting to [`DEFAULT_LABEL`].
/// Idempotent — a reconnect reuses the existing button.
fn ensure_button(this: &HtmlElement) -> Option<Element> {
    let document = window().and_then(|w| w.document())?;
    if let Ok(Some(existing)) = this.query_selector(&format!(":scope > .{BUTTON}")) {
        return Some(existing);
    }
    let label = this
        .text_content()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| DEFAULT_LABEL.to_string());
    this.set_text_content(None);

    let button = document.create_element("button").ok()?;
    let _ = button.set_attribute("class", BUTTON);
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("part", "button");
    button.set_text_content(Some(&label));
    let _ = this.append_child(&button);
    Some(button)
}

/// Resolve and fill the default remote URL, if this page can determine it.
fn fill_default(this: &HtmlElement) {
    if let Some(url) = default_remote_url() {
        fill_target(this, &url);
    }
}

/// Install the click listener: resolve `origin + /ucan/` and write it
/// into the form control named by the element's `field` attribute.
fn install_click(this: &HtmlElement, slot: &ClickClosure) {
    let host = this.clone();
    let listener = Closure::wrap(Box::new(move |_event: Event| {
        fill_default(&host);
    }) as Box<dyn FnMut(Event)>);

    let _ = this.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    *slot.borrow_mut() = Some(listener);
}

/// Set the `value` of the form control named by `this`'s `field`
/// attribute (default `remote`) within the closest `<form>`. The target
/// is a `<wa-input>` (a form-associated custom element), so we set its
/// `value` *property* via reflection rather than the `value` attribute
/// — that's the slot the event layer reads on submit
/// (`elements.<field>.value`).
fn fill_target(this: &HtmlElement, url: &str) {
    let field = this
        .get_attribute("field")
        .unwrap_or_else(|| DEFAULT_FIELD.to_string());
    let Ok(Some(form)) = this.closest("form") else {
        return;
    };
    let Ok(Some(input)) = form.query_selector(&format!("[name=\"{field}\"]")) else {
        return;
    };
    let input_js: &JsValue = input.as_ref();
    let _ = js_sys::Reflect::set(
        input_js,
        &JsValue::from_str("value"),
        &JsValue::from_str(url),
    );
}

/// Register `<tonk-default-remote>`. Idempotent.
pub(crate) fn register() {
    let Some(elements) = window().map(|w| w.custom_elements()) else {
        return;
    };
    if elements.get("tonk-default-remote").is_undefined() {
        TonkDefaultRemote::define("tonk-default-remote");
    }
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Clicking the button writes `origin + /ucan/` into the form input
    /// named by `field`. Uses a native `<input>` as the target so the
    /// test can read `.value` back without a `<wa-input>` upgrade.
    #[dialog_common::test]
    async fn it_fills_the_named_input_with_the_default_service() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        // <form><tonk-default-remote field="remote"/><input name="remote"></form>
        let form = document.create_element("form").unwrap();
        let element = document.create_element("tonk-default-remote").unwrap();
        element.set_attribute("field", "remote").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("name", "remote").unwrap();
        form.append_child(&element).unwrap();
        form.append_child(&input).unwrap();
        // Appending a defined element runs connectedCallback synchronously,
        // so the button is present by the time `append_child` returns.
        body.append_child(&form).unwrap();

        let button = element
            .query_selector(".workspace__default-remote")
            .unwrap()
            .expect("button injected on connect");
        button.dyn_ref::<HtmlElement>().unwrap().click();

        let value = js_sys::Reflect::get(input.as_ref(), &JsValue::from_str("value"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let origin = window().unwrap().location().origin().unwrap();
        assert_eq!(
            value,
            format!("{origin}/ucan/"),
            "click should fill the named input with origin + /ucan/"
        );

        form.remove();
    }

    /// The element's text content becomes the button label; an empty
    /// element falls back to the default.
    #[dialog_common::test]
    async fn it_uses_its_text_as_the_button_label() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let labeled = document.create_element("tonk-default-remote").unwrap();
        labeled.set_text_content(Some("Use this server"));
        body.append_child(&labeled).unwrap();
        let button = labeled
            .query_selector(".workspace__default-remote")
            .unwrap()
            .expect("button injected");
        assert_eq!(button.text_content().as_deref(), Some("Use this server"));

        let bare = document.create_element("tonk-default-remote").unwrap();
        body.append_child(&bare).unwrap();
        let bare_button = bare
            .query_selector(".workspace__default-remote")
            .unwrap()
            .expect("button injected");
        assert_eq!(bare_button.text_content().as_deref(), Some(DEFAULT_LABEL));

        labeled.remove();
        bare.remove();
    }

    /// With `auto`, the element fills the field as soon as it connects and
    /// does not render the manual button.
    #[dialog_common::test]
    async fn it_auto_fills_without_rendering_a_button() {
        register();
        let document = window().unwrap().document().unwrap();
        let body = document.body().unwrap();

        let form = document.create_element("form").unwrap();
        let element = document.create_element("tonk-default-remote").unwrap();
        element.set_attribute("field", "remote").unwrap();
        element.set_attribute("auto", "").unwrap();
        let input = document.create_element("input").unwrap();
        input.set_attribute("name", "remote").unwrap();
        form.append_child(&input).unwrap();
        form.append_child(&element).unwrap();
        body.append_child(&form).unwrap();

        let value = js_sys::Reflect::get(input.as_ref(), &JsValue::from_str("value"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let origin = window().unwrap().location().origin().unwrap();
        assert_eq!(value, format!("{origin}/ucan/"));
        assert!(
            element
                .query_selector(".workspace__default-remote")
                .unwrap()
                .is_none(),
            "auto mode should not render a manual button"
        );

        form.remove();
    }
}
