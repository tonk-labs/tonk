//! The `<tonk-fab>` custom element — a floating, draggable container.
//!
//! Generic affordance: it renders its content as a `position: fixed` box on a
//! high z-index (so it floats over whatever is below) and lets the user drag it
//! around the viewport. It is NOT a portal and uses no iframe — it lives in the
//! same document as its content and moves itself directly. The FAB chrome uses
//! it to float the profile pill over the space content, but nothing here is
//! FAB-specific beyond the `.fab` class names the view supplies.
//!
//! - Telescope collapse/expand: the bar rests EXPANDED (all segments shown).
//!   A plain click on the circle toggles it — the segments after the cap
//!   animate their `max-width` open/closed, staggered, so the bar unfolds from
//!   / retracts into the circle.
//! - Drag: `pointerdown` (not on an interactive descendant) starts a free drag,
//!   capturing the grab offset; promotion closes any open menu and drops the
//!   `fab-dock-*` classes; `pointermove` sets the element's own `left`/`top`
//!   and, every move, resyncs the `fab-mirror` host class LIVE from the drag
//!   HANDLE's current center (the circle cap — held fixed across mirror
//!   flips), so the visual right-anchored flip previews the eventual snap
//!   continuously, not just at drop; `pointerup` SNAPS the FAB to the corner
//!   nearest the HANDLE'S CENTER (the same anchor, so the drop always
//!   matches what the live mirror was just showing) — by swapping its
//!   `fab-dock-*` classes, resyncing `fab-mirror` from the dock, and
//!   persisting the dock as a profile claim via `window.tonk.transact(...)`.
//!   The view stylesheet (profile.yaml) owns the resting pixel position and
//!   the menus' vertical open-direction (both keyed off `fab-dock-*`, which
//!   only exist at rest); the visual right-anchored flips key off
//!   `fab-mirror` instead, since that is the class still present mid-drag. A
//!   press that never moves past a small threshold is a click.
//! - On connect it wraps each collapsible segment for the telescope, restores
//!   the persisted dock (or a default bottom-right), and applies its classes.
//!
//! The element does NOT use Shadow DOM — it is a transparent wrapper.

use crate::logic::{
    DOCK_CLASSES, Dock, corrected_min_width, dock_claim_json, dock_from_conclusions, mirrored,
    nearest_dock, ratchet_min_width, telescope_delay_ms, telescope_settle_ms,
};
use custom_elements::CustomElement;
use js_sys::Promise;
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, MutationObserver, MutationObserverInit, PointerEvent, window};

// web-sys doesn't expose a typed `clearTimeout`/`setTimeout` wrapper in the
// features we have, so we call them via js_sys::Function from the global.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

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
            preload_menu_widths(this);
        }
        // Restore the persisted position and apply it to our own style.
        restore_position(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        // Cancel any pending timers so their closures don't fire against a
        // detached element.
        if let Some(id_str) = this.dataset().get("settleTimer") {
            if let Ok(id) = id_str.parse::<i32>() {
                clear_timeout(id);
            }
            this.dataset().delete("settleTimer");
        }

        // Drop any in-flight press: the window-scoped drag listeners outlive
        // a clone remount, and a press left armed on the old element would
        // let its stale `finish_drag` persist a phantom dock on the next
        // buttons-up move.
        this.dataset().delete("fabPressing");
        this.dataset().delete("fabMoved");
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
/// `fab--settled` state (the bar rests EXPANDED — all segments shown). Mirrors
/// the wireframe's programmatic `wrapTele` — done in JS, not the authored markup,
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
        // Start each wrapper EXPANDED — the bar rests open (every segment shown),
        // not as a bare circle. `max-width: none` + `margin-left: 0` is the same
        // resting geometry `schedule_settle` leaves after an expand; the matching
        // `fab--settled` class below makes the wrappers overflow-visible so the
        // dropdowns can escape. `set_telescope` drives these on toggle.
        let style = wrapper.unchecked_ref::<HtmlElement>().style();
        let _ = style.set_property("max-width", "none");
        let _ = style.set_property("margin-left", "0px");
        // Insert the wrapper where the tile is, then move the tile inside it.
        if let Some(parent) = tile.parent_node() {
            let _ = parent.insert_before(&wrapper, Some(tile));
            let _ = wrapper.append_child(tile);
        }
    }
    // Rest EXPANDED + settled (no `fab--collapsed`), so the first circle click
    // collapses. `fab--anim` still gates the transitions for later toggles.
    fab.class_list().add_2("fab--anim", "fab--settled").ok();
}

