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
//! its iframe to the full viewport and keeps it there for the whole drag, so
//! the pointer coordinate frame never moves under itself. `pointermove`
//! translates the FAB *inside* the iframe (local `position: fixed`, no
//! per-frame postMessage) to follow the pointer, preserving the grab offset
//! captured on `pointerdown`. `pointerup` clamps the position, posts `drop`
//! (the host shrinks the iframe to a box at the drop point), and persists the
//! x/y via `window.tonk.transact(...)`.
//!
//! On connect, the element queries the persisted position from
//! `window.tonk.query(...)` and, if found, relays it to the host as a `drop`
//! message so the portal iframe is placed at the saved location.
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
            // Listen for host→guest `__tonkFab` sync messages. The sync
            // state lives on the SPACE branch overlay (`state:here`/`tonk:sync`),
            // not on profile/meta — the host `<tonk-fab-portal>` relays it via
            // `{ __tonkFab: { type: "sync", state } }`. V1 sends an honest
            // `offline` default; real state observation is a follow-up task.
            attach_host_messages(this);
        }
        // Query persisted position and relay it to the host portal.
        restore_position(this);
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
/// All closures are `forget()`-ed exactly once. `connected_callback` guards
/// against double-registration via the `data-fab-hover-bound` flag, so this
/// function is called at most once per element instance. The collapse
/// `Closure` is created ONCE here and its JS `Function` handle is captured
/// by the `on_leave` closure — reused across every `mouseleave` — so no
/// new closure is allocated per interaction.
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
        // The `expanded` class drives the CSS on the inner `.fab` div
        // (`.fab.expanded`, `.fab:not(.expanded) .fab__menu`), NOT the
        // `<tonk-fab>` host — so toggle it there.
        if let Some(fab) = element_for_enter.query_selector(".fab").ok().flatten() {
            fab.class_list().add_1("expanded").ok();
        }
        apply_menu_direction(&element_for_enter);
        post_resize(&element_for_enter);
    });

    // Build the collapse closure ONCE. Its JS Function handle is captured by
    // `on_leave` and passed to `setTimeout` on each mouseleave, so no new
    // Closure is allocated per interaction. `forget()` here is safe: the
    // element is a singleton that lives for the page lifetime, and the
    // collapse logic is idempotent (removing an absent class is a no-op).
    let element_for_collapse = element.clone();
    let collapse_once = Closure::<dyn Fn()>::new(move || {
        element_for_collapse.dataset().delete("collapseTimer");
        if let Some(fab) = element_for_collapse.query_selector(".fab").ok().flatten() {
            fab.class_list().remove_1("expanded").ok();
        }
        post_resize(&element_for_collapse);
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
        // A press on an interactive descendant (the name editable, a menu
        // link, a form control) is a click — not a drag. Bail before
        // `prevent_default`/capture so the control receives the press; the
        // FAB stays draggable everywhere else (the circle, the empty pill).
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
        // under the cursor (no snap-to-corner). The client coords and the rect
        // are read in the same frame, so their difference is a frame-
        // independent local offset that survives the iframe's expansion.
        let rect = el_down.get_bounding_client_rect();
        let grab_x = e.client_x() as f64 - rect.left();
        let grab_y = e.client_y() as f64 - rect.top();
        el_down.dataset().set("fabGrabX", &grab_x.to_string()).ok();
        el_down.dataset().set("fabGrabY", &grab_y.to_string()).ok();

        // Mark element as dragging.
        el_down.dataset().set("fabDragging", "1").ok();
        // Capture pointer so moves/up fire even outside element bounds.
        el_down.set_pointer_capture(e.pointer_id()).ok();
        // Pin the FAB so it can be placed anywhere inside the full-viewport
        // iframe. Anchor at the FAB's known page position so it stays under
        // the cursor the instant the host expands the iframe — without it,
        // `fixed` coords valid in the small iframe land at the corner once the
        // iframe grows, and the FAB only catches up on the first move. Fall
        // back to the in-frame rect if the position isn't cached yet.
        let (anchor_x, anchor_y) =
            read_cached_position(&el_down).unwrap_or((rect.left(), rect.top()));
        set_drag_style(&el_down, anchor_x, anchor_y);
        // Tell the host to expand the iframe to full viewport.
        post_fab_msg("dragstart", None, None);
    });

    let el_move = element.clone();
    let on_move = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_move.dataset().get("fabDragging").is_none() {
            return;
        }
        // Translate the FAB *inside* the full-viewport iframe to follow the
        // pointer. Local to the guest — no per-frame postMessage, so it's
        // smooth and the coordinate frame never moves under itself.
        let (grab_x, grab_y) = read_grab_offset(&el_move);
        let left = e.client_x() as f64 - grab_x;
        let top = e.client_y() as f64 - grab_y;
        set_drag_style(&el_move, left, top);
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabDragging").is_none() {
            return;
        }
        el_up.dataset().delete("fabDragging");
        el_up.release_pointer_capture(e.pointer_id()).ok();

        // FAB top-left in viewport coords (pointer minus the grab offset).
        let (grab_x, grab_y) = read_grab_offset(&el_up);
        el_up.dataset().delete("fabGrabX");
        el_up.dataset().delete("fabGrabY");
        let raw_x = e.client_x() as f64 - grab_x;
        let raw_y = e.client_y() as f64 - grab_y;

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

        // Return the FAB to normal in-flow layout; the host shrinks the iframe
        // to the dropped box so the FAB lands back at (x, y).
        clear_drag_style(&el_up);

        // Remember where we landed so the next drag anchors correctly.
        cache_position(&el_up, x, y);

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

