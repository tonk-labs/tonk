//! The bar — one object.
//!
//! `[circle 36][space 216][share 144][fold 24][mode 18]`, flush cells on a
//! single frost surface separated by 1px weighted lines. Anchored right the
//! whole run mirrors to `[mode][fold][share][space][circle]`, so the circle
//! keeps the corner — collapse stays one tap from where the thumb already is
//! — and every rung keeps its place relative to the circle. See
//! [`apply_flip`] for why this mirrors rather than swapping bookends alone.
//!
//! The spec's `changes` rung (432px, between space and share) is deliberately
//! absent — see `plan/fabb-conformance.md`. It drives preview / accept /
//! discard / restore over proposals and history points, and nothing in this
//! repo implements either, so building it would be dead chrome. Cell widths
//! and the flush-run geometry are otherwise spec-correct; the bar is simply
//! shorter until the feature it serves exists.
//!
//! ## Attributes
//!
//! - `space` — the space's DID. The mount contract from `profile.yaml`; it
//!   addresses subscriptions, it is not shown.
//! - `label` — the space's display name, which IS shown. Written onto the
//!   host by the headless name subscriber, so the bar renders text it owns
//!   rather than hosting a foreign element inside a cell.
//! - `state` — `synced` | `offline` | `paused`, likewise written by the
//!   headless sync subscriber.
//! - `alert` — changes to review. Collapsed the disc blinks; expanded the
//!   alerted rung washes. Never a colour (law 5).
//! - `collapsed` `up` `flip` `responsive` `static`.
//!
//! The mode pill overrides the system preference for this bar, and the
//! override lasts the session. It cannot yet be remembered: the bar renders
//! inside a sealed guest (`sandbox="allow-scripts"`, an opaque origin) where
//! `localStorage` throws. See `tonk-workspace::ui_mode_switch` for the two
//! routes that would fix it — both belong in their own change.
//!
//! ## Stacks
//!
//! A stack is a light-DOM `<tonk-menu slot="menu" data-for="space|share">`.
//! The bar sizes it from the rung that opened it and positions it one gap
//! below (or above, when `up`).

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Object, Reflect};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, window};

use crate::markup;
use crate::menu;
use crate::shadow::{self, Bound, Edit};

/// Below this the space rung's stack discloses in place rather than flying
/// out sideways: sideways flight needs the room a hover pointer implies.
const INPLACE_MAX_WIDTH_PX: f64 = 640.0;

/// The `responsive` breakpoints, in parent width. `rfold` reduces the bar to
/// circle · space · fold · mode; `rd` drops the strip entirely.
const RFOLD_PX: f64 = 640.0;
const RDROP_PX: f64 = 330.0;

/// Which cell's stack is open, and the live rename if one is running.
#[derive(Default)]
pub(crate) struct BarState {
    /// The open cell's `data-cell` value, if any.
    pub open_cell: Option<String>,
    /// The most recent rename. Deliberately NOT cleared when the edit
    /// settles: the commit runs from inside this `Edit`'s own blur listener,
    /// and dropping it there would free the closure currently executing.
    /// It is released when the next rename replaces it.
    pub edit: Option<Rc<Edit>>,
    /// Whether [`Self::edit`] is still taking input. The presence of an
    /// `Edit` cannot answer that, for the reason above.
    pub editing: bool,
    /// A sub-stack hoisted out of its row for in-place disclosure, and the
    /// row it came from — so `close` can put it back.
    pub hoisted: Option<(Element, Element)>,
}

/// Shared handle to the bar's state, so listeners can reach it.
pub(crate) type Shared = Rc<RefCell<BarState>>;

