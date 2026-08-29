//! The bar — one object.
//!
//! `[circle 36][space 216][share 144][mode 18]`, flush cells on a
//! single frost surface separated by 1px weighted lines. Anchored right the
//! full run mirrors to `[mode][share][space][circle]`; compact mirrors to
//! `[more][share?][space][circle]`. The sync disc keeps the corner and every
//! rung keeps its place relative to it. See
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
//! - `up` `flip` `responsive` `static`.
//!
//! The mode pill switches the whole app, not only this bar: it paints the
//! bar's own tokens and then calls [`tonk_host::theme`], which relays the
//! change down the frame tree to the space behind it. The override lasts the
//! session — it cannot yet be remembered, because the bar renders inside a
//! sealed guest (`sandbox="allow-scripts"`, an opaque origin) where
//! `localStorage` throws. See `plan/fabb-conformance.md` for the fix.
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

use crate::logic::{self, BarLayout};
use crate::markup;
use crate::menu;
use crate::shadow::{self, Bound, Edit};

/// Below this the space rung's stack discloses in place rather than flying
/// out sideways: sideways flight needs the room a hover pointer implies.
const INPLACE_MAX_WIDTH_PX: f64 = 640.0;
const MENU_VIEWPORT_MARGIN_PX: f64 = 8.0;

pub(crate) const CONNECT_BANNER_ID: &str = "fabb-connect-banner";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cell {
    Sync,
    Space,
    Share,
    More,
}

