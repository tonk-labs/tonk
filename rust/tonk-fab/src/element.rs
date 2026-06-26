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
//! Drag: `pointerdown` on the circle starts a free drag — the host expands
//! its iframe to the full viewport so the circle can be moved anywhere.
//! `pointermove` posts dragmove x/y; `pointerup` clamps the position, posts
//! `drop`, and persists the x/y via `window.tonk.transact(...)`.
//!
//! On connect, the element queries the persisted position from
//! `window.tonk.query(...)` and, if found, relays it to the host as a `drop`
//! message so the portal iframe is placed at the saved location.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{COLLAPSE_MS, clamp_position, position_claim_json};
use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use js_sys::Promise;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlElement, PointerEvent, window};

// web-sys doesn't expose a typed `clearTimeout`/`setTimeout` wrapper in the
// features we have, so we call them via js_sys::Function from the global.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

/// Circle size in CSS pixels — matches the `.fab__circle` 24px width/height
/// plus 6px padding on each side in the `.fab` container. We clamp to a 64px
/// footprint so the full circle (with its padding) stays on screen.
const CIRCLE_SIZE: f64 = 64.0;

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
            attach_drag(this);
        }
        // Query persisted position and relay it to the host portal.
        restore_position();
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

/// Attach pointer event listeners for free drag-and-drop.
///
/// - `pointerdown`: post `dragstart`, set pointer capture so all subsequent
///   pointer events are delivered even if the cursor leaves the element.
/// - `pointermove`: if dragging, post `dragmove{x,y}` in viewport coords.
/// - `pointerup`: clamp position, post `drop{x,y}`, persist via
///   `window.tonk.transact(...)`.
///
/// All closures are `forget()`-ed; `connected_callback` guards against
/// double-registration via `data-fab-hover-bound`.
fn attach_drag(element: &HtmlElement) {
    let el_down = element.clone();
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        e.prevent_default();
        // Mark element as dragging.
        el_down.dataset().set("fabDragging", "1").ok();
        // Capture pointer so moves/up fire even outside element bounds.
        el_down
            .set_pointer_capture(e.pointer_id())
            .ok();
        // Tell the host to expand the iframe to full viewport.
        post_fab_msg("dragstart", None, None);
    });

    let el_move = element.clone();
    let on_move = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_move.dataset().get("fabDragging").is_none() {
            return;
        }
        let x = e.client_x() as f64;
        let y = e.client_y() as f64;
        post_fab_msg("dragmove", Some(x), Some(y));
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabDragging").is_none() {
            return;
        }
        el_up.dataset().delete("fabDragging");
        el_up.release_pointer_capture(e.pointer_id()).ok();

        let raw_x = e.client_x() as f64;
        let raw_y = e.client_y() as f64;

        // Clamp to keep the circle on-screen.
        let (x, y) = if let Some(win) = window() {
            let vw = win
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(1024.0);
            let vh = win
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or(768.0);
            clamp_position(raw_x, raw_y, vw, vh, CIRCLE_SIZE, CIRCLE_SIZE)
        } else {
            (raw_x, raw_y)
        };

        // Tell the host to shrink the iframe to a box at (x, y).
        post_fab_msg("drop", Some(x), Some(y));

        // Persist the new position over the bridge.
        persist_position(x as u32, y as u32);
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())
        .ok();
    target
        .add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref())
        .ok();
    target
        .add_event_listener_with_callback("pointerup", on_up.as_ref().unchecked_ref())
        .ok();

    on_down.forget();
    on_move.forget();
    on_up.forget();
}

/// Post a `__tonkFab` geometry message to `window.parent`.
fn post_fab_msg(kind: &str, x: Option<f64>, y: Option<f64>) {
    let Some(win) = window() else {
        return;
    };
    let msg = Object::new();
    let fab = Object::new();
    Reflect::set(&fab, &"type".into(), &JsValue::from_str(kind)).ok();
    if let Some(x) = x {
        Reflect::set(&fab, &"x".into(), &JsValue::from_f64(x)).ok();
    }
    if let Some(y) = y {
        Reflect::set(&fab, &"y".into(), &JsValue::from_f64(y)).ok();
    }
    Reflect::set(&msg, &"__tonkFab".into(), &fab).ok();

    if let Ok(Some(parent)) = win.parent() {
        parent.post_message(&msg, "*").ok();
    }
}

