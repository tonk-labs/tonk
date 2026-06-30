//! The `<tonk-fab>` custom element — a floating, draggable container.
//!
//! Generic affordance: it renders its content as a `position: fixed` box on a
//! high z-index (so it floats over whatever is below) and lets the user drag it
//! around the viewport. It is NOT a portal and uses no iframe — it lives in the
//! same document as its content and moves itself directly. The FAB chrome uses
//! it to float the profile pill over the space content, but nothing here is
//! FAB-specific beyond the `.fab` class names the view supplies.
//!
//! - Hover expand/collapse: `mouseenter` adds the `expanded` class to the inner
//!   `.fab`; `mouseleave` schedules a collapse after `COLLAPSE_MS`. A re-enter
//!   before the timeout cancels the pending collapse.
//! - Drag: `pointerdown` (not on an interactive descendant) starts a free drag,
//!   capturing the grab offset; `pointermove` sets the element's own
//!   `left`/`top`; `pointerup` clamps to keep it on-screen and persists the
//!   x/y as a profile claim via `window.tonk.transact(...)`.
//! - On connect it restores the persisted position (or a default top-centre) and
//!   applies it to its own style.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{COLLAPSE_MS, clamp_position, position_claim_json, submenu_opens_down};
use custom_elements::CustomElement;
use js_sys::Promise;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, PointerEvent, window};

// web-sys doesn't expose a typed `clearTimeout`/`setTimeout` wrapper in the
// features we have, so we call them via js_sys::Function from the global.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

/// Circle size in CSS pixels — the FAB's resting footprint. We clamp to this so
/// the full circle (with its padding) stays on screen.
const CIRCLE_SIZE: f64 = 64.0;