impl Cell {
    fn selector(self) -> &'static str {
        match self {
            Self::Sync => "[data-cell=sync]",
            Self::Space => "[data-cell=space]",
            Self::Share => "[data-cell=share]",
            Self::More => "[data-cell=more]",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Panel {
    Space,
    Share,
    Overflow,
}

impl Panel {
    fn name(self) -> &'static str {
        match self {
            Self::Space => "space",
            Self::Share => "share",
            Self::Overflow => "overflow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenPanel {
    pub panel: Panel,
    pub anchor: Cell,
    pub return_to: Option<Panel>,
}

/// Which cell's stack is open, and the live rename if one is running.
#[derive(Default)]
pub(crate) struct BarState {
    pub open_panel: Option<OpenPanel>,
    pub layout: Option<BarLayout>,
    pub usable_width_px: f64,
    pub collapsed: bool,
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
            toggle_mode(&host);
        }));
    }

    for (cell, panel) in [(Cell::Space, Panel::Space), (Cell::Share, Panel::Share)] {
        let Ok(Some(button)) = root.query_selector(cell.selector()) else {
            continue;
        };
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::on_click(&button, move || {
            // A click on the space cell mid-rename is aimed at the text:
            // commit it, do not also open the stack over what was typed.
            if cell == Cell::Space && shared.borrow().editing {
                commit_edit(&host, &shared);
                return;
            }
            open_panel(&host, &shared, panel, cell, None);
            let detail = Object::new();
            let _ = Reflect::set(&detail, &"cell".into(), &panel.name().into());
            shadow::emit(&host, "fabb-cell", &detail);
        }));
    }

    if let Ok(Some(button)) = root.query_selector(Cell::More.selector()) {
        let host = this.clone();
        let shared = state.clone();
        listeners.push(shadow::on_click(&button, move || {
            open_panel(&host, &shared, Panel::Overflow, Cell::More, None);
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
            if row.has_attribute("data-overflow-share") {
                open_panel(
                    &host,
                    &shared,
                    Panel::Share,
                    Cell::More,
                    Some(Panel::Overflow),
                );
                return;
            }
            if row.has_attribute("data-mi-back") {
                open_panel(&host, &shared, Panel::Overflow, Cell::More, None);
                return;
            }
            if row.has_attribute("data-overflow-mode") {
                toggle_mode(&host);
                close(&host, &shared);
                return;
            }
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
            if shared.borrow().open_panel.is_none() {
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
/// The whole run mirrors, cells included, so the arrangement relative to the
/// sync disc is the same at either edge.
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
    let (Some(bar), Some(run), Some(fab), Some(space), Some(share), Some(more), Some(toggle)) = (
        query(this, ".bar"),
        query(this, ".run"),
        query(this, ".fab"),
        query(this, ".space"),
        query(this, ".share"),
        query(this, ".more"),
        query(this, "[data-cell=toggle]"),
    ) else {
        return;
    };
    let flipped = this.has_attribute("flip");
    // Appending a node that is already a child MOVES it, so laying the run
    // out in order is enough to reorder it — no removal pass needed.
    let cells: [&Element; 4] = if flipped {
        // [more] · mode · share · space | sync
        [&more, &toggle, &share, &space]
    } else {
        // sync | space · share · mode · [more]
        [&space, &share, &toggle, &more]
    };
    for cell in cells {
        let _ = run.append_child(cell);
    }
    if flipped {
        let _ = bar.append_child(&fab);
    } else {
        let _ = bar.insert_before(&fab, bar.first_child().as_ref());
    }
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper.class_list().toggle_with_force("flip", flipped);
    }
    update_more_glyph(this);
}

/// Compatibility entrypoint for the imperative `open(cell)` surface.
pub(crate) fn open(this: &HtmlElement, state: &Shared, cell: &str) {
    match cell {
        "space" => open_panel(this, state, Panel::Space, Cell::Space, None),
        "share" => {
            let anchor = if state
                .borrow()
                .layout
                .is_some_and(|layout| layout.show_share)
            {
                Cell::Share
            } else {
                Cell::More
            };
            let return_to = (anchor == Cell::More).then_some(Panel::Overflow);
            open_panel(this, state, Panel::Share, anchor, return_to);
        }
        _ => {}
    }
}

/// Open one canonical stack from the cell that currently exposes it.
pub(crate) fn open_panel(
    this: &HtmlElement,
    state: &Shared,
    panel: Panel,
    anchor: Cell,
    return_to: Option<Panel>,
) {
    commit_edit(this, state);
    let requested = OpenPanel {
        panel,
        anchor,
        return_to,
    };
    if state.borrow().open_panel == Some(requested) {
        close(this, state);
        return;
    }
    let preserve_anchor = state
        .borrow()
        .open_panel
        .is_some_and(|open| open.anchor == anchor);
    close_internal(this, state, false);

    let Ok(Some(stack)) = this.query_selector(&format!("tonk-menu[data-for=\"{}\"]", panel.name()))
    else {
        return;
    };
    state.borrow_mut().open_panel = Some(requested);

    if let Ok(Some(back)) = this.query_selector("[data-mi-back]") {
        if return_to == Some(Panel::Overflow) {
            let _ = back.remove_attribute("hidden");
        } else {
            let _ = back.set_attribute("hidden", "");
        }
    }
    if let Ok(Some(overflow_share)) = this.query_selector("[data-overflow-share]") {
        let share_visible = state
            .borrow()
            .layout
            .is_some_and(|layout| layout.show_share);
        if share_visible {
            let _ = overflow_share.set_attribute("hidden", "");
        } else {
            let _ = overflow_share.remove_attribute("hidden");
        }
    }

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
        query(this, anchor.selector()),
        query(this, ".bar"),
        query(this, ".mw"),
    ) else {
        return;
    };
    let rung: HtmlElement = rung.unchecked_into();
    let menus_style = menus.unchecked_ref::<HtmlElement>().style();
    let stack_style = stack.unchecked_ref::<HtmlElement>().style();

    let usable = state.borrow().usable_width_px.max(0.0);
    let width = if panel == Panel::Overflow || anchor == Cell::More {
        usable.min(logic::SPACE_CELL_PX)
    } else if panel == Panel::Share {
        logic::SHARE_CELL_PX
    } else {
        rung.offset_width() as f64
    };
    let _ = stack_style.set_property("--fabb-menu-w", &format!("{width}px"));
    if let Some(viewport_height) = window()
        .and_then(|window| window.inner_height().ok())
        .and_then(|height| height.as_f64())
    {
        let rect = bar.get_bounding_client_rect();
        let max_height = available_menu_height(
            viewport_height,
            rect.top(),
            rect.bottom(),
            this.has_attribute("up"),
        );
        let _ = stack_style.set_property("--fabb-menu-max-h", &format!("{max_height}px"));
    }

    if !preserve_anchor {
        let bar_width = bar.unchecked_ref::<HtmlElement>().offset_width();
        let align_right = match anchor {
            Cell::Space => this.has_attribute("flip"),
            Cell::Share | Cell::More => !this.has_attribute("flip"),
            Cell::Sync => false,
        };
        if align_right {
            let _ = menus_style.set_property("left", "auto");
            let right = bar_width - rung.offset_left() - rung.offset_width();
            let _ = menus_style.set_property("right", &format!("{right}px"));
        } else {
            let _ = menus_style.set_property("right", "auto");
            let _ = menus_style.set_property("left", &format!("{}px", rung.offset_left()));
        }
    }
    let _ = menus.class_list().add_1("on");

    // Cut the underlay's mask now, not on the observer's schedule: a stack
    // that paints one frame unmasked shows glass across its 7px gaps.
    menu::recut_mask(stack.unchecked_ref());
    sync_expanded(this, state);
    focus_first_row(&stack);
}

fn available_menu_height(
    viewport_height: f64,
    bar_top: f64,
    bar_bottom: f64,
    opens_up: bool,
) -> f64 {
    let available = if opens_up {
        bar_top
    } else {
        viewport_height - bar_bottom
    };
    (available - f64::from(markup::STACK_GAP_PX) - MENU_VIEWPORT_MARGIN_PX)
        .max(0.0)
        .floor()
}

/// Close whatever stack is open, restoring any hoisted sub-stack first.
pub(crate) fn close(this: &HtmlElement, state: &Shared) {
    close_internal(this, state, true);
}

fn close_internal(this: &HtmlElement, state: &Shared, restore_focus: bool) {
    let opener = state.borrow().open_panel.map(|open| open.anchor);
    // A row holding its flyout open closes with the stack it lives in.
    // Left set, it would still be open the next time the stack is
    // raised — and a press outside dismisses the stack, so the flyout
    // has to go with it.
    if let Ok(open_rows) = this.query_selector_all("tonk-mi[open]") {
        for index in 0..open_rows.length() {
            if let Some(node) = open_rows.item(index)
                && let Ok(row) = node.dyn_into::<Element>()
            {
                let _ = row.remove_attribute("open");
            }
        }
    }
    restore_sub(state);
    state.borrow_mut().open_panel = None;
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
    if restore_focus
        && let Some(opener) = opener
        && let Some(button) = query(this, opener.selector())
        && !button.has_attribute("hidden")
        && !state.borrow().collapsed
    {
        let _ = button.unchecked_ref::<HtmlElement>().focus();
    }
}

fn focus_first_row(stack: &Element) {
    let Ok(Some(item)) = stack.query_selector(":scope > tonk-mi:not([hidden])") else {
        return;
    };
    let Some(row) = item
        .shadow_root()
        .and_then(|root| root.query_selector(".row").ok().flatten())
    else {
        return;
    };
    let _ = row.unchecked_ref::<HtmlElement>().focus();
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
    let open = state.borrow().open_panel;
    for cell in [Cell::Space, Cell::Share, Cell::More] {
        if let Some(button) = query(this, cell.selector()) {
            let _ = button.set_attribute(
                "aria-expanded",
                &(open.is_some_and(|panel| panel.anchor == cell)).to_string(),
            );
        }
    }
    update_more_glyph(this);
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

fn toggle_mode(this: &HtmlElement) {
    let dark = !shadow::is_dark(this);
    let next = if dark { "dark" } else { "light" };
    shadow::set_mode(this, Some(next));
    shadow::apply_mode(this);
    tonk_host::theme::set_mode(dark);
    propagate(this);
    update(this);
    let detail = Object::new();
    let _ = Reflect::set(&detail, &"mode".into(), &next.into());
    shadow::emit(this, "fabb-mode", &detail);
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
            if wrapper(this).is_some_and(|wrapper| wrapper.class_list().contains("compact")) {
                let _ = element.set_attribute("compact", "");
            } else {
                let _ = element.remove_attribute("compact");
            }
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
        let collapsed =
            wrapper(this).is_some_and(|wrapper| wrapper.class_list().contains("collapsed"));
        let label = if collapsed {
            format!("expand FABB · sync: {reported} · drag to move")
        } else {
            format!("sync: {reported}{suffix} · drag to move")
        };
        let _ = fab.set_attribute("aria-label", &label);
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
    if let Ok(Some(label)) = this.query_selector("[data-mode-label]") {
        label.set_text_content(Some(if shadow::is_dark(this) {
            "light mode"
        } else {
            "dark mode"
        }));
    }
    if let Ok(Some(mode)) = this.query_selector("[data-overflow-mode]") {
        let _ = mode.set_attribute("pressed", &shadow::is_dark(this).to_string());
    }
    update_more_glyph(this);
    update_sync_condition(this);
}

fn update_sync_condition(this: &HtmlElement) {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    let local_only = this.get_attribute("data-sync-status").as_deref() == Some("sync:local")
        && this.get_attribute("data-customer-status").as_deref() != Some("Registered");
    if !local_only {
        if let Some(cluster) = document.get_element_by_id("fabb-connect-cluster") {
            let _ = cluster.set_attribute("hidden", "");
        }
        if let Some(banner) = document.get_element_by_id(CONNECT_BANNER_ID) {
            retire_banner(&banner);
        }
        return;
    }
    if document.get_element_by_id(CONNECT_BANNER_ID).is_some() {
        sync_condition_mode(this, &document);
        return;
    }
    let Ok(banner) = document.create_element("tonk-banner") else {
        return;
    };
    banner.set_id(CONNECT_BANNER_ID);
    banner.set_inner_html("connect this space<span slot=\"door\">connect</span>");
    crate::shadow::set_mode(&banner, this.get_attribute("mode").as_deref());
    let on_open = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        crate::share::open_enable_sync_from_banner();
    });
    let _ = banner.add_event_listener_with_callback("fabb-open", on_open.as_ref().unchecked_ref());
    on_open.forget();
    if let Some(body) = document.body() {
        let _ = body.append_child(&banner);
    }
}

fn sync_condition_mode(this: &HtmlElement, document: &web_sys::Document) {
    for id in [CONNECT_BANNER_ID, "fabb-connect-cluster"] {
        let Some(element) = document.get_element_by_id(id) else {
            continue;
        };
        crate::shadow::set_mode(&element, this.get_attribute("mode").as_deref());
    }
}

pub(crate) fn refresh_sync_condition(this: &HtmlElement) {
    update_sync_condition(this);
}

fn retire_banner(banner: &Element) {
    let retire = Reflect::get(banner, &"retire".into())
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok());
    if let Some(retire) = retire {
        let _ = retire.call0(banner);
    } else {
        banner.remove();
    }
}