/// Persist `(x, y)` by calling `window.tonk.transact(request)` over the
/// guest bridge. The `request` is the `TransactRequest` JSON produced by
/// `position_claim_json`, serialised to a JS value via `serde_wasm_bindgen`
/// is unavailable here, so we serialise to a JSON string and parse it back
/// with `JSON.parse` (the bridge accepts any structured-clonable object).
fn persist_position(x: u32, y: u32) {
    let claim = position_claim_json(x, y);
    let json_str = match serde_json::to_string(&claim) {
        Ok(s) => s,
        Err(_) => return,
    };
    // Obtain `window.tonk.transact` and call it with the parsed JS object.
    let Some(win) = window() else {
        return;
    };
    let tonk = match Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    {
        Some(t) => t,
        None => return,
    };
    let transact_fn = match Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => return,
    };
    // Parse the JSON string into a JS object via `JSON.parse`.
    let js_obj = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => return,
    };
    // Call transact — returns a Promise we can ignore (fire-and-forget).
    transact_fn.call1(&tonk, &js_obj).ok();
}

/// On connect, query the persisted FAB position from `window.tonk.query(...)`
/// and relay it to the host as a `drop` geometry message so the portal iframe
/// is placed at the saved location. Falls back to a default top-center
/// position if no persisted value exists.
fn restore_position() {
    // Build the query body: pin `this` to `state:fab`, project x and y.
    // Shape matches what `tonk-portal/src/query.rs` produces for entity queries.
    let query_body = serde_json::json!({
        "terms": {
            "this": "state:fab",
            "x": { "?": { "name": "x" } },
            "y": { "?": { "name": "y" } }
        },
        "predicate": {
            "description": "Persisted FAB position (profile-meta claim).",
            "with": {
                "x": { "the": "xyz.tonk.fab/x", "cardinality": "one", "as": "UnsignedInteger" },
                "y": { "the": "xyz.tonk.fab/y", "cardinality": "one", "as": "UnsignedInteger" }
            }
        }
    });

    let json_str = match serde_json::to_string(&query_body) {
        Ok(s) => s,
        Err(_) => return,
    };

    let Some(win) = window() else {
        return;
    };

    let tonk = match Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    {
        Some(t) => t,
        None => {
            // No bridge yet — use default top-center position.
            post_default_position();
            return;
        }
    };

    let query_fn = match Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => {
            post_default_position();
            return;
        }
    };

    let js_body = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => {
            post_default_position();
            return;
        }
    };

    let result = match query_fn.call1(&tonk, &js_body).ok() {
        Some(v) => v,
        None => {
            post_default_position();
            return;
        }
    };

    // `window.tonk.query` returns a Promise<Conclusion[]>.
    // Await it and relay the position to the host if present.
    if let Ok(promise) = result.dyn_into::<Promise>() {
        spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(rows) => {
                    if let Some((x, y)) = read_position_from_rows(&rows) {
                        post_fab_msg("drop", Some(x), Some(y));
                    } else {
                        post_default_position();
                    }
                }
                Err(_) => post_default_position(),
            }
        });
    } else {
        post_default_position();
    }
}

/// Extract the first `x`/`y` from a `Conclusion[]` rows value returned by
/// `window.tonk.query(...)`. The rows are a JS array of objects; each object
/// has a key per projected variable (here `"x"` and `"y"`).
fn read_position_from_rows(rows: &JsValue) -> Option<(f64, f64)> {
    let arr = rows.dyn_ref::<js_sys::Array>()?;
    let first = arr.get(0);
    if first.is_undefined() || first.is_null() {
        return None;
    }
    let x = Reflect::get(&first, &"x".into())
        .ok()
        .and_then(|v| v.as_f64())?;
    let y = Reflect::get(&first, &"y".into())
        .ok()
        .and_then(|v| v.as_f64())?;
    Some((x, y))
}

/// Post a default `drop` position (top-center of the viewport).
fn post_default_position() {
    let (x, y) = if let Some(win) = window() {
        let vw = win
            .inner_width()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(1024.0);
        // Top-center: horizontally centered, near top.
        ((vw / 2.0 - CIRCLE_SIZE / 2.0).max(0.0), 16.0)
    } else {
        (480.0, 16.0)
    };
    post_fab_msg("drop", Some(x), Some(y));
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
