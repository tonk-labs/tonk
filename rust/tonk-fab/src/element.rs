//! The `<tonk-fab>` custom element — a floating, draggable container.
//!
//! Generic affordance: it renders its content as a `position: fixed` box on a
//! high z-index (so it floats over whatever is below) and lets the user drag it
//! around the viewport. It is NOT a portal and uses no iframe — it lives in the
//! same document as its content and moves itself directly. The FAB chrome uses
//! it to float the profile pill over the space content, but nothing here is
//! FAB-specific beyond the `.fab` class names the view supplies.
//!
//! - Telescope collapse/expand: the bar rests COLLAPSED (just the sync circle).
//!   A plain click on the circle toggles it — the segments after the cap
//!   animate their `max-width` open/closed, staggered, so the bar unfolds from
//!   / retracts into the circle. A DOUBLE click toggles pause/resume of sync
//!   (the circle is the pause switch, matching the control-panel wireframe).
//! - Drag: `pointerdown` (not on an interactive descendant) starts a free drag,
//!   capturing the grab offset; `pointermove` sets the element's own
//!   `left`/`top`; `pointerup` clamps to keep it on-screen and persists the
//!   x/y as a profile claim via `window.tonk.transact(...)`. A press that never
//!   moves past a small threshold is treated as a click, not a drag.
//! - On connect it wraps each collapsible segment for the telescope, restores
//!   the persisted position (or a default top-centre), and applies it.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{
    clamp_position, position_claim_json, submenu_opens_down, telescope_delay_ms,
    telescope_settle_ms,
};
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

/// How far (CSS px) the pointer must travel from the press origin before it
/// counts as a drag rather than a click. Below this the press toggles the
/// telescope; above it the FAB moves and the click is suppressed.
const DRAG_THRESHOLD_PX: f64 = 4.0;

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
            wrap_telescope_tiles(this);
            attach_drag(this);
            attach_gestures(this);
        }
        // Restore the persisted position and apply it to our own style.
        restore_position(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        // Cancel any pending timers so their closures don't fire against a
        // detached element.
        for key in ["settleTimer", "tapTimer", "editTimer"] {
            if let Some(id_str) = this.dataset().get(key) {
                if let Ok(id) = id_str.parse::<i32>() {
                    clear_timeout(id);
                }
                this.dataset().delete(key);
            }
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

/// Wrap each collapsible segment (every `.fab` child after the sync-circle cap)
/// in a `.fab__tele` div whose `max-width` the telescope animates. Adds the
/// `fab--anim` marker (enables the transition CSS) and the initial
/// `fab--collapsed` state (the bar rests as just the circle). Mirrors the
/// wireframe's programmatic `wrapTele` — done in JS, not the authored markup,
/// so the view template stays a plain segment list.
fn wrap_telescope_tiles(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    // Children after the first (the `.fab__cap-l` circle) are collapsible.
    let children = fab.children();
    let mut tiles: Vec<Element> = Vec::new();
    for i in 1..children.length() {
        if let Some(child) = children.item(i) {
            // Skip anything already wrapped (defensive against a re-run).
            if child.class_list().contains("fab__tele") {
                continue;
            }
            tiles.push(child);
        }
    }
    for tile in &tiles {
        let Ok(wrapper) = document.create_element("div") else {
            continue;
        };
        let _ = wrapper.set_attribute("class", "fab__tele");
        // Start each wrapper in the collapsed geometry (clamped to zero, gap
        // swallowed) so the bar rests as just the circle without a first-frame
        // flash of the full width. `set_telescope` drives these on toggle.
        let style = wrapper.unchecked_ref::<HtmlElement>().style();
        let _ = style.set_property("max-width", "0px");
        let _ = style.set_property("margin-left", "-2px");
        // Insert the wrapper where the tile is, then move the tile inside it.
        if let Some(parent) = tile.parent_node() {
            let _ = parent.insert_before(&wrapper, Some(tile));
            let _ = wrapper.append_child(tile);
        }
    }
    fab.class_list().add_2("fab--anim", "fab--collapsed").ok();
}

/// Attach the FAB's NATIVE click/dblclick gesture listeners. Because only the
/// circle is draggable (see `attach_drag`), the pointer is never captured over a
/// segment, so the browser's own `click`/`dblclick` fire normally — no manual
/// tap detection, no timers. The listeners sit on the `<tonk-fab>` host and
/// route by the event target:
///
/// - CIRCLE cap: `click` folds/expands the bar, `dblclick` pauses/resumes sync.
/// - DISCLOSURE border: `click` reveals/hides its section.
/// - SPOT segment: `click` toggles the switcher menu.
/// - SHARE segment: `click` toggles the roster menu.
///
/// The name/spot editables edit on their OWN native `dblclick` (editable.rs).
fn attach_gestures(element: &HtmlElement) {
    let el_click = element.clone();
    let on_click = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
        let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
            return;
        };
        if t.closest(".fab__cap-l").ok().flatten().is_some() {
            // Only the FIRST click of a click sequence folds the bar; the second
            // click of a double (`detail == 2`) is left for `dblclick` to pause,
            // so a double-click doesn't fold-then-fold back.
            if e.detail() <= 1 {
                toggle_telescope(&el_click);
            }
        } else if let Some(border) = t.closest(".fab__disclose").ok().flatten()
            && let Some(section) = border.get_attribute("data-section")
        {
            toggle_section(&el_click, &section);
        } else if t
            .closest(".fab__menu, .fab__share-menu")
            .ok()
            .flatten()
            .is_some()
        {
            // A click inside an open menu acts on that menu's own row.
        } else if let Some(seg) = t.closest(".fab__repo").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__share");
        } else if let Some(seg) = t.closest(".fab__share").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__repo");
        }
    });

    let el_dbl = element.clone();
    let on_dbl = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
        // Double-clicking the circle pauses/resumes sync (its ring already shows
        // the state). The browser fires one `click` (which folds once) before
        // this `dblclick`; reverse that fold so a double-click reads as ONLY a
        // pause, not a fold. Double-click on editables is handled by the editable.
        if let Some(t) = e.target().and_then(|t| t.dyn_into::<Element>().ok())
            && t.closest(".fab__cap-l").ok().flatten().is_some()
        {
            toggle_telescope(&el_dbl);
            trigger_pause_toggle(&el_dbl);
        }
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())
        .ok();
    target
        .add_event_listener_with_callback("dblclick", on_dbl.as_ref().unchecked_ref())
        .ok();
    on_click.forget();
    on_dbl.forget();
}