pub(crate) fn remove_conditions() {
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(banner) = document.get_element_by_id(CONNECT_BANNER_ID) {
        banner.remove();
    }
}

fn update_more_glyph(this: &HtmlElement) {
    if let Some(glyph) = query(this, ".more-glyph") {
        glyph.set_text_content(Some(if this.has_attribute("up") {
            "\u{25B4}"
        } else {
            "\u{25BE}"
        }));
    }
}

/// Apply the one fit-driven action partition to DOM, focus, and menus.
pub(crate) fn apply_responsive(this: &HtmlElement, usable_width_px: f64, state: &Shared) {
    let layout = logic::bar_layout(usable_width_px);
    if state.borrow().layout == Some(layout) {
        return;
    }

    let stale_opener = state
        .borrow()
        .open_panel
        .is_some_and(|open| match open.anchor {
            Cell::Share => !layout.show_share,
            Cell::More => !layout.show_overflow,
            Cell::Sync | Cell::Space => false,
        });
    if stale_opener {
        close_internal(this, state, false);
    }

    {
        let mut current = state.borrow_mut();
        current.layout = Some(layout);
        current.usable_width_px = usable_width_px.max(0.0);
    }

    let Some(wrapper) = wrapper(this) else { return };
    let classes = wrapper.class_list();
    let _ = classes.toggle_with_force("compact", layout.compact);
    let _ = classes.toggle_with_force("hide-share", !layout.show_share);
    let collapsed = state.borrow().collapsed;
    let _ = classes.toggle_with_force("collapsed", collapsed);
    let _ = wrapper
        .unchecked_ref::<HtmlElement>()
        .style()
        .set_property("--_space-w", &format!("{}px", layout.space_width_px));

    set_cell_visible(this, Cell::Space, true, collapsed);
    set_cell_visible(this, Cell::Share, layout.show_share, collapsed);
    set_cell_visible(this, Cell::More, layout.show_overflow, collapsed);
    if let Some(mode) = query(this, "[data-cell=toggle]") {
        set_element_visible(&mode, layout.show_mode, collapsed);
    }
    if let Some(run) = query(this, ".run") {
        if collapsed {
            let _ = run.set_attribute("aria-hidden", "true");
        } else {
            let _ = run.remove_attribute("aria-hidden");
        }
    }
    if let Ok(Some(overflow_share)) = this.query_selector("[data-overflow-share]") {
        set_element_visible(&overflow_share, !layout.show_share, false);
    }

    apply_flip(this);
    propagate(this);
    sync_expanded(this, state);
    update(this);
    if stale_opener && let Some(space) = query(this, Cell::Space.selector()) {
        let _ = space.unchecked_ref::<HtmlElement>().focus();
    }
}

