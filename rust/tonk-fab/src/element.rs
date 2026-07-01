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
//! - On connect it wraps each collapsible segment for the telescope, then
//!   restores the persisted STATE — position and expansion shape (collapsed +
//!   which disclosures are shown) — from `state:fab`, or seeds the
//!   `defaultposition` / `defaultexpansion` attributes on a first load. The
//!   read is load-only (never subscribed): later fact changes don't move the
//!   live bar, like a form control's `defaultValue`. Position is written back
//!   only on drop; expansion on each telescope/disclosure change.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{
    clamp_position, expansion_claim_json, position_claim_json, submenu_opens_down,
    telescope_delay_ms, telescope_settle_ms,
};
use custom_elements::CustomElement;
use js_sys::Promise;
use js_sys::{Function, Object, Reflect};
use serde_json::Value;
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
        // Hide until the persisted state resolves, so the restore doesn't flash
        // the default shape/position first. `reveal` clears this once seeded.
        let _ = style.set_property("visibility", "hidden");

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
        // Restore the persisted state (position + expansion), or seed defaults.
        restore_state(this);
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
    // Write back the new expansion shape so the next load restores it.
    persist_current_expansion(element);
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
    // Write back the new expansion shape so the next load restores it.
    persist_current_expansion(element);
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

/// Read the current expansion shape off `.fab`'s state classes and persist it.
/// Called after every telescope/disclosure change so the write reflects exactly
/// what the DOM now shows. `element` is the `<tonk-fab>` host — the claim is
/// dispatched from it so the event bubbles to the `<tonk-host>` ancestor.
fn persist_current_expansion(element: &HtmlElement) {
    let Some(fab) = element.query_selector(".fab").ok().flatten() else {
        return;
    };
    let classes = fab.class_list();
    let collapsed = classes.contains("fab--collapsed");
    let account = classes.contains("fab--show-account");
    let share = classes.contains("fab--show-share");
    persist_expansion(element, collapsed, account, share);
}

/// Apply a restored expansion shape to the bar: set the disclosure classes,
/// then drive the telescope to the collapsed-or-expanded state (which honours
/// those classes via `tile_section_hidden`). No write-back — this is the load
/// path, and re-persisting the value we just read would be a redundant claim.
fn apply_expansion(this: &HtmlElement, collapsed: bool, account: bool, share: bool) {
    let Some(fab) = this.query_selector(".fab").ok().flatten() else {
        return;
    };
    fab.class_list()
        .toggle_with_force("fab--show-account", account)
        .ok();
    fab.class_list()
        .toggle_with_force("fab--show-share", share)
        .ok();
    set_telescope(this, &fab, collapsed);
    if !collapsed {
        apply_menu_direction(this);
    }
}

/// The resting expansion on a true first load (empty query), from the
/// `defaultexpansion` attribute. `collapsed` rests as just the circle;
/// `spot` (the default) expands the bar to the circle + space-name segment with
/// both disclosures hidden. Unknown / absent → `spot`.
fn default_expansion(this: &HtmlElement) {
    let value = this
        .get_attribute("defaultexpansion")
        .unwrap_or_else(|| "spot".to_string());
    let collapsed = value == "collapsed";
    // `spot` and the fallback both rest expanded with account + share hidden.
    apply_expansion(this, collapsed, false, false);
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
        persist_position(&el_up, x as u32, y as u32);
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
    let (vw, vh) = viewport_size();
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
/// `right` when the CIRCLE's center is in the right half of the viewport (else
/// `left`), and to `bottom` when in the bottom half (else `top`). Anchoring to
/// the near edge keeps the FAB in the same corner when the viewport resizes,
/// instead of drifting from a fixed top-left offset. When docked right, the bar
/// gets `fab--dock-right` so it row-reverses — the circle welds to the right
/// edge and the segments telescope INWARD (leftward). Used at drop and restore.
fn settle_position(el: &HtmlElement, left: f64, top: f64) {
    let style = el.style();
    let (vw, vh) = viewport_size();

    // Anchor by the CIRCLE's center, not the whole (variable-width) bar: the
    // circle is the fixed handle and the always-visible part, so which half it
    // sits in decides the docking edge. `.fab__cap-l` is `CIRCLE_SIZE` wide.
    let circle_center_x = left + CIRCLE_SIZE / 2.0;
    let circle_center_y = top + CIRCLE_SIZE / 2.0;
    let dock_right = circle_center_x > vw / 2.0;
    let dock_bottom = circle_center_y > vh / 2.0;

    // Toggle the row-reverse dock class so the telescope expands inward from the
    // docked edge (the CSS `.fab--dock-right` swaps the cap radii + direction).
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list()
            .toggle_with_force("fab--dock-right", dock_right)
            .ok();
    }

    // Horizontal: pin to whichever edge the circle is nearer, so a resize keeps
    // the circle its measured distance from that edge. The circle is the anchor,
    // so the right offset is measured from the circle's right edge.
    let _ = style.remove_property("left");
    let _ = style.remove_property("right");
    if dock_right {
        let right = (vw - (left + CIRCLE_SIZE)).max(0.0);
        let _ = style.set_property("right", &format!("{right}px"));
    } else {
        let _ = style.set_property("left", &format!("{}px", left.max(0.0)));
    }

    // Vertical: same, top vs bottom.
    let _ = style.remove_property("top");
    let _ = style.remove_property("bottom");
    if dock_bottom {
        let bottom = (vh - (top + CIRCLE_SIZE)).max(0.0);
        let _ = style.set_property("bottom", &format!("{bottom}px"));
    } else {
        let _ = style.set_property("top", &format!("{}px", top.max(0.0)));
    }
}