/// Attach the FAB's NATIVE click gesture listener. Because only the circle is
/// draggable (see `attach_drag`), the pointer is never captured over a
/// segment, so the browser's own `click` fires normally — no manual tap
/// detection, no timers. The listener sits on the `<tonk-fab>` host and
/// routes by the event target:
///
/// - CIRCLE cap: `click` folds/expands the bar; alt/option-`click` toggles
///   pause ⇄ resume of sync (the cap's ring already shows the state).
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
            if e.alt_key() {
                submit_pause_form(&el_click);
            } else {
                toggle_telescope(&el_click);
            }
        } else if t
            .closest(".fab__menu, .fab__share-menu")
            .ok()
            .flatten()
            .is_some()
        {
            // A click inside an open menu acts on that menu's own row. If
            // it hit an actionable item (a space link, "all spots", "new"),
            // the interaction is complete — retract the dropdown so it
            // doesn't sit open over the next view.
            if t.closest(".fab__menu a, .fab__menu button")
                .ok()
                .flatten()
                .is_some()
                && let Some(seg) = el_click.query_selector(".fab__repo.is-open").ok().flatten()
            {
                let _ = seg.class_list().remove_1("is-open");
            }
        } else if let Some(seg) = t.closest(".fab__repo").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__share");
        } else if let Some(seg) = t.closest(".fab__share").ok().flatten() {
            toggle_menu(&el_click, &seg, ".fab__repo");
        }
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())
        .ok();
    on_click.forget();
}

/// Toggle pause ⇄ resume: submit the hidden pause form (`tonk:view/fab-pause`,
/// mounted space-scoped by the FAB template), whose `onsubmit=tonk:pause-sync`
/// binding fires the toggle command against the space's content branch.
/// `requestSubmit()` (not `submit()`) so the form's `submit` event — which the
/// command delegation listens for — actually fires. The cap's
/// `<ui-sync-status>` ring reflects the result via its live subscription.
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

/// Open (or close) the dropdown owned by `seg` by toggling its `is-open` class,
/// closing the other menu (matched by `other_sel`) so only one is open at a time.
/// The open-direction is CSS, keyed off the FAB's `fab-dock-*` class.
///
/// On open the segment is widened (an eased inline `min-width`) to the menu's
/// natural width when the menu is the wider of the two — the stylesheet's
/// `width: 100%` then makes menu and rung exactly equal. The stamped
/// `min-width` RATCHETS: it is never cleared, so a column keeps its width
/// across open/close and across the other menu's toggles, and only grows —
/// re-measured on each open — when a wider element has entered the menu.
/// (Clearing on close made the bar's columns visibly resize depending on
/// which dropdown was open.)
fn toggle_menu(element: &HtmlElement, seg: &Element, other_sel: &str) {
    // No menu work while the bar is mid-drag (a second pointer's click): the
    // dock classes that anchor an open menu are stripped during a drag, so an
    // open here would float unanchored mid-bar.
    if element
        .query_selector(".fab.dragging")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    if let Some(other) = element.query_selector(other_sel).ok().flatten() {
        other.class_list().remove_1("is-open").ok();
    }
    let opening = !seg.class_list().contains("is-open");
    seg.class_list().toggle_with_force("is-open", opening).ok();
    if opening {
        equalize_menu_width(seg);
    }
}