/// Build the bar's shadow tree and wire its cells.
///
/// Returns the listeners the caller must keep alive.
pub(crate) fn build(this: &HtmlElement, state: &Shared) -> Vec<Bound> {
    let root = shadow::build(this, markup::BAR_CSS, markup::BAR_HTML);
    let mut listeners = Vec::new();

    // The mode half pill — law 8's grammar for free: its fill is solid ink,
    // which inverts with the tokens, so it is lit when dark and dark when
    // light. It writes the attribute AND reports, so a host can persist the
    // choice.
    if let Ok(Some(toggle)) = root.query_selector("[data-cell=toggle]") {
        let host = this.clone();
        listeners.push(shadow::on_click(&toggle, move || {
            let next = if shadow::is_dark(&host) {
                "light"
            } else {
                "dark"
            };
            let _ = host.set_attribute("mode", next);
            shadow::apply_mode(&host);
            propagate(&host);
            let detail = Object::new();
            let _ = Reflect::set(&detail, &"mode".into(), &next.into());
            shadow::emit(&host, "fabb-mode", &detail);
        }));
    }

    if let Ok(Some(fold)) = root.query_selector("[data-cell=fold]") {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::on_click(&fold, move || {
            commit_edit(&host, &shared);
            let Some(wrapper) = wrapper(&host) else {
                return;
            };
            let classes = wrapper.class_list();
            if is_reduced(&host) {
                let _ = classes.remove_1("folded");
                let _ = classes.add_1("xopen");
            } else {
                let _ = classes.remove_1("xopen");
                let _ = classes.add_1("folded");
                close(&host, &shared);
            }
            set_fold_glyph(&host);
            let detail = Object::new();
            let _ = Reflect::set(&detail, &"folded".into(), &is_reduced(&host).into());
            shadow::emit(&host, "fabb-fold", &detail);
        }));
    }

    for cell in ["space", "share"] {
        let Ok(Some(button)) = root.query_selector(&format!("[data-cell={cell}]")) else {
            continue;
        };
        let host = this.clone();
        let shared = state.clone();
        let name = cell.to_string();
        listeners.push(shadow::on_click(&button, move || {
            // A click on the space cell mid-rename is aimed at the text:
            // commit it, do not also open the stack over what was typed.
            if name == "space" && shared.borrow().editing {
                commit_edit(&host, &shared);
                return;
            }
            open(&host, &shared, &name);
            let detail = Object::new();
            let _ = Reflect::set(&detail, &"cell".into(), &name.as_str().into());
            shadow::emit(&host, "fabb-cell", &detail);
        }));
    }

    // A picked row closes the stack unless it owns a sub-stack — and on a
    // coarse pointer, a row with a sub discloses it in place instead of
    // flying out.
    {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(this, "fabb-pick", move |ev| {
            let Some(detail) = ev.dyn_ref::<web_sys::CustomEvent>().map(|e| e.detail()) else {
                return;
            };
            let Ok(item) = Reflect::get(&detail, &"item".into()) else {
                return;
            };
            let Ok(row) = item.dyn_into::<Element>() else {
                return;
            };
            let has_sub = matches!(
                row.query_selector(":scope > tonk-menu[slot=sub]"),
                Ok(Some(_))
            );
            if has_sub && wants_inplace() {
                open_sub(&host, &shared, &row);
            } else if !has_sub {
                close(&host, &shared);
            }
        }));
    }

    // A stack must be dismissable without picking from it. Both listeners are
    // document-scoped because that is where the dismissing gesture lands —
    // anywhere BUT the bar — and both detach with the element (`shadow::Bound`).
    if let Some(document) = window().and_then(|w| w.document()) {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(&document, "pointerdown", move |ev| {
            if shared.borrow().open_cell.is_none() {
                return;
            }
            // The composed path, not `target`: a press inside the bar's own
            // shadow tree retargets to the host, and a press on a slotted
            // stack row is a light-DOM descendant. Both count as inside.
            let host_element: &Element = host.unchecked_ref();
            let path = ev.composed_path();
            let inside = (0..path.length()).any(|index| {
                path.get(index)
                    .dyn_into::<Element>()
                    .is_ok_and(|element| element == *host_element)
            });
            if !inside {
                close(&host, &shared);
            }
        }));

        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::bind(&document, "keydown", move |ev| {
            let Some(key) = ev.dyn_ref::<web_sys::KeyboardEvent>().map(|e| e.key()) else {
                return;
            };
            if key != "Escape" {
                return;
            }
            // A rename in flight owns Escape — it reverts the name, and its
            // own handler stops the event before it reaches here.
            if shared.borrow().editing {
                return;
            }
            close(&host, &shared);
        }));
    }

    listeners.push(shadow::install_visibility_pause(this));
    if let Some(listener) = shadow::install_system_mode(this) {
        listeners.push(listener);
    }

    if this.has_attribute("folded")
        && let Some(wrapper) = wrapper(this)
    {
        let _ = wrapper.class_list().add_1("folded");
    }
    apply_flip(this);
    update(this);
    listeners
}