fn set_cell_visible(this: &HtmlElement, cell: Cell, visible: bool, collapsed: bool) {
    if let Some(element) = query(this, cell.selector()) {
        set_element_visible(&element, visible, collapsed);
    }
}

fn set_element_visible(element: &Element, visible: bool, collapsed: bool) {
    if visible {
        let _ = element.remove_attribute("hidden");
    } else {
        let _ = element.set_attribute("hidden", "");
    }
    if visible && !collapsed {
        let _ = element.remove_attribute("tabindex");
    } else {
        let _ = element.set_attribute("tabindex", "-1");
    }
}

pub(crate) fn collapse(this: &HtmlElement, state: &Shared) {
    commit_edit(this, state);
    close_internal(this, state, false);
    state.borrow_mut().collapsed = true;
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper.class_list().add_1("collapsed");
    }
    if let Some(run) = query(this, ".run") {
        let _ = run.set_attribute("aria-hidden", "true");
    }
    for selector in [
        Cell::Space.selector(),
        Cell::Share.selector(),
        Cell::More.selector(),
        "[data-cell=toggle]",
    ] {
        if let Some(element) = query(this, selector) {
            let _ = element.set_attribute("tabindex", "-1");
        }
    }
    if let Some(sync) = query(this, Cell::Sync.selector()) {
        let _ = sync.unchecked_ref::<HtmlElement>().focus();
    }
    update(this);
}

pub(crate) fn expand(this: &HtmlElement, state: &Shared) {
    if !state.borrow().collapsed {
        return;
    }
    state.borrow_mut().collapsed = false;
    if let Some(wrapper) = wrapper(this) {
        let _ = wrapper.class_list().remove_1("collapsed");
    }
    if let Some(run) = query(this, ".run") {
        let _ = run.remove_attribute("aria-hidden");
    }
    if let Some(layout) = state.borrow().layout {
        set_cell_visible(this, Cell::Space, true, false);
        set_cell_visible(this, Cell::Share, layout.show_share, false);
        set_cell_visible(this, Cell::More, layout.show_overflow, false);
        if let Some(mode) = query(this, "[data-cell=toggle]") {
            set_element_visible(&mode, layout.show_mode, false);
        }
    }
    update(this);
}

#[cfg(test)]
mod tests {
    use super::available_menu_height;

    #[test]
    fn it_caps_menus_to_the_space_on_their_opening_side() {
        assert_eq!(available_menu_height(900.0, 820.0, 856.0, true), 805.0);
        assert_eq!(available_menu_height(900.0, 40.0, 76.0, false), 809.0);
        assert_eq!(available_menu_height(100.0, 4.0, 96.0, true), 0.0);
    }
}