/// Reveal or hide a disclosure section (`account` / `share`) by toggling the
/// matching `fab--show-<section>` class on `.fab`, then re-run the telescope so
/// the now-shown/hidden segments animate to their new widths. Keeps the border's
/// `title` in step for the tooltip ("show …" ⇄ "hide …").
fn toggle_section(element: &HtmlElement, section: &str) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let class = format!("fab--show-{section}");
    let showing = !fab.class_list().contains(&class);
    fab.class_list().toggle_with_force(&class, showing).ok();
    // Update the border tooltip for the new state.
    let title = if showing {
        format!("hide {section}")
    } else {
        format!("show {section}")
    };
    if let Some(border) = fab
        .query_selector(&format!(".fab__disclose[data-section=\"{section}\"]"))
        .ok()
        .flatten()
    {
        let _ = border.set_attribute("title", &title);
    }
    // Re-flow the telescope to the new set of shown tiles (unless collapsed).
    if !fab.class_list().contains("fab--collapsed") {
        set_telescope(element, &fab, false);
    }
}

/// Open (or close) the dropdown owned by `seg` by toggling its `is-open` class,
/// closing the other menu (matched by `other_sel`) so only one is open at a
/// time. Reorients the opened menu to the current viewport half.
fn toggle_menu(element: &HtmlElement, seg: &Element, other_sel: &str) {
    if let Some(other) = element.query_selector(other_sel).ok().flatten() {
        other.class_list().remove_1("is-open").ok();
    }
    let opening = !seg.class_list().contains("is-open");
    seg.class_list().toggle_with_force("is-open", opening).ok();
    if opening {
        apply_menu_direction(element);
    }
}

/// Toggle the telescope open/closed: flip `fab--collapsed` on `.fab` and drive
/// each `.fab__tele` tile's `max-width` / `margin-left` / staggered
/// `transition-delay`, then schedule the post-animation `settled` state that
/// unclamps `max-width` so expanded content can reflow freely.
fn toggle_telescope(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let collapsing = !fab.class_list().contains("fab--collapsed");
    set_telescope(element, &fab, collapsing);
    // Expanding reorients the dropdowns to the current viewport half.
    if !collapsing {
        apply_menu_direction(element);
    }
}