/// The z-index the floating FAB sits at — above page content (and the repo
/// content portal) so it never gets covered. Near `MAX_SAFE_INTEGER` to beat
/// any app stacking context.
const FAB_Z_INDEX: &str = "2147483646";

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
        // Float the element: fixed-position, high z-index. Its left/top come
        // from the restored position below.
        let style = this.style();
        let _ = style.set_property("position", "fixed");
        let _ = style.set_property("margin", "0");
        let _ = style.set_property("z-index", FAB_Z_INDEX);

        // Guard against double-binding when the SAME element reconnects.
        //
        // The marker is a JS expando PROPERTY, not a `data-*` attribute, on
        // purpose: `<tonk-display>` snapshots its view by `cloneNode`-ing the
        // authored subtree and mounting the clone. `cloneNode` copies
        // attributes but NOT event listeners or JS properties — so an
        // attribute guard would ride along on the clone (marking it "bound")
        // while the listeners stayed on the discarded original, leaving the
        // live element inert. A property is dropped by `cloneNode`, so the
        // mounted clone re-binds; it still persists across a genuine
        // disconnect/reconnect of the same node, so reconnects don't double-bind.
        let already_bound = Reflect::get(this.as_ref(), &"__tonkFabBound".into())
            .map(|v| v.is_truthy())
            .unwrap_or(false);
        if !already_bound {
            let _ = Reflect::set(this.as_ref(), &"__tonkFabBound".into(), &JsValue::TRUE);
            attach_hover(this);
            attach_drag(this);
        }
        // Restore the persisted position and apply it to our own style.
        restore_position(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        // Cancel any pending collapse timer so the closure doesn't fire against
        // a detached element.
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
/// - `mouseenter`: add `expanded` class to the inner `.fab`, cancel any pending
///   collapse timer, and reorient the submenu.
/// - `mouseleave`: schedule collapse after `COLLAPSE_MS`; on fire, remove the
///   `expanded` class.
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
        // The `expanded` class drives the CSS on the inner `.fab` div, NOT the
        // `<tonk-fab>` host — so toggle it there.
        if let Some(fab) = element_for_enter.query_selector(".fab").ok().flatten() {
            fab.class_list().add_1("expanded").ok();
        }
        apply_menu_direction(&element_for_enter);
    });

    // Build the collapse closure ONCE. Its JS Function handle is captured by
    // `on_leave` and passed to `setTimeout` on each mouseleave, so no new
    // Closure is allocated per interaction.
    let element_for_collapse = element.clone();
    let collapse_once = Closure::<dyn Fn()>::new(move || {
        element_for_collapse.dataset().delete("collapseTimer");
        if let Some(fab) = element_for_collapse.query_selector(".fab").ok().flatten() {
            fab.class_list().remove_1("expanded").ok();
        }
    });
    let collapse_fn = collapse_once
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    collapse_once.forget();

    let element_for_leave = element.clone();
    let on_leave = Closure::<dyn Fn()>::new(move || {
        let id = set_timeout(&collapse_fn, COLLAPSE_MS as i32);
        element_for_leave
            .dataset()
            .set("collapseTimer", &id.to_string())
            .ok();
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

/// Attach pointer event listeners for free drag-and-drop. The element moves
/// itself (its own `position: fixed` `left`/`top`); there is no iframe to relay
/// to.
///
/// - `pointerdown` (not on an interactive descendant): capture the grab offset,
///   set pointer capture.
/// - `pointermove`: set the element's `left`/`top` to follow the pointer.
/// - `pointerup`: clamp to keep the circle on-screen and persist x/y.
fn attach_drag(element: &HtmlElement) {
    let el_down = element.clone();
    let on_down = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        // A press on an interactive descendant (the name editable, a menu
        // link, a form control) is a click — not a drag. Bail before
        // `prevent_default`/capture so the control receives the press.
        if let Some(target) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) {
            if target
                .closest("a, button, input, textarea, select, tonk-editable, [contenteditable]")
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
        }
        e.prevent_default();

        // Record the pointer's offset within the FAB so the grab point stays
        // under the cursor (no snap-to-corner).
        let rect = el_down.get_bounding_client_rect();
        let grab_x = e.client_x() as f64 - rect.left();
        let grab_y = e.client_y() as f64 - rect.top();
        el_down.dataset().set("fabGrabX", &grab_x.to_string()).ok();
        el_down.dataset().set("fabGrabY", &grab_y.to_string()).ok();

        el_down.dataset().set("fabDragging", "1").ok();
        el_down.set_pointer_capture(e.pointer_id()).ok();
        // Mark the inner `.fab` dragging (CSS may suppress hover/transitions).
        if let Some(fab) = el_down.query_selector(".fab").ok().flatten() {
            fab.class_list().add_1("dragging").ok();
        }
    });

    let el_move = element.clone();
    let on_move = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_move.dataset().get("fabDragging").is_none() {
            return;
        }
        let (grab_x, grab_y) = read_grab_offset(&el_move);
        let left = e.client_x() as f64 - grab_x;
        let top = e.client_y() as f64 - grab_y;
        track_position(&el_move, left, top);
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabDragging").is_none() {
            return;
        }
        el_up.dataset().delete("fabDragging");
        el_up.release_pointer_capture(e.pointer_id()).ok();
        if let Some(fab) = el_up.query_selector(".fab").ok().flatten() {
            fab.class_list().remove_1("dragging").ok();
        }

        // FAB top-left in viewport coords (pointer minus the grab offset).
        let (grab_x, grab_y) = read_grab_offset(&el_up);
        el_up.dataset().delete("fabGrabX");
        el_up.dataset().delete("fabGrabY");
        let raw_x = e.client_x() as f64 - grab_x;
        let raw_y = e.client_y() as f64 - grab_y;

        // Clamp to keep the circle on-screen, then settle there.
        let (x, y) = clamp_to_viewport(raw_x, raw_y);
        settle_position(&el_up, x, y);

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

/// Clamp `(raw_x, raw_y)` so the FAB circle stays within the viewport.
fn clamp_to_viewport(raw_x: f64, raw_y: f64) -> (f64, f64) {
    let Some(win) = window() else {
        return (raw_x, raw_y);
    };
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
}