/// Measure the menu's natural (max-content) width — momentarily overriding
/// the stylesheet's `width: 100%`, reading the box, restoring — and stamp the
/// segment's inline `min-width` to the RATCHETED target (never below a prior
/// stamp — see `ratchet_min_width`). Works whether the menu is open or
/// closed: a closed menu (`display: none`) is forced measurable — laid out at
/// its natural width, invisible and out of the paint — then restored, all
/// within one task, so nothing flashes. Called on open (`toggle_menu`), at
/// connect and on content mutation (`preload_menu_widths`, `observe_menu`),
/// and once fonts finish loading (`refresh_on_fonts_ready`). On the
/// first-ever stamp, pins the segment's current rendered width and flushes
/// layout before the target, so the 0.2s `min-width` ease has a numeric start
/// instead of animating from the unanimatable `auto`.
fn equalize_menu_width(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let natural = menu_natural_width(seg, &menu);
    let seg_el = seg.unchecked_ref::<HtmlElement>();
    let segment = seg.get_bounding_client_rect().width();
    // A prior ratchet stamp, read back off the inline style ("260px" → 260.0).
    let stamped = seg_el
        .style()
        .get_property_value("min-width")
        .ok()
        .and_then(|v| v.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()));
    if let Some(min_width) = ratchet_min_width(natural, segment, stamped) {
        // Give the 0.2s ease a NUMERIC start on the first stamp: `min-width`
        // rests at `auto` (not animatable), so pin the current rendered width
        // and flush layout before stamping the target — otherwise the first
        // (and, ratcheted, usually only) widening snaps instead of easing.
        if stamped.is_none() {
            let _ = seg_el
                .style()
                .set_property("min-width", &format!("{segment}px"));
            let _ = seg_el.offset_width();
        }
        let _ = seg_el
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}

/// Measure the menu's natural (max-content) width, open or closed — a closed
/// menu (`display: none`) is momentarily forced measurable, invisible and out
/// of the paint (`visibility: hidden`); everything is restored before return.
/// Synchronous within one task, so nothing flashes.
fn menu_natural_width(seg: &Element, menu: &Element) -> f64 {
    let style = menu.unchecked_ref::<HtmlElement>().style();
    // A closed menu is `display: none` (no boxes). Force it measurable —
    // invisible and out of the paint (`visibility: hidden`), laid out at its
    // natural width — then restore. All within one task, so no flash.
    let closed = !seg.class_list().contains("is-open");
    if closed {
        let _ = style.set_property("display", "flex");
        let _ = style.set_property("visibility", "hidden");
    }
    let _ = style.set_property("width", "max-content");
    let natural = menu.get_bounding_client_rect().width();
    let _ = style.remove_property("width");
    if closed {
        let _ = style.remove_property("display");
        let _ = style.remove_property("visibility");
    }
    natural
}

/// Restamp `seg`'s width from a FRESH measurement, replacing any ratcheted
/// stamp in both directions — the one-time correction for stamps taken
/// against fallback-font metrics before the Plex face landed. The min-width
/// transition eases the correction, riding the font swap's own reflow.
fn restamp_menu_width(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let natural = menu_natural_width(seg, &menu);
    if let Some(min_width) = corrected_min_width(natural) {
        let _ = seg
            .unchecked_ref::<HtmlElement>()
            .style()
            .set_property("min-width", &format!("{min_width}px"));
    }
}

/// The two dropdown-owning segments.
const MENU_SEGMENTS: [&str; 2] = [".fab__repo", ".fab__share"];

/// The `fab-mirror` host class carrying the visual right-anchored flips —
/// separate from the `fab-dock-*` classes (position + menu vertical
/// direction) because a drag removes those while the mirror must track the
/// bar LIVE across the midline.
const MIRROR_CLASS: &str = "fab-mirror";

