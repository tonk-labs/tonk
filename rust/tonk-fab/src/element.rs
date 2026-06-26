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
//! Hover expand/collapse: `mouseenter` adds the `expanded` class (and
//! re-posts resize for the expanded bar); `mouseleave` schedules a collapse
//! after `COLLAPSE_MS` (removes the class, re-posts resize for the circle).
//! A re-enter before the timeout cancels the pending collapse.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::COLLAPSE_MS;
use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, window};

// web-sys doesn't expose a typed `clearTimeout`/`setTimeout` wrapper in the
// features we have, so we call them via js_sys::Function from the global.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

/// The `<tonk-fab>` custom element.
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
        // Guard against double-registration if the element reconnects.
        if this.dataset().get("fabHoverBound").is_none() {
            this.dataset().set("fabHoverBound", "1").ok();
            attach_hover(this);
        }
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        // Cancel any pending collapse timer so the closure doesn't fire against
        // a detached element and send a spurious resize postMessage.
        if let Some(id_str) = this.dataset().get("collapseTimer") {
            if let Ok(id) = id_str.parse::<i32>() {
                clear_timeout(id);
            }
            this.dataset().delete("collapseTimer");
        }
    }

    fn attribute_changed_callback(
        &mut self,
        _this: &HtmlElement,
        _name: String,
        _old: Option<String>,
        _new: Option<String>,
    ) {
    }
}

/// Attach `mouseenter` / `mouseleave` listeners to `element`.
///
/// - `mouseenter`: add `expanded` class, re-post resize, cancel any pending
///   collapse timer.
/// - `mouseleave`: schedule collapse after `COLLAPSE_MS`; on fire, remove
///   `expanded` class and re-post resize.
///
/// Both closures are `forget()`-ed. This is safe because the FAB element is
/// created once and lives for the page lifetime. `connected_callback` guards
/// against double-registration via the `data-fab-hover-bound` flag, so this
/// function is called at most once per element instance.
fn attach_hover(element: &HtmlElement) {
    let element_for_enter = element.clone();
    let on_enter = Closure::<dyn Fn()>::new(move || {
        // Cancel any pending collapse stored in the element dataset.
        if let Some(id_str) = element_for_enter.dataset().get("collapseTimer") {
            if let Ok(id) = id_str.parse::<i32>() {
                clear_timeout(id);
            }
            element_for_enter.dataset().delete("collapseTimer");
        }
        element_for_enter.class_list().add_1("expanded").ok();
        post_resize(&element_for_enter);
    });

    let element_for_leave = element.clone();
    let on_leave = Closure::<dyn Fn()>::new(move || {
        let element_for_timer = element_for_leave.clone();
        let collapse = Closure::<dyn Fn()>::new(move || {
            element_for_timer.dataset().delete("collapseTimer");
            element_for_timer.class_list().remove_1("expanded").ok();
            post_resize(&element_for_timer);
        });
        let id = set_timeout(collapse.as_ref().unchecked_ref(), COLLAPSE_MS as i32);
        collapse.forget();
        element_for_leave.dataset().set("collapseTimer", &id.to_string()).ok();
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("mouseenter", on_enter.as_ref().unchecked_ref())
        .ok();
    target
        .add_event_listener_with_callback("mouseleave", on_leave.as_ref().unchecked_ref())
        .ok();

    on_enter.forget();
    on_leave.forget();
}

/// Measure `element`'s bounding rect and post a `__tonkFab` resize message to
/// `window.parent`.
fn post_resize(element: &HtmlElement) {
    let Some(win) = window() else {
        return;
    };

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

    if let Ok(Some(parent)) = win.parent() {
        parent.post_message(&msg, "*").ok();
    }
}

/// Register `<tonk-fab>` with the page's custom element registry. Idempotent.
pub fn register() {
    let Some(win) = window() else {
        return;
    };
    if !win.custom_elements().get("tonk-fab").is_undefined() {
        return;
    }
    TonkFab::define("tonk-fab");
}