/// The `.w` wrapper every token and state class hangs on.
fn wrapper(this: &HtmlElement) -> Option<Element> {
    this.shadow_root()?.query_selector(".w").ok().flatten()
}

fn query(this: &HtmlElement, selector: &str) -> Option<Element> {
    this.shadow_root()?.query_selector(selector).ok().flatten()
}

/// Whether the bar is currently reduced to circle · space · fold · mode —
/// by the hand (`folded`) or by the observer (`rfold`, unless overridden).
pub(crate) fn is_reduced(this: &HtmlElement) -> bool {
    let Some(wrapper) = wrapper(this) else {
        return false;
    };
    let classes = wrapper.class_list();
    classes.contains("folded") || (classes.contains("rfold") && !classes.contains("xopen"))
}

/// Sideways flight is a hover-pointer's move.
fn wants_inplace() -> bool {
    let Some(win) = window() else { return false };
    let coarse = win
        .match_media("(pointer: coarse)")
        .ok()
        .flatten()
        .is_some_and(|q| q.matches());
    let narrow = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .is_some_and(|w| w < INPLACE_MAX_WIDTH_PX);
    coarse || narrow
}

/// The flip — anchored right, the bar is a mirror image of itself.
///
/// The bar telescopes away from its anchor, so the handle must sit ON it:
/// collapse stays one tap from the corner the thumb already holds. The whole
/// run mirrors, cells included, so the arrangement RELATIVE TO THE CIRCLE is
/// the same at either edge — the space rung is always the circle's
/// neighbour, share is always out at the fold end.
///
/// This departs from the reference's law 10, which holds content order fixed
/// (`space · changes · share`) and swaps only the bookends. Keeping the cells
/// put meant the space and share rungs traded places relative to the circle
/// when the bar changed sides, which read as the two controls swapping. The
/// mirror is what actually preserves the shape.
///
/// The DOM is genuinely reordered rather than `flex-direction: row-reverse`d,
/// so focus and screen-reader order match the eye.
pub(crate) fn apply_flip(this: &HtmlElement) {
    let (Some(bar), Some(tele), Some(fab), Some(space), Some(share), Some(fold), Some(toggle)) = (
        query(this, ".bar"),
        query(this, ".tele"),
        query(this, ".fab"),
        query(this, ".space"),
        query(this, ".share"),
        query(this, ".fold"),
        query(this, "[data-cell=toggle]"),
    ) else {
        return;
    };
    let flipped = this.has_attribute("flip");
    // Appending a node that is already a child MOVES it, so laying the run
    // out in order is enough to reorder it — no removal pass needed.
    let cells: [&Element; 4] = if flipped {
        // mode · fold · share · space | circle
        [&toggle, &fold, &share, &space]
    } else {
        // circle | space · share · fold · mode
        [&space, &share, &fold, &toggle]
    };
    for cell in cells {
        let _ = tele.append_child(cell);
    }
    if flipped {
        let _ = bar.append_child(&fab);
    } else {
        let _ = bar.insert_before(&fab, bar.first_child().as_ref());
    }
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper.class_list().toggle_with_force("flip", flipped);
    }
    set_fold_glyph(this);
}