/// The viewport `(width, height)` in CSS px, with sane fallbacks.
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

/// Persist `(x, y)` on drop as a profile claim. Position is asserted ONLY here
/// (on pointer-up), never mid-drag, and independently of the expansion claim.
fn persist_position(this: &HtmlElement, x: u32, y: u32) {
    transact_claim(this, position_claim_json(x, y));
}

/// Persist the bar's expansion shape (collapsed + which sections shown) as a
/// profile claim, written back on a telescope/disclosure change. Independent of
/// the position claim, so it never disturbs the persisted `x`/`y`.
fn persist_expansion(this: &HtmlElement, collapsed: bool, account: bool, share: bool) {
    transact_claim(this, expansion_claim_json(collapsed, account, share));
}

/// Fire-and-forget a profile-branch claim by dispatching a `tonk-claim`
/// CustomEvent on the FAB element. The outer `<tonk-host>` catches the bubbling
/// event, routes it (here `profile: true`, `branch: "meta"` targets the profile
/// meta branch), executes the transact, and writes a result Promise back — the
/// same path `<tonk-display>` uses, so the FAB shares the host's routing and
/// permissions rather than calling a global `window.tonk`. The result Promise is
/// dropped: the persisted state is load-only and never fed back into the live bar.
fn transact_claim(this: &HtmlElement, claim: Value) {
    let Some(request) = value_to_js(&claim) else {
        return;
    };
    let detail = Object::new();
    let _ = Reflect::set(&detail, &"request".into(), &request);
    apply_profile_route(&detail);
    // Fire-and-forget: the result Promise is dropped (load-only persistence).
    let _ = dispatch_host_event(this, "tonk-claim", &detail);
}

/// Stamp a `detail` for the profile meta branch: `profile = true` targets the
/// profile-as-repository endpoint, `branch = "meta"` names its branch. Mirrors
/// `consumer::apply_route(profile = true)`.
fn apply_profile_route(detail: &Object) {
    let _ = Reflect::set(detail, &"profile".into(), &JsValue::TRUE);
    let _ = Reflect::set(detail, &"branch".into(), &JsValue::from_str("meta"));
}

/// Dispatch a bubbling, composed, cancelable `tonk-*` CustomEvent carrying
/// `detail` on `consumer`. The `<tonk-host>` ancestor handles it and (for
/// one-shots) calls `preventDefault` + writes `detail.result`. Returns the
/// `detail.result` Promise the host wrote, or `None` if no host handled it.
fn dispatch_host_event(consumer: &HtmlElement, name: &str, detail: &Object) -> Option<Promise> {
    let init = web_sys::CustomEventInit::new();
    init.set_detail(detail);
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(true);
    let ev = web_sys::CustomEvent::new_with_event_init_dict(name, &init).ok()?;
    let _ = consumer.dispatch_event(&ev);
    if !ev.default_prevented() {
        return None;
    }
    Reflect::get(detail, &"result".into())
        .ok()
        .and_then(|v| v.dyn_into::<Promise>().ok())
}

/// Parse a `serde_json::Value` into a JS object via `JSON.parse` (the host
/// accepts any structured-clonable object).
fn value_to_js(value: &Value) -> Option<JsValue> {
    let json_str = serde_json::to_string(value).ok()?;
    js_sys::JSON::parse(&json_str).ok()
}

/// On connect, seed the bar from the persisted FAB state (`state:fab`). This is
/// a load-only read (no subscription): a later fact change never feeds back into
/// the live element, like a form control's `defaultValue`. When a query is empty
/// (a first load, or an older profile that only persisted position), the
/// `defaultposition` / `defaultexpansion` attributes supply that aspect instead.
///
/// Position and expansion are queried SEPARATELY, because each is an all-required
/// join and an older profile has position facts but no expansion facts — a single
/// combined query would return nothing and lose the persisted position. Two
/// queries let each aspect restore or default on its own. The bar is revealed
/// only after both resolve, so no default shape flashes first.
fn restore_state(this: &HtmlElement) {
    // Dispatch the reads through `<tonk-host>` (the same path `<tonk-display>`
    // uses). If no host handled them (`None`), there's nothing to read from —
    // seed the defaults and reveal.
    let position = run_query(this, &position_query_body());
    let expansion = run_query(this, &expansion_query_body());
    if position.is_none() && expansion.is_none() {
        seed_defaults(this);
        reveal(this);
        return;
    }

    let this = this.clone();
    spawn_local(async move {
        // Position: persisted x/y, else the default.
        match await_rows(position)
            .await
            .and_then(|r| read_position(&first_row(&r)))
        {
            Some((x, y)) => settle_position(&this, x, y),
            None => default_position(&this),
        }
        // Expansion: persisted shape, else the `defaultexpansion` attribute.
        match await_rows(expansion)
            .await
            .and_then(|r| read_expansion(&first_row(&r)))
        {
            Some((collapsed, account, share)) => apply_expansion(&this, collapsed, account, share),
            None => default_expansion(&this),
        }
        reveal(&this);
    });
}