/// The grab handle's (circle cap's) viewport center, or `None` if the bar
/// hasn't rendered one. The handle is the drag's anchor: the flip
/// compensation holds it fixed, so decisions keyed on it are stable across
/// mirror flips (the bar's own center moves by nearly a bar-width per flip
/// and would oscillate).
fn handle_center(el: &HtmlElement) -> Option<(f64, f64)> {
    el.query_selector(".fab__cap-l").ok().flatten().map(|c| {
        let r = c.get_bounding_client_rect();
        (r.left() + r.width() / 2.0, r.top() + r.height() / 2.0)
    })
}

/// Set the mirror from the drag HANDLE's current center: mirrored on the
/// right half of the viewport, upright on the left. Called per drag move
/// (apply_dock owns the resting sync). Falls back to the bar-rect center
/// only if the handle is missing.
///
/// A flip row-reverses the bar inside its fixed box, which would teleport
/// the circle — the grab handle under the pointer — to the bar's other end.
/// So a mid-drag flip SHIFTS the bar by the handle's measured displacement
/// (its center before vs after the class toggle): the handle stays put, the
/// bar swings around it. The shift is folded into the drag's stored
/// `fabStartLeft` so the next pointer delta doesn't undo it. Because the
/// flip decision is keyed on the handle — the very point the compensation
/// holds fixed — a compensated flip cannot re-cross the threshold that
/// triggered it: no oscillation, no hysteresis needed. (Keying on the bar's
/// own center would oscillate: the compensation moves it by nearly a
/// bar-width, straight back across the midline.)
fn apply_mirror_from_handle(el: &HtmlElement) {
    let rect = el.get_bounding_client_rect();
    let before = handle_center(el);
    let anchor_x = before.map_or(rect.left() + rect.width() / 2.0, |(x, _)| x);
    let want = mirrored(anchor_x, viewport_width());
    if el.class_list().contains(MIRROR_CLASS) == want {
        return;
    }
    el.class_list().toggle_with_force(MIRROR_CLASS, want).ok();
    // Compensate only while actually dragging: at promotion (and any
    // non-drag call) the bar is at rest geometry and no pointer is anchored.
    if el.dataset().get("fabMoved").is_none() {
        return;
    }
    let (Some(before), Some(after)) = (before, handle_center(el)) else {
        return;
    };
    let shift = before.0 - after.0;
    if shift != 0.0 {
        let left = rect.left() + shift;
        let _ = el.style().set_property("left", &format!("{left}px"));
        let start = read_data_f64(el, "fabStartLeft") + shift;
        el.dataset().set("fabStartLeft", &start.to_string()).ok();
    }
}

/// Close both dropdowns (the ratcheted widths stay). A drag drops the
/// `fab-dock-*` classes that give an open menu its vertical anchor, so an
/// open menu would float mid-bar; dragging with a menu open isn't a state
/// the chrome supports.
fn close_menus(el: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = el.query_selector(sel).ok().flatten() {
            seg.class_list().remove_1("is-open").ok();
        }
    }
}

/// Stamp both segments' ratcheted widths up front and keep them fresh, so a
/// dropdown OPEN never changes the bar: rows render asynchronously (a
/// MutationObserver per menu re-ratchets as content lands) and the Plex face
/// loads asynchronously (a font swap changes metrics but fires no mutation,
/// so `document.fonts.ready` triggers one more pass).
fn preload_menu_widths(element: &HtmlElement) {
    for sel in MENU_SEGMENTS {
        if let Some(seg) = element.query_selector(sel).ok().flatten() {
            equalize_menu_width(&seg);
            observe_menu(&seg);
        }
    }
    refresh_on_fonts_ready(element);
}

/// Re-ratchet `seg`'s width whenever its menu's content changes (rows arrive
/// from live subscriptions well after connect). The observer lives as long as
/// the page (closure forgotten) — one FAB, two menus, so no accounting.
fn observe_menu(seg: &Element) {
    let Some(menu) = seg.query_selector(".fab__menu").ok().flatten() else {
        return;
    };
    let seg_for_cb = seg.clone();
    let cb = Closure::<dyn FnMut(js_sys::Array, web_sys::MutationObserver)>::new(
        move |_records: js_sys::Array, _obs: web_sys::MutationObserver| {
            equalize_menu_width(&seg_for_cb);
        },
    );
    if let Ok(observer) = MutationObserver::new(cb.as_ref().unchecked_ref()) {
        let init = MutationObserverInit::new();
        init.set_child_list(true);
        init.set_subtree(true);
        init.set_character_data(true);
        observer.observe_with_options(&menu, &init).ok();
    }
    cb.forget();
}