/// On the mute fold cell the triangle is DIRECTIONAL: it points the way the
/// cells will travel — toward the anchor when folding, back out when
/// expanding — so the pair mirrors with the flip.
///
/// Semantic-only was tried in the reference and overruled: on a flipped bar
/// it pointed away from the motion it named. `▸ opens / ◂ folds` still holds
/// wherever a word rides the glyph (`open ▸`, `back ◂`).
pub(crate) fn set_fold_glyph(this: &HtmlElement) {
    let Some(fold) = query(this, ".fold") else {
        return;
    };
    let flipped = this.has_attribute("flip");
    let reduced = is_reduced(this);
    let toward_corner = if flipped { "\u{25B8}" } else { "\u{25C2}" };
    let back_out = if flipped { "\u{25C2}" } else { "\u{25B8}" };
    fold.set_text_content(Some(if reduced { back_out } else { toward_corner }));
    let _ = fold.set_attribute(
        "aria-label",
        if reduced { "expand" } else { "fold to space" },
    );
}

/// Open a cell's stack, sized and aligned to the rung that owns it.
pub(crate) fn open(this: &HtmlElement, state: &Shared, cell: &str) {
    commit_edit(this, state);
    if state.borrow().open_cell.as_deref() == Some(cell) {
        close(this, state);
        return;
    }
    close(this, state);

    let Ok(Some(stack)) = this.query_selector(&format!("tonk-menu[data-for=\"{cell}\"]")) else {
        return;
    };
    state.borrow_mut().open_cell = Some(cell.to_string());

    // Exactly one stack is visible at a time.
    if let Ok(all) = this.query_selector_all("tonk-menu[data-for]") {
        for index in 0..all.length() {
            let Some(node) = all.item(index) else {
                continue;
            };
            let Ok(element) = node.dyn_into::<Element>() else {
                continue;
            };
            if element == stack {
                let _ = element.remove_attribute("hidden");
            } else {
                let _ = element.set_attribute("hidden", "");
            }
        }
    }
    shadow::pass_mode(this, &stack);

    let (Some(rung), Some(bar), Some(menus)) = (
        query(this, &format!("[data-cell={cell}]")),
        query(this, ".bar"),
        query(this, ".mw"),
    ) else {
        return;
    };
    let rung: HtmlElement = rung.unchecked_into();
    let bar_width = bar.unchecked_ref::<HtmlElement>().offset_width();
    let menus_style = menus.unchecked_ref::<HtmlElement>().style();

    // Every stack inherits the width of its parent rung.
    let _ = stack
        .unchecked_ref::<HtmlElement>()
        .style()
        .set_property("--fabb-menu-w", &format!("{}px", rung.offset_width()));

    // Left cells hang left-aligned; the share stack aligns its right edge
    // with the right edge of its own rung.
    if cell == "share" {
        let _ = menus_style.set_property("left", "auto");
        let right = bar_width - rung.offset_left() - rung.offset_width();
        let _ = menus_style.set_property("right", &format!("{right}px"));
    } else {
        let _ = menus_style.set_property("right", "auto");
        let _ = menus_style.set_property("left", &format!("{}px", rung.offset_left()));
    }
    let _ = menus.class_list().add_1("on");

    // Cut the underlay's mask now, not on the observer's schedule: a stack
    // that paints one frame unmasked shows glass across its 7px gaps.
    menu::recut_mask(stack.unchecked_ref());
    sync_expanded(this, state);
}

/// Close whatever stack is open, restoring any hoisted sub-stack first.
pub(crate) fn close(this: &HtmlElement, state: &Shared) {
    restore_sub(state);
    state.borrow_mut().open_cell = None;
    if let Some(menus) = query(this, ".mw") {
        let _ = menus.class_list().remove_1("on");
    }
    if let Ok(all) = this.query_selector_all("tonk-menu[data-for]") {
        for index in 0..all.length() {
            let Some(node) = all.item(index) else {
                continue;
            };
            if let Ok(element) = node.dyn_into::<Element>() {
                let _ = element.set_attribute("hidden", "");
            }
        }
    }
    sync_expanded(this, state);
}