/// Drive the telescope to the given state. `collapsing = true` retracts the
/// tiles into the circle; `false` unfolds them to their measured widths.
fn set_telescope(element: &HtmlElement, fab: &Element, collapsing: bool) {
    let tiles = telescope_tiles(fab);
    let count = tiles.len();

    // Clear any prior settle timer + `settled` class: while animating, tiles
    // must be clamped (overflow hidden) so `max-width` can drive them.
    if let Some(id_str) = element.dataset().get("settleTimer") {
        if let Ok(id) = id_str.parse::<i32>() {
            clear_timeout(id);
        }
        element.dataset().delete("settleTimer");
    }
    fab.class_list().remove_1("fab--settled").ok();

    // Measure natural widths BEFORE mutating the state class, so an
    // already-expanded tile reports its true width (see `measure_tile_widths`).
    let widths = if collapsing {
        Vec::new()
    } else {
        measure_tile_widths(&tiles)
    };

    for (i, tile) in tiles.iter().enumerate() {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let delay = telescope_delay_ms(i, count, collapsing);
        let _ = style.set_property("transition-delay", &format!("{delay}ms"));
        // A tile stays collapsed if the whole bar is folding OR it wraps a
        // section the disclosure ladder currently hides (account / share).
        let hidden = collapsing || tile_section_hidden(fab, tile);
        // Mark hidden tiles so the post-settle `overflow: visible; max-width:
        // none` unclamp SKIPS them — otherwise a hidden section would reappear
        // (and its absolutely-positioned menu overlay the page) once settled.
        tile.class_list()
            .toggle_with_force("fab__tele--hidden", hidden)
            .ok();
        if hidden {
            let _ = style.set_property("max-width", "0px");
            let _ = style.set_property("margin-left", "-2px");
        } else {
            let w = widths.get(i).copied().unwrap_or(0.0);
            let _ = style.set_property("max-width", &format!("{w}px"));
            let _ = style.set_property("margin-left", "0px");
        }
    }

    if collapsing {
        fab.class_list().add_1("fab--collapsed").ok();
    } else {
        fab.class_list().remove_1("fab--collapsed").ok();
        // After the sweep, mark settled so `max-width` unclamps (`none`) and
        // the expanded content can reflow (e.g. a growing invite link).
        schedule_settle(element, fab, count);
    }
}

/// Whether a telescope tile wraps a disclosure section (`.fab__account` /
/// `.fab__share`, tagged `data-section`) that the ladder currently hides — i.e.
/// `.fab` lacks the matching `fab--show-<section>`. Borders and the always-shown
/// spot are never hidden this way. Such tiles stay at zero width even when the
/// bar is expanded, until their disclosure border reveals them.
fn tile_section_hidden(fab: &Element, tile: &Element) -> bool {
    // Only SECTION segments gate on disclosure — borders (`.fab__disclose`) are
    // always shown when the bar is expanded, so look for a `.fab__seg` child
    // (not a border) carrying `data-section`.
    let Some(section) = tile
        .query_selector(".fab__seg[data-section]")
        .ok()
        .flatten()
        .and_then(|el| el.get_attribute("data-section"))
    else {
        return false;
    };
    !fab.class_list().contains(&format!("fab--show-{section}"))
}

/// Collect the `.fab__tele` wrapper tiles in DOM order.
fn telescope_tiles(fab: &Element) -> Vec<Element> {
    let mut out = Vec::new();
    let children = fab.children();
    for i in 0..children.length() {
        if let Some(child) = children.item(i)
            && child.class_list().contains("fab__tele")
        {
            out.push(child);
        }
    }
    out
}

/// Measure each tile's natural width by momentarily unclamping it (max-width
/// none, overflow visible, no negative margin), reading the box, then
/// restoring the inline styles. Mirrors the wireframe's `measure()`.
fn measure_tile_widths(tiles: &[Element]) -> Vec<f64> {
    let mut widths = Vec::with_capacity(tiles.len());
    for tile in tiles {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let saved_mw = style.get_property_value("max-width").unwrap_or_default();
        let saved_ov = style.get_property_value("overflow").unwrap_or_default();
        let saved_ml = style.get_property_value("margin-left").unwrap_or_default();
        let _ = style.set_property("max-width", "none");
        let _ = style.set_property("overflow", "visible");
        let _ = style.set_property("margin-left", "0px");
        let w = tile.get_bounding_client_rect().width().ceil() + 1.0;
        // Restore (empty string removes the inline prop).
        let _ = style.set_property("max-width", &saved_mw);
        let _ = style.set_property("overflow", &saved_ov);
        let _ = style.set_property("margin-left", &saved_ml);
        widths.push(w);
    }
    widths
}