/// Cache the FAB's current page top-left on the element. Written whenever the
/// guest tells the host where the FAB is (restore / default / drop) so that
/// `pointerdown` can anchor the pinned FAB at its real on-screen position —
/// see `set_drag_style`.
fn cache_position(el: &HtmlElement, x: f64, y: f64) {
    el.dataset().set("fabX", &x.to_string()).ok();
    el.dataset().set("fabY", &y.to_string()).ok();
}

/// Read the cached FAB page top-left, if known.
fn read_cached_position(el: &HtmlElement) -> Option<(f64, f64)> {
    let x = el.dataset().get("fabX").and_then(|s| s.parse::<f64>().ok())?;
    let y = el.dataset().get("fabY").and_then(|s| s.parse::<f64>().ok())?;
    Some((x, y))
}

/// Read the grab offset stashed on the element at `pointerdown`.
///
/// Returns `(0.0, 0.0)` if absent (e.g. a stray `pointermove`), which makes
/// the FAB track from its top-left — harmless, since a real drag always sets
/// these first.
fn read_grab_offset(el: &HtmlElement) -> (f64, f64) {
    let parse = |k: &str| {
        el.dataset()
            .get(k)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    (parse("fabGrabX"), parse("fabGrabY"))
}

/// Pin the FAB at `(left, top)` (viewport coords) while dragging. `fixed`
/// positioning is relative to the iframe viewport, whose origin tracks the
/// iframe's on-screen position, so this maps 1:1 to screen coords once the
/// host has expanded the iframe to the full viewport.
fn set_drag_style(el: &HtmlElement, left: f64, top: f64) {
    let style = el.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("margin", "0");
    let _ = style.set_property("left", &format!("{}px", left));
    let _ = style.set_property("top", &format!("{}px", top));
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().add_1("dragging").ok();
    }
}