/// In-place disclosure — narrow or coarse, the sub-stack replaces its parent
/// in the same column at the rung width.
fn open_sub(this: &HtmlElement, state: &Shared, row: &Element) {
    if state.borrow().hoisted.is_some() {
        return;
    }
    let Ok(Some(sub)) = row.query_selector(":scope > tonk-menu[slot=sub]") else {
        return;
    };
    let Some(parent_stack) = row.closest("tonk-menu[data-for]").ok().flatten() else {
        return;
    };

    let width = parent_stack
        .unchecked_ref::<HtmlElement>()
        .style()
        .get_property_value("--fabb-menu-w")
        .unwrap_or_default();

    let _ = parent_stack.set_attribute("hidden", "");
    let _ = sub.set_attribute("slot", "menu");
    let _ = this.append_child(&sub);
    let _ = sub.remove_attribute("hidden");
    if !width.is_empty() {
        let _ = sub
            .unchecked_ref::<HtmlElement>()
            .style()
            .set_property("--fabb-menu-w", &width);
    }
    shadow::pass_mode(this, &sub);
    menu::recut_mask(sub.unchecked_ref());
    state.borrow_mut().hoisted = Some((sub, row.clone()));
}

/// Put a hoisted sub-stack back in the row it came from.
fn restore_sub(state: &Shared) {
    let Some((sub, row)) = state.borrow_mut().hoisted.take() else {
        return;
    };
    let _ = sub
        .unchecked_ref::<HtmlElement>()
        .style()
        .remove_property("--fabb-menu-w");
    let _ = sub.set_attribute("slot", "sub");
    let _ = row.append_child(&sub);
}

fn sync_expanded(this: &HtmlElement, state: &Shared) {
    let open = state.borrow().open_cell.clone();
    for cell in ["space", "share"] {
        if let Some(button) = query(this, &format!("[data-cell={cell}]")) {
            let _ = button.set_attribute(
                "aria-expanded",
                &(open.as_deref() == Some(cell)).to_string(),
            );
        }
    }
}

/// The space renames in place — reached through `rename` in its own stack.
///
/// The block cursor is already blinking on the last character, so naming a
/// freshly created space costs no navigation: create, land, type.
pub(crate) fn edit_space(this: &HtmlElement, state: &Shared) {
    if state.borrow().editing {
        return;
    }
    let Some(cell) = query(this, ".space") else {
        return;
    };
    let old = this.get_attribute("label").unwrap_or_default();
    let _ = cell.class_list().add_1("editing");

    let host = this.clone();
    let shared = state.clone();
    let original = old.clone();
    let edit = shadow::mount_edit(&cell, &old, move |accepted, value| {
        // No `is_some` guard here: `commit_edit` runs this through an `Rc`
        // clone, so reading the slot back would say nothing about whether
        // the work is still needed. `Edit`'s own `committed` flag is what
        // makes this run exactly once. `try_borrow_mut` because the commit
        // can re-enter from a path that is already inside the cell.
        if let Ok(mut state) = shared.try_borrow_mut() {
            state.editing = false;
        }
        restore_space_cell(&host);
        let settled = if accepted && !value.is_empty() {
            value
        } else {
            original.clone()
        };
        let _ = host.set_attribute("label", &settled);
        update(&host);
        if accepted && settled != original {
            let detail = Object::new();
            let _ = Reflect::set(&detail, &"field".into(), &"space".into());
            let _ = Reflect::set(&detail, &"value".into(), &settled.as_str().into());
            shadow::emit(&host, "fabb-rename", &detail);
        }
    });
    edit.focus_end();
    // Replacing the previous edit here — outside any of its callbacks — is
    // the safe moment to drop it.
    let mut state = state.borrow_mut();
    state.edit = Some(Rc::new(edit));
    state.editing = true;
}

