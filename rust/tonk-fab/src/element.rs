//! The `<tonk-fab>` custom element.
//!
//! Wraps the FAB view inside the sealed profile-branch iframe. On connect it
//! measures its own bounding rect and posts a resize intent to the parent
//! window so `<tonk-fab-portal>` can size the iframe to fit the content:
//!
//! ```json
//! { "__tonkFab": { "type": "resize", "w": <f64>, "h": <f64> } }
//! ```
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper. It
//! exposes its own box via `getBoundingClientRect()`, which returns the
//! content rectangle once the element is connected and laid out.

use custom_elements::CustomElement;
use js_sys::{Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::{HtmlElement, window};

/// The `<tonk-fab>` custom element struct. Stateless: all behaviour is in
/// `connected_callback`.
#[derive(Default)]
pub struct TonkFab;

impl CustomElement for TonkFab {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        post_resize(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Measure `element`'s bounding rect and post a `__tonkFab` resize message to
/// `window.parent`. If the element has no size yet (width/height both zero),
/// we still post so the host has a defined initial state.
fn post_resize(element: &HtmlElement) {
    let Some(win) = window() else {
        return;
    };

    // `getBoundingClientRect()` is available immediately after connect,
    // though the layout may still be pending. The portal host applies the
    // dimensions on receipt; a subsequent re-measure (Task 7's
    // ResizeObserver) will correct any initial-layout inaccuracy.
    let elem: &web_sys::Element = element.unchecked_ref();
    let rect = elem.get_bounding_client_rect();
    let w = rect.width();
    let h = rect.height();

    let msg = Object::new();
    let fab = Object::new();
    Reflect::set(&fab, &"type".into(), &JsValue::from_str("resize")).ok();
    Reflect::set(&fab, &"w".into(), &JsValue::from_f64(w)).ok();
    Reflect::set(&fab, &"h".into(), &JsValue::from_f64(h)).ok();
    Reflect::set(&msg, &"__tonkFab".into(), &fab).ok();

    // `window.parent` is the outer document's window (the FAB portal host).
    // In a sandboxed opaque-origin iframe `parent` is the same as `window`
    // when there is no parent; the host silently ignores unknown messages.
    if let Ok(Some(parent)) = win.parent() {
        parent.post_message(&msg, "*").ok();
    }
}

/// Register `<tonk-fab>` with the page's custom element registry. Idempotent.
pub fn register() {
    let Some(win) = window() else {
        return;
    };
    // Guard against double-registration (the element crate may be imported
    // by multiple consumers in a single document).
    if !win.custom_elements().get("tonk-fab").is_undefined() {
        return;
    }
    TonkFab::define("tonk-fab");
}