/// One authoritative restamp once the fonts land: measurements taken before
/// this point (connect, mutation) used the fallback face, which typically
/// OVER-reports condensed Plex's metrics — and the ratchet those passes use
/// can only widen, never correct an over-wide stamp back down. This pass
/// restamps from a fresh measurement in both directions instead of ratcheting.
fn refresh_on_fonts_ready(element: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let ready = match document.fonts().ready() {
        Ok(p) => p,
        Err(_) => return,
    };
    let el = element.clone();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(ready).await;
        for sel in MENU_SEGMENTS {
            if let Some(seg) = el.query_selector(sel).ok().flatten() {
                restamp_menu_width(&seg);
            }
        }
    });
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

    // Collapsing from the settled state, shown tiles rest at `max-width: none`
    // (see `schedule_settle`), and a `none → 0` transition does not animate.
    // Pin each tile to its current rendered width and flush layout, so the
    // clamp-to-zero below animates from a concrete start value.
    if collapsing {
        for tile in &tiles {
            let w = tile.get_bounding_client_rect().width();
            let style = tile.unchecked_ref::<HtmlElement>().style();
            let _ = style.set_property("max-width", &format!("{w}px"));
        }
        let _ = fab.unchecked_ref::<HtmlElement>().offset_width();
    }

    for (i, tile) in tiles.iter().enumerate() {
        let style = tile.unchecked_ref::<HtmlElement>().style();
        let delay = telescope_delay_ms(i, count, collapsing);
        let _ = style.set_property("transition-delay", &format!("{delay}ms"));
        // A tile is hidden only while the whole bar is folding; when expanded
        // every segment shows (no per-section disclosure gate).
        let hidden = collapsing;
        // Mark hidden tiles so the post-settle `overflow: visible; max-width:
        // none` unclamp SKIPS them while collapsed.
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
        // Unclamp shown tiles: drop the inline `max-width` pinned during the
        // expand animation so each tile now sizes to its content. Inline styles
        // beat the stylesheet's `max-width: none`, so the clamp must be lifted
        // here in JS — otherwise content that grows AFTER the expand (a minted
        // invite link, a longer edited name) overflows its measured box and,
        // with the tile's `justify-content: flex-end`, spills leftward over the
        // neighbouring segment instead of widening the bar.
        for tile in telescope_tiles(&fab_for_settle) {
            if tile.class_list().contains("fab__tele--hidden") {
                continue;
            }
            let style = tile.unchecked_ref::<HtmlElement>().style();
            let _ = style.set_property("max-width", "none");
        }
    });
    let settle_fn = settle_once
        .as_ref()
        .unchecked_ref::<js_sys::Function>()
        .clone();
    settle_once.forget();
    let id = set_timeout(&settle_fn, telescope_settle_ms(count) as i32);
    element.dataset().set("settleTimer", &id.to_string()).ok();
}