/// Put the space cell back the way `update` expects to find it.
///
/// `mount_edit` empties the cell to install the editable span, which destroys
/// the `.n` span the name is painted into. Without rebuilding it the commit
/// has nowhere to write, the editable span and its block cursor are never
/// removed, and the cursor goes on blinking after Enter.
fn restore_space_cell(this: &HtmlElement) {
    let Some(cell) = query(this, ".space") else {
        return;
    };
    let _ = cell.class_list().remove_1("editing");
    cell.set_inner_html(r#"<span class="n"></span>"#);
}

/// Settle a live rename before the bar does anything else.
pub(crate) fn commit_edit(this: &HtmlElement, state: &Shared) {
    // Clone the handle out and DROP the borrow before committing: the commit
    // callback reaches back into this same `RefCell`, and holding a borrow
    // across it panics at runtime. The `Rc` is what keeps the edit alive for
    // the duration of the call.
    let edit = {
        let state = state.borrow();
        if !state.editing {
            return;
        }
        state.edit.clone()
    };
    if let Some(edit) = edit {
        edit.commit();
    }
    let _ = this;
}

/// Hand the resolved mode to every slotted stack.
pub(crate) fn propagate(this: &HtmlElement) {
    let Ok(stacks) = this.query_selector_all("tonk-menu") else {
        return;
    };
    for index in 0..stacks.length() {
        let Some(node) = stacks.item(index) else {
            continue;
        };
        if let Ok(element) = node.dyn_into::<Element>() {
            shadow::pass_mode(this, &element);
        }
    }
}

/// The sync state the disc renders.
fn state_of(this: &HtmlElement) -> String {
    match this.get_attribute("state").as_deref() {
        Some("offline") => "offline".into(),
        Some("paused") => "paused".into(),
        _ => "synced".into(),
    }
}

/// Repaint everything driven by attributes: the label, the disc, the titles.
pub(crate) fn update(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };

    // Never overwrite text the user is currently typing into.
    let editing = query(this, ".space")
        .map(|cell| cell.class_list().contains("editing"))
        .unwrap_or(false);
    if !editing && let Ok(Some(name)) = root.query_selector(".space .n") {
        name.set_text_content(Some(&this.get_attribute("label").unwrap_or_default()));
    }

    let state = state_of(this);
    if let Ok(Some(disc)) = root.query_selector(".fab .disc") {
        let class = if state == "synced" {
            "disc st".to_string()
        } else {
            format!("disc st {state}")
        };
        disc.set_class_name(&class);
    }
    let alert = this.has_attribute("alert");
    if let Ok(Some(fab)) = root.query_selector(".fab") {
        let suffix = if alert { " — changes to review" } else { "" };
        // Report the PRECISE status where there is one. The disc draws three
        // shapes for eight states, so `revoked` and `conflict` both render a
        // hollow ring — announcing them as merely "offline" would make the
        // difference unreachable to anyone not looking at the pixel.
        let reported = this
            .get_attribute("data-sync-status")
            .map(|status| status.trim_start_matches("sync:").to_string())
            .unwrap_or_else(|| state.clone());
        let _ = fab.set_attribute(
            "aria-label",
            &format!("sync: {reported}{suffix} — collapse / expand · drag to move"),
        );
    }
    // The blink is never the only teller: every rung answers a hover.
    if let Ok(Some(share)) = root.query_selector(".share") {
        let _ = share.set_attribute(
            "title",
            if alert {
                "changes to review"
            } else {
                "share with others"
            },
        );
    }
    if let Ok(Some(toggle)) = root.query_selector("[data-cell=toggle]") {
        let _ = toggle.set_attribute("aria-checked", &shadow::is_dark(this).to_string());
    }
}

/// Apply the `responsive` breakpoints and clamp the pan stage.
pub(crate) fn apply_responsive(this: &HtmlElement, parent_width: f64) {
    let Some(wrapper) = wrapper(this) else { return };
    let classes = wrapper.class_list();
    let _ = classes.toggle_with_force("rfold", parent_width < RFOLD_PX);
    let _ = classes.toggle_with_force("rd", parent_width < RDROP_PX);
    // The pan clamp: the strip never outgrows its stage — it scrolls instead.
    // The two 36px bookends (circle and, at the far end, the caps) stay out
    // of the scrollable width.
    let stage = (parent_width - 72.0).max(0.0);
    let _ = wrapper
        .unchecked_ref::<HtmlElement>()
        .style()
        .set_property("--_telemax", &format!("{stage}px"));
    set_fold_glyph(this);
}