/// The persisted-position query body (`state:fab` x/y).
fn position_query_body() -> Value {
    serde_json::json!({
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
    })
}

/// The persisted-expansion query body (`state:fab` collapsed/account/share).
fn expansion_query_body() -> Value {
    serde_json::json!({
        "terms": {
            "this": "state:fab",
            "collapsed": { "?": { "name": "collapsed" } },
            "account": { "?": { "name": "account" } },
            "share": { "?": { "name": "share" } }
        },
        "predicate": {
            "description": "Persisted FAB expansion (profile-meta claim).",
            "with": {
                "collapsed": { "the": "xyz.tonk.fab/collapsed", "cardinality": "one", "as": "Boolean" },
                "account": { "the": "xyz.tonk.fab/account", "cardinality": "one", "as": "Boolean" },
                "share": { "the": "xyz.tonk.fab/share", "cardinality": "one", "as": "Boolean" }
            }
        }
    })
}

/// Dispatch a profile-branch `tonk-query` on the FAB element and return the
/// host's result Promise, or `None` when no `<tonk-host>` handled it. Routed to
/// the profile meta branch — the same event path `<tonk-display>` reads through.
fn run_query(this: &HtmlElement, body: &Value) -> Option<Promise> {
    let query = value_to_js(body)?;
    let detail = Object::new();
    let _ = Reflect::set(&detail, &"query".into(), &query);
    apply_profile_route(&detail);
    dispatch_host_event(this, "tonk-query", &detail)
}

/// Await a query Promise into its `Conclusion[]` rows value, or `None`.
async fn await_rows(promise: Option<Promise>) -> Option<JsValue> {
    let promise = promise?;
    wasm_bindgen_futures::JsFuture::from(promise).await.ok()
}

/// The first row of a `Conclusion[]` value, or a null `JsValue` when empty.
fn first_row(rows: &JsValue) -> JsValue {
    js_sys::Array::from(rows).get(0)
}

/// A conclusion's `fields` sub-object: `{ this, fields: { <term>: value } }` is
/// the wire shape a query row takes, so the projected values live under
/// `fields`, not at the row's top level (mirrors `<ui-sync-status>`'s frame read).
fn row_fields(row: &JsValue) -> Option<JsValue> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    Reflect::get(row, &"fields".into())
        .ok()
        .filter(|f| !f.is_undefined() && !f.is_null())
}

/// Read `x`/`y` off a persisted `state:fab` conclusion.
fn read_position(row: &JsValue) -> Option<(f64, f64)> {
    let fields = row_fields(row)?;
    let x = Reflect::get(&fields, &"x".into())
        .ok()
        .and_then(|v| v.as_f64())?;
    let y = Reflect::get(&fields, &"y".into())
        .ok()
        .and_then(|v| v.as_f64())?;
    Some((x, y))
}

/// Read `collapsed`/`account`/`share` off a persisted `state:fab` conclusion.
/// All three must be present (they are written together) or the whole expansion
/// falls back to the default.
fn read_expansion(row: &JsValue) -> Option<(bool, bool, bool)> {
    let fields = row_fields(row)?;
    let collapsed = Reflect::get(&fields, &"collapsed".into())
        .ok()
        .and_then(|v| v.as_bool())?;
    let account = Reflect::get(&fields, &"account".into())
        .ok()
        .and_then(|v| v.as_bool())?;
    let share = Reflect::get(&fields, &"share".into())
        .ok()
        .and_then(|v| v.as_bool())?;
    Some((collapsed, account, share))
}

/// Seed both aspects from their defaults (used when there is no `window.tonk`
/// or the query never resolves).
fn seed_defaults(this: &HtmlElement) {
    default_position(this);
    default_expansion(this);
}

/// Reveal the bar after the restore has run, clearing the connect-time
/// `visibility: hidden` that suppresses a default-shape flash.
fn reveal(this: &HtmlElement) {
    let _ = this.style().remove_property("visibility");
}

/// Apply the first-load position from the `defaultposition` attribute. Only
/// `top-left` is implemented (also the fallback for an absent / unknown value);
/// the attribute is the seam for other resting corners later. A small inset
/// keeps the circle off the very edge.
fn default_position(this: &HtmlElement) {
    // `defaultposition` is read but currently only `top-left` is realized.
    let _ = this.get_attribute("defaultposition");
    settle_position(this, 16.0, 16.0);
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