/// Schedule the `fab--settled` class after the telescope finishes expanding, so
/// each tile's `max-width` unclamps and the content can reflow past its
/// measured width. Stashes the timer id so a re-toggle (or disconnect) cancels it.
fn schedule_settle(element: &HtmlElement, fab: &Element, count: usize) {
    let fab_for_settle = fab.clone();
    let settle_once = Closure::<dyn Fn()>::new(move || {
        fab_for_settle.class_list().add_1("fab--settled").ok();
    });
    let settle_fn = settle_once
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    settle_once.forget();
    let id = set_timeout(&settle_fn, telescope_settle_ms(count) as i32);
    element.dataset().set("settleTimer", &id.to_string()).ok();
}

/// The circle's double-click — pause or resume, whichever the current state
/// isn't. `tonk:pause-sync` is a single TOGGLE command, so the same submit both
/// pauses and resumes; only the confirmation UX differs by direction:
///
/// - When SYNCED, we open the `#fab-pause-sync` confirm dialog — pausing has a
///   consequence (changes stop propagating), so we confirm first.
/// - When already PAUSED, we resume IMMEDIATELY — resuming is safe and needs no
///   confirmation — by submitting the toggle form directly (the same form the
///   dialog's "Pause sync" button submits).
///
/// The live state comes off the `<ui-sync-status>` disc, whose subscription
/// stamps a `.sync--paused` modifier class as the status changes.
fn trigger_pause_toggle(element: &HtmlElement) {
    if is_sync_paused(element) {
        submit_pause_form(element);
    } else {
        open_pause_dialog(element);
    }
}

/// Whether sync is currently paused, read from the `<ui-sync-status>` disc's
/// state modifier class (`.sync--paused`), which its subscription keeps live.
fn is_sync_paused(element: &HtmlElement) -> bool {
    element
        .query_selector(".sync--paused")
        .ok()
        .flatten()
        .is_some()
}

/// Open the pause-sync confirm dialog. We call the WebAwesome dialog's `.show()`
/// (not just `open`) so the guest's modal wiring — which listens for `wa-show`
/// to size the sealed iframe — fires, the same path a `data-dialog="open …"`
/// control triggers. Falls back to the `open` attribute if `.show()` is absent.
fn open_pause_dialog(element: &HtmlElement) {
    let Some(dialog) = element.query_selector("#fab-pause-sync").ok().flatten() else {
        return;
    };
    if let Ok(show) = Reflect::get(dialog.as_ref(), &"show".into())
        && let Ok(show) = show.dyn_into::<Function>()
    {
        let _ = show.call0(dialog.as_ref());
    } else {
        let _ = dialog.set_attribute("open", "");
    }
}