/// Read the grab offset stashed on the element at `pointerdown`.
fn read_grab_offset(el: &HtmlElement) -> (f64, f64) {
    let parse = |k: &str| {
        el.dataset()
            .get(k)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (parse("fabGrabX"), parse("fabGrabY"))
}

/// Track the FAB at `(left, top)` (viewport top-left) with plain `left`/`top`
/// during a drag — no corner anchoring, so it follows the cursor 1:1 without
/// jumping as it crosses the viewport midlines.
fn track_position(el: &HtmlElement, left: f64, top: f64) {
    let style = el.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{}px", left));
    let _ = style.set_property("top", &format!("{}px", top));
}

/// Settle the FAB at `(left, top)` anchored to the NEAREST corner: pin to
/// `right` when its center is in the right half of the viewport (else `left`),
/// and to `bottom` when in the bottom half (else `top`). Anchoring to the near
/// edge keeps the FAB in the same corner when the viewport resizes, instead of
/// drifting from a fixed top-left offset. Used at drop and on restore.
fn settle_position(el: &HtmlElement, left: f64, top: f64) {
    let style = el.style();
    let (vw, vh) = viewport_size();
    let rect = el.get_bounding_client_rect();
    let (w, h) = (
        rect.width().max(CIRCLE_SIZE),
        rect.height().max(CIRCLE_SIZE),
    );

    let _ = style.remove_property("left");
    let _ = style.remove_property("right");
    if left + w / 2.0 > vw / 2.0 {
        let right = (vw - (left + w)).max(0.0);
        let _ = style.set_property("right", &format!("{}px", right));
    } else {
        let _ = style.set_property("left", &format!("{}px", left.max(0.0)));
    }

    let _ = style.remove_property("top");
    let _ = style.remove_property("bottom");
    if top + h / 2.0 > vh / 2.0 {
        let bottom = (vh - (top + h)).max(0.0);
        let _ = style.set_property("bottom", &format!("{}px", bottom));
    } else {
        let _ = style.set_property("top", &format!("{}px", top.max(0.0)));
    }
}

/// The viewport size, defaulting if unavailable.
fn viewport_size() -> (f64, f64) {
    let Some(win) = window() else {
        return (1024.0, 768.0);
    };
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
    (vw, vh)
}

/// Persist `(x, y)` by calling `window.tonk.transact(request)`. The request is
/// the `TransactRequest` JSON produced by `position_claim_json`, parsed back to
/// a JS object via `JSON.parse` (the bridge accepts any structured-clonable
/// object).
fn persist_position(x: u32, y: u32) {
    let claim = position_claim_json(x, y);
    let json_str = match serde_json::to_string(&claim) {
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
        None => return,
    };
    let transact_fn = match Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => return,
    };
    let js_obj = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => return,
    };
    transact_fn.call1(&tonk, &js_obj).ok();
}

/// On connect, query the persisted FAB position from `window.tonk.query(...)`
/// and apply it to the element's own style. Falls back to a default top-centre
/// position if no persisted value exists.
fn restore_position(this: &HtmlElement) {
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
            default_position(this);
            return;
        }
    };

    let query_fn = match Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => {
            default_position(this);
            return;
        }
    };

    let js_body = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => {
            default_position(this);
            return;
        }
    };

    let result = match query_fn.call1(&tonk, &js_body).ok() {
        Some(v) => v,
        None => {
            default_position(this);
            return;
        }
    };

    // `window.tonk.query` returns a Promise<Conclusion[]>. Await it and apply
    // the position if present.
    if let Ok(promise) = result.dyn_into::<Promise>() {
        let this = this.clone();
        spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(rows) => match read_position_from_rows(&rows) {
                    Some((x, y)) => settle_position(&this, x, y),
                    None => default_position(&this),
                },
                Err(_) => default_position(&this),
            }
        });
    } else {
        default_position(this);
    }
}

/// Extract the first `x`/`y` from a `Conclusion[]` rows value returned by
/// `window.tonk.query(...)`.
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

/// Apply a default top-centre position to the element.
fn default_position(this: &HtmlElement) {
    let (x, y) = if let Some(win) = window() {
        let vw = win
            .inner_width()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(1024.0);
        ((vw / 2.0 - CIRCLE_SIZE / 2.0).max(0.0), 16.0)
    } else {
        (480.0, 16.0)
    };
    settle_position(this, x, y);
}

/// Apply `opens-down` or `opens-up` to the `.fab__menu` inside `element`, based
/// on whether the FAB is in the top or bottom half of the viewport.
fn apply_menu_direction(element: &HtmlElement) {
    let Some(win) = window() else {
        return;
    };
    let vh = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(768.0);

    let elem: &web_sys::Element = element.unchecked_ref();
    let rect = elem.get_bounding_client_rect();
    let current_y = rect.top();
    let opens_down = submenu_opens_down(current_y, vh);

    if let Some(menu_el) = elem.query_selector(".fab__menu").ok().flatten() {
        let cl = menu_el.class_list();
        if opens_down {
            cl.remove_1("opens-up").ok();
            cl.add_1("opens-down").ok();
        } else {
            cl.remove_1("opens-down").ok();
            cl.add_1("opens-up").ok();
        }
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