/// Undo `set_drag_style`, returning the FAB to normal in-flow layout.
fn clear_drag_style(el: &HtmlElement) {
    let style = el.style();
    let _ = style.remove_property("position");
    let _ = style.remove_property("margin");
    let _ = style.remove_property("left");
    let _ = style.remove_property("top");
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().remove_1("dragging").ok();
    }
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
/// position if no persisted value exists. Caches the resolved position on
/// `this` so a subsequent drag anchors the FAB at its real on-screen spot.
fn restore_position(this: &HtmlElement) {
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
            post_default_position(this);
            return;
        }
    };

    let query_fn = match Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    {
        Some(f) => f,
        None => {
            post_default_position(this);
            return;
        }
    };

    let js_body = match js_sys::JSON::parse(&json_str).ok() {
        Some(v) => v,
        None => {
            post_default_position(this);
            return;
        }
    };

    let result = match query_fn.call1(&tonk, &js_body).ok() {
        Some(v) => v,
        None => {
            post_default_position(this);
            return;
        }
    };

    // `window.tonk.query` returns a Promise<Conclusion[]>.
    // Await it and relay the position to the host if present.
    if let Ok(promise) = result.dyn_into::<Promise>() {
        let this = this.clone();
        spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(rows) => {
                    if let Some((x, y)) = read_position_from_rows(&rows) {
                        cache_position(&this, x, y);
                        post_fab_msg("drop", Some(x), Some(y));
                    } else {
                        post_default_position(&this);
                    }
                }
                Err(_) => post_default_position(&this),
            }
        });
    } else {
        post_default_position(this);
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
fn post_default_position(this: &HtmlElement) {
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
    cache_position(this, x, y);
    post_fab_msg("drop", Some(x), Some(y));
}

/// Apply `opens-down` or `opens-up` to the `.fab__menu` inside `element`,
/// based on whether the FAB is in the top or bottom half of the viewport.
fn apply_menu_direction(element: &HtmlElement) {
    let Some(win) = window() else {
        return;
    };
    let vh = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(768.0);

    // Read current_y from the element's bounding rect.
    let elem: &web_sys::Element = element.unchecked_ref();
    let rect = elem.get_bounding_client_rect();
    let current_y = rect.top();

    let opens_down = submenu_opens_down(current_y, vh);

    // Find the .fab__menu child and toggle the direction class.
    let menu = elem.query_selector(".fab__menu").ok().flatten();
    if let Some(menu_el) = menu {
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

/// Listen for `{ __tonkFab: { type: "sync", state } }` messages from the
/// parent host and apply `data-sync` to `.fab__circle`.
///
/// The sync state lives on the SPACE branch overlay (`state:here` /
/// `Replica::SYNC_STATE_HERE`) — content-replica state, NOT profile/meta.
/// The FAB guest is sealed to the profile branch and cannot query the content
/// branch. The host `<tonk-fab-portal>` observes the active space's sync state
/// and relays it through this `__tonkFab` channel. V1 sends an honest `offline`
/// default; wiring the real value is a follow-up task for the host.
///
/// Valid `state` values: `"synced"` (filled circle), `"offline"` (outlined),
/// `"syncing"` (blinking). Any unrecognised value falls back to `"offline"`.
fn attach_host_messages(element: &HtmlElement) {
    let element_clone = element.clone();
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let data = event.data();
            let fab_payload =
                Reflect::get(&data, &"__tonkFab".into()).unwrap_or(JsValue::UNDEFINED);
            if fab_payload.is_undefined() || fab_payload.is_null() {
                return;
            }
            let msg_type = Reflect::get(&fab_payload, &"type".into())
                .ok()
                .and_then(|v| v.as_string());
            if msg_type.as_deref() != Some("sync") {
                return;
            }
            let state = Reflect::get(&fab_payload, &"state".into())
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            // Map to a valid data-sync value; any unrecognised state → offline.
            let sync_val = match state.as_str() {
                "synced" => "synced",
                "syncing" => "syncing",
                _ => "offline",
            };
            let elem: &web_sys::Element = element_clone.unchecked_ref();
            if let Some(circle) = elem.query_selector(".fab__circle").ok().flatten() {
                circle.set_attribute("data-sync", sync_val).ok();
            }
        });
    if let Some(win) = window() {
        win.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref())
            .ok();
    }
    // Safe to forget: the FAB element is a singleton that lives for the page
    // lifetime, so the closure never becomes dangling.
    on_message.forget();
}

/// Measure `element`'s bounding rect and post a `__tonkFab` resize message to
/// `window.parent`.
fn post_resize(element: &HtmlElement) {
    let Some(win) = window() else {
        return;
    };

    // Measure the inner `.fab`'s CONTENT extent, not the host's bounding box.
    // `<tonk-fab>`/`.fab` are `display:block`, so their border-box width is
    // clamped to the (small) iframe width — measuring it would size the iframe
    // from a value that depends on the iframe's own width (a feedback deadlock
    // that can never grow), and the expanded bar's `white-space:nowrap`
    // content would just overflow and get clipped. `scrollWidth`/`scrollHeight`
    // report the true content extent: the overflowing nowrap content
    // horizontally, and the absolutely-positioned `.fab__menu` hanging below
    // vertically. Fall back to the host bounding box if `.fab` is absent.
    let host: &web_sys::Element = element.unchecked_ref();
    let (w, h) = match host.query_selector(".fab").ok().flatten() {
        Some(fab) => (f64::from(fab.scroll_width()), f64::from(fab.scroll_height())),
        None => {
            let rect = host.get_bounding_client_rect();
            (rect.width(), rect.height())
        }
    };

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