/// Resume directly: submit the toggle form the dialog's Pause button targets,
/// which carries the `onsubmit=tonk:pause-sync` binding routed to the space
/// branch. `requestSubmit()` (not `submit()`) so the form's `submit` event —
/// which the command delegation listens for — actually fires.
fn submit_pause_form(element: &HtmlElement) {
    let Some(form) = element
        .query_selector("#fab-pause-sync-form")
        .ok()
        .flatten()
    else {
        return;
    };
    if let Ok(request_submit) = Reflect::get(form.as_ref(), &"requestSubmit".into())
        && let Ok(request_submit) = request_submit.dyn_into::<Function>()
    {
        let _ = request_submit.call0(form.as_ref());
    }
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
        // Only the primary button drags, and only from the CIRCLE cap — that is
        // the sole draggable handle. A press anywhere else on the bar (a
        // segment, an editable, a menu) is left entirely to native click.
        if e.button() != 0 {
            return;
        }
        let on_circle = e
            .target()
            .and_then(|t| t.dyn_into::<Element>().ok())
            .and_then(|el| el.closest(".fab__cap-l").ok().flatten())
            .is_some();
        if !on_circle {
            return;
        }
        // DELTA-based drag: remember the pointer's start AND the element's start
        // `left`/`top`, then translate by the pointer delta. No grab-offset or
        // rect math — the element moves 1:1 with the cursor and drops exactly
        // where released. We do NOT capture or `preventDefault` here so a plain
        // press still fires native click/dblclick; capture is taken in
        // `pointermove` only once the press passes the drag threshold.
        let rect = el_down.get_bounding_client_rect();
        el_down
            .dataset()
            .set("fabStartLeft", &rect.left().to_string())
            .ok();
        el_down
            .dataset()
            .set("fabStartTop", &rect.top().to_string())
            .ok();
        el_down
            .dataset()
            .set("fabDownX", &(e.client_x() as f64).to_string())
            .ok();
        el_down
            .dataset()
            .set("fabDownY", &(e.client_y() as f64).to_string())
            .ok();
        el_down.dataset().set("fabPressing", "1").ok();
        el_down.dataset().delete("fabMoved");
    });

    let el_move = element.clone();
    let on_move = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_move.dataset().get("fabPressing").is_none() {
            return;
        }
        let dx = e.client_x() as f64 - read_data_f64(&el_move, "fabDownX");
        let dy = e.client_y() as f64 - read_data_f64(&el_move, "fabDownY");
        // Promote to a DRAG once past the dead zone; take capture only then, so a
        // stationary press stays a plain native click.
        if el_move.dataset().get("fabMoved").is_none() {
            if dx.hypot(dy) < DRAG_THRESHOLD_PX {
                return;
            }
            el_move.dataset().set("fabMoved", "1").ok();
            el_move.set_pointer_capture(e.pointer_id()).ok();
            if let Some(fab) = el_move.query_selector(".fab").ok().flatten() {
                fab.class_list().add_1("dragging").ok();
            }
        }
        e.prevent_default();
        let left = read_data_f64(&el_move, "fabStartLeft") + dx;
        let top = read_data_f64(&el_move, "fabStartTop") + dy;
        track_position(&el_move, left, top);
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabPressing").is_none() {
            return;
        }
        el_up.dataset().delete("fabPressing");
        let moved = el_up.dataset().get("fabMoved").is_some();
        // A press that never moved is a plain click — leave it to native
        // click/dblclick; do nothing here.
        if !moved {
            return;
        }
        el_up.release_pointer_capture(e.pointer_id()).ok();
        if let Some(fab) = el_up.query_selector(".fab").ok().flatten() {
            fab.class_list().remove_1("dragging").ok();
        }
        let dx = e.client_x() as f64 - read_data_f64(&el_up, "fabDownX");
        let dy = e.client_y() as f64 - read_data_f64(&el_up, "fabDownY");
        let left = read_data_f64(&el_up, "fabStartLeft") + dx;
        let top = read_data_f64(&el_up, "fabStartTop") + dy;
        let (x, y) = clamp_to_viewport(left, top);
        settle_position(&el_up, x, y);
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

/// Read a numeric `data-*` value off the element, defaulting to 0.
fn read_data_f64(el: &HtmlElement, key: &str) -> f64 {
    el.dataset()
        .get(key)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
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
    // Settle at exactly the tracked `left`/`top` — same coordinate space
    // `track_position` uses during the drag — so the FAB drops precisely where
    // the cursor released it, with no re-anchoring drift. (Corner-anchoring on
    // viewport resize is deferred; it fought the wide host box and the
    // row-reverse geometry.)
    let style = el.style();
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let _ = style.set_property("left", &format!("{}px", left.max(0.0)));
    let _ = style.set_property("top", &format!("{}px", top.max(0.0)));
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

    // The bar has more than one menu — the repo switcher (`.fab__menu`, first
    // match) and the share roster (`.fab__share-menu`) — so reorient each.
    // `query_selector` returns only the first match, so query them separately.
    let orient = |menu: &Element| {
        let cl = menu.class_list();
        if opens_down {
            cl.remove_1("opens-up").ok();
            cl.add_1("opens-down").ok();
        } else {
            cl.remove_1("opens-down").ok();
            cl.add_1("opens-up").ok();
        }
    };
    if let Some(menu_el) = elem.query_selector(".fab__menu").ok().flatten() {
        orient(&menu_el);
    }
    if let Some(menu_el) = elem.query_selector(".fab__share-menu").ok().flatten() {
        orient(&menu_el);
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
