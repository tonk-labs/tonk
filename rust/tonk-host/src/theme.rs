//! Light/dark for the whole app, not just the frame that asks.
//!
//! The theme is one property of the product, but the product is a tree of
//! documents: the page, the profile guest, and the space guest nested inside
//! it. Each has its own root element, so setting a class reaches exactly one
//! of them — which is why toggling the bar's pill themed the bar and left the
//! space behind it untouched.
//!
//! So a change is applied locally and then relayed DOWN the frame tree: each
//! guest's bootstrap (`tonk-portal::bridge`) applies it and passes it on, so
//! one call reaches every depth.
//!
//! ## What this deliberately does not do
//!
//! It does not travel UP. A guest cannot reach the page — it is an opaque
//! origin — and the page sits behind full-bleed guest content anyway, so the
//! visible result is already correct. The consequence is that the choice is
//! not remembered: `localStorage` belongs to the page, and the guest cannot
//! write it. Persisting needs a `mode` page effect alongside `navigate` and
//! `title`; see `plan/fabb-conformance.md`.
//!
//! ## Law 6
//!
//! The FABB spec says the chrome themes itself and never the view. That holds
//! for a space's own CONTENT styling, which nothing here rewrites — but the
//! app's light/dark ground is not the chrome's private business, and a
//! toggle that leaves the page around it in the other mode reads as broken
//! rather than as principled.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::window;

/// Apply `dark` to this document and every guest beneath it.
pub fn set_mode(dark: bool) {
    apply_here(dark);
    relay_down(dark);
}

/// Whether this document is currently painting dark.
pub fn is_dark() -> bool {
    window()
        .and_then(|win| win.document())
        .and_then(|document| document.document_element())
        .is_some_and(|root| root.class_list().contains("wa-dark"))
}

fn apply_here(dark: bool) {
    let Some(root) = window()
        .and_then(|win| win.document())
        .and_then(|document| document.document_element())
    else {
        return;
    };
    let classes = root.class_list();
    let _ = classes.toggle_with_force("wa-dark", dark);
    let _ = classes.toggle_with_force("wa-light", !dark);
}

/// Post the change into every child frame. Their bootstraps recurse, so this
/// only has to reach one level.
fn relay_down(dark: bool) {
    let Some(document) = window().and_then(|win| win.document()) else {
        return;
    };
    let Ok(frames) = document.query_selector_all("iframe") else {
        return;
    };
    let message = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&message, &"__tonkRuntime".into(), &"mode".into());
    let _ = js_sys::Reflect::set(
        &message,
        &"mode".into(),
        &JsValue::from_str(if dark { "dark" } else { "light" }),
    );
    for index in 0..frames.length() {
        let Some(frame) = frames
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::HtmlIFrameElement>().ok())
        else {
            continue;
        };
        if let Some(inner) = frame.content_window() {
            let _ = inner.post_message(&message, "*");
        }
    }
}