/// Attach pointer event listeners for free drag-and-drop. The element moves
/// itself (its own `position: fixed` `left`/`top`); there is no iframe to relay
/// to.
///
/// `pointerdown` stays on the element (only a press starting on the circle cap
/// should arm a drag), but `pointermove`, `pointerup`, and `pointercancel` are
/// attached to the guest `window` instead: a fast flick can outrun the element
/// before its first `pointermove` fires (capture is only taken once the press
/// passes the drag threshold), so element-scoped listeners can lose the
/// pointer mid-drag and never see the release, leaving `fabPressing` stuck set
/// so a later hover resumes a phantom drag. The FAB's overlay iframe is pinned
/// full-viewport while dragging, so the window sees every pointer event;
/// captured events (post-threshold) still bubble there too. `on_move` also
/// carries a stale-press guard: if a move event arrives with no buttons held,
/// the release was already lost, so the drag is finished right there instead
/// of waiting for a `pointerup` that will never come.
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
        // A press with NO button still held means the pointerup was lost
        // (fast flick released outside the element before capture was taken):
        // finish the drag here so a later hover can't resume a phantom press.
        if e.buttons() == 0 {
            finish_drag(&el_move, e.pointer_id());
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
            // A drag can't support an open menu (see `close_menus`): the
            // dock classes it's about to drop are the menu's vertical anchor.
            close_menus(&el_move);
            // Drop the dock class so the stylesheet's `.fab-dock-*` position
            // (which pins `bottom`/`top`) stops fighting the inline `left`/`top`
            // that now tracks the pointer 1:1.
            let cl = el_move.class_list();
            for c in DOCK_CLASSES {
                cl.remove_1(c).ok();
            }
            // The dock classes just vanished — resync the mirror from the
            // live handle immediately so it doesn't flash upright for one frame.
            apply_mirror_from_handle(&el_move);
        }
        e.prevent_default();
        let left = read_data_f64(&el_move, "fabStartLeft") + dx;
        let top = read_data_f64(&el_move, "fabStartTop") + dy;
        track_position(&el_move, left, top);
        apply_mirror_from_handle(&el_move);
    });

    let el_up = element.clone();
    let on_up = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_up.dataset().get("fabPressing").is_none() {
            return;
        }
        finish_drag(&el_up, e.pointer_id());
    });

    let el_cancel = element.clone();
    let on_cancel = Closure::<dyn FnMut(PointerEvent)>::new(move |e: PointerEvent| {
        if el_cancel.dataset().get("fabPressing").is_none() {
            return;
        }
        finish_drag(&el_cancel, e.pointer_id());
    });

    let target: &web_sys::EventTarget = element.unchecked_ref();
    target
        .add_event_listener_with_callback("pointerdown", on_down.as_ref().unchecked_ref())
        .ok();
    // WINDOW-scoped move/up/cancel: a fast flick outruns the element before
    // its first pointermove fires (capture is only taken past the drag
    // threshold), so element-scoped listeners lose the pointer mid-drag and
    // never see the release. The overlay iframe is pinned full-viewport while
    // dragging, so the window sees every event. Captured events (post
    // threshold) still bubble here.
    if let Some(win) = window() {
        let wtarget: &web_sys::EventTarget = win.unchecked_ref();
        for (name, cb) in [
            ("pointermove", on_move.as_ref()),
            ("pointerup", on_up.as_ref()),
            ("pointercancel", on_cancel.as_ref()),
        ] {
            wtarget
                .add_event_listener_with_callback(name, cb.unchecked_ref())
                .ok();
        }
    }
    on_down.forget();
    on_move.forget();
    on_up.forget();
    on_cancel.forget();
}

/// Finish a press: clear the press flags and — if the press had been promoted
/// to a drag — release capture, drop the dragging class, and snap/persist the
/// dock nearest the HANDLE'S CENTER (falling back to the bar-rect center if
/// the handle is missing). The handle is what you drag, and it is the anchor
/// the mirror preview keys on (`apply_mirror_from_handle`), so the snap
/// always agrees with the live preview the bar has been showing throughout
/// the drag. Shared by `pointerup`, `pointercancel`, and the stale-press
/// guard in `pointermove`.
fn finish_drag(el: &HtmlElement, pointer_id: i32) {
    el.dataset().delete("fabPressing");
    let moved = el.dataset().get("fabMoved").is_some();
    if !moved {
        return;
    }
    el.release_pointer_capture(pointer_id).ok();
    if let Some(fab) = el.query_selector(".fab").ok().flatten() {
        fab.class_list().remove_1("dragging").ok();
    }
    let rect = el.get_bounding_client_rect();
    let (center_x, center_y) = handle_center(el).unwrap_or((
        rect.left() + rect.width() / 2.0,
        rect.top() + rect.height() / 2.0,
    ));
    let dock = nearest_dock(center_x, center_y, viewport_width(), viewport_height());
    apply_dock(el, dock);
    persist_dock(dock);
}

/// The viewport height in CSS px, defaulting if unavailable.
fn viewport_height() -> f64 {
    window()
        .and_then(|w| w.inner_height().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(768.0)
}

/// The viewport width in CSS px, defaulting if unavailable.
fn viewport_width() -> f64 {
    window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .unwrap_or(1024.0)
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

/// Dock the FAB by swapping its `fab-dock-*` classes and clearing any drag-time
/// inline offsets, so the view stylesheet's `.fab-dock-*` rules own the resting
/// pixel position (and the submenu open-direction), and sync the `fab-mirror`
/// class from the dock. Used at drop and on restore. Anchoring by class — not
/// a fixed pixel offset — keeps the FAB pinned to its corner when the
/// viewport resizes.
fn apply_dock(el: &HtmlElement, dock: Dock) {
    let style = el.style();
    let _ = style.remove_property("left");
    let _ = style.remove_property("top");
    let _ = style.remove_property("right");
    let _ = style.remove_property("bottom");
    let cl = el.class_list();
    for c in DOCK_CLASSES {
        cl.remove_1(c).ok();
    }
    for c in dock.css_classes() {
        cl.add_1(c).ok();
    }
    // Sync the mirror from the dock, not the rect — at rest the dock IS the
    // truth (a drag drives it from the live handle instead; see
    // `apply_mirror_from_handle`).
    cl.toggle_with_force(MIRROR_CLASS, dock.css_classes()[1] == "fab-dock-right")
        .ok();
}

/// Persist `dock` by calling `window.tonk.transact(request)`. The request is the
/// `TransactRequest` JSON produced by `dock_claim_json`, parsed back to a JS
/// object via `JSON.parse` (the bridge accepts any structured-clonable object).
fn persist_dock(dock: Dock) {
    let claim = dock_claim_json(dock);
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

/// On connect, query the persisted FAB dock from `window.tonk.query(...)` and
/// apply its class. Falls back to the default (bottom-right) dock if none is stored.
fn restore_position(this: &HtmlElement) {
    // Position at the default dock immediately so the FAB is placed on first
    // paint; the async query below swaps in the persisted dock if one exists.
    apply_dock(this, Dock::BottomRight);

    let query_body = serde_json::json!({
        "terms": {
            "this": "state:fab",
            "dock": { "?": { "name": "dock" } }
        },
        "predicate": {
            "description": "Persisted FAB dock (profile claim).",
            "with": {
                "dock": { "the": "xyz.tonk.fab/dock", "cardinality": "one", "as": "Entity" }
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
    // the persisted dock if present.
    if let Ok(promise) = result.dyn_into::<Promise>() {
        let this = this.clone();
        spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(promise).await {
                Ok(rows) => match read_dock_from_rows(&rows) {
                    Some(dock) => apply_dock(&this, dock),
                    None => default_position(&this),
                },
                Err(_) => default_position(&this),
            }
        });
    } else {
        default_position(this);
    }
}

/// Extract the persisted dock from a `Conclusion[]` value returned by
/// `window.tonk.query(...)`. Decodes the JS value to JSON and delegates the
/// row-shape parsing to [`dock_from_conclusions`], which is unit-tested
/// against the `{ this, fields: { dock } }` conclusion shape.
fn read_dock_from_rows(rows: &JsValue) -> Option<Dock> {
    let json = js_sys::JSON::stringify(rows).ok()?.as_string()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    dock_from_conclusions(&value)
}

/// Apply the default dock (bottom-right) to the element.
fn default_position(this: &HtmlElement) {
    apply_dock(this, Dock::BottomRight);
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
