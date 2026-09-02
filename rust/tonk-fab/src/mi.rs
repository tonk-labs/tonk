//! `<tonk-mi>` — one block in a stack.
//!
//! 36px, label bottom-right, no surface of its own: the parent
//! [`crate::menu`] wears the glass once for the whole stack and masks it to
//! the rows, so a `tonk-mi` paints only its ring, its washes and — when
//! `current` — its solid ink.
//!
//! A nested `<tonk-menu slot="sub">` flies out one 7px gap to the right on
//! hover or focus, flipping left when the right edge would clip it. Sideways
//! flight is a hover-pointer's move: on coarse pointers the bar intercepts
//! the pick and discloses the sub-stack in place instead (see
//! `bar::open_sub`), because a flyout needs room a finger does not imply.
//!
//! Attributes: `muted` `chrome` `tall` `current` `cap=left|right` `label`.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Object, Reflect};
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, window};

use crate::shadow::{self, Bound};

/// The gap between a row and its flyout — one stack gap, so the flyout reads
/// as a sibling stack rather than a nested panel.
const FLYOUT_GAP_PX: f64 = 7.0;

/// Breathing room kept between a flyout and the edge that would clip it.
const CLIP_MARGIN_PX: f64 = 8.0;

/// The width a flyout is assumed to need before it has been measured.
const DEFAULT_MENU_WIDTH_PX: f64 = 216.0;

const CSS: &str = r#"
:host{ display:block; position:relative; }
:host([hidden]){ display:none !important; }
.row{ width:100%; min-height:var(--_mi-min-height, 36px); display:flex; align-items:flex-end; justify-content:flex-end;
  gap:8px; padding:0 10px 9px 22px;
  font-size:13px; line-height:1; font-weight:500; color:var(--_ink);
  background:transparent; /* the stack's underlay wears the glass */
  box-shadow:var(--_ring); }
.row:hover{ background:var(--_hover); }
.row:active{ background:var(--_press); }
/* capped rows keep their own frost — the underlay is rectangular and cannot
   follow the 18px radii (dialog rails only, and the mask skips them so no
   square glass shows behind the curve) */
:host([cap]) .row{ background:var(--_bg);
  -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter); }
:host([cap]) .row:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_bg); }
:host([cap]) .row:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_bg); }
:host([chrome]) .row{ text-transform:lowercase; }
:host([muted]) .row{ color:var(--_soft); }
/* current wears near-ink — the CTA register keeps solid ink */
:host([current]) .row{ background:var(--_cur); color:var(--_on); font-weight:600; }
:host([cap=left]) .row{ border-radius:18px 0 0 18px; }
:host([cap=right]) .row{ border-radius:0 18px 18px 0; }
:host([tall]) .row{ min-height:56px; flex-direction:column; align-items:flex-end;
  justify-content:flex-end; gap:4px; padding-top:10px; }
/* two type levels only: the label (13/500 ink) and meta (11/400 soft) */
::slotted(.sub), ::slotted(.when){ font-size:11px; font-weight:400; color:var(--_soft); line-height:1.2; }
/* glyphs take the ink tokens explicitly — document styles beat ::slotted(),
   so a slotted mark that inherits would be repainted by the host page */
::slotted(.g){ font-weight:500; color:var(--_ink); }
:host([muted]) ::slotted(.g){ color:var(--_soft); }
:host([current]) ::slotted(.g), :host([current]) ::slotted(.sub), :host([current]) ::slotted(.when){ color:var(--_on); }
/* the flyout — a stack one gap to the right; flips left when clipped, and
   grows UP instead of down when there is no room below (a bar docked at the
   bottom opens its stack upward, so its flyout has to follow) */
.fly{ display:none; position:absolute; left:calc(100% + 7px); top:0; z-index:6; }
.fly.flip{ left:auto; right:calc(100% + 7px); }
.fly.up{ top:auto; bottom:0; }
/* a connected flyout bridges its gap — the parent row's surface spans the
   7px so the pair reads as one piece, not neighbours */
.fly.sub::before{ content:""; position:absolute; top:-1px; left:-8px; width:9px; height:38px;
  background:var(--_bg); -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  border-top:1px solid var(--_ringc); border-bottom:1px solid var(--_ringc); }
.fly.flip.sub::before{ left:auto; right:-8px; }
/* the bridge joins the row it came from, so it moves to the other end too */
.fly.up.sub::before{ top:auto; bottom:-1px; }
@media (hover:hover) and (pointer:fine){
  :host(:hover) .fly, :host(:focus-within) .fly{ display:block; }
}
/* Picked open. Hover is the pointer's way in, but a row that is taken --
   by click, by keyboard, by anything that is not a hovering mouse -- has
   to be able to open its flyout too, and nothing else did. */
:host([open]) .fly{ display:block; }
"#;

const HTML: &str = r#"<div class="w" style="display:contents">
  <button class="row" part="row"><slot></slot></button>
  <div class="fly"><slot name="sub"></slot></div>
</div>"#;

/// Per-element state — listeners kept alive for the element's lifetime.
#[derive(Default)]
pub(crate) struct TonkMi {
    listeners: Vec<Bound>,
    /// Retained so repeated `slotchange` wiring is not re-installed.
    wired: Rc<RefCell<bool>>,
}

impl CustomElement for TonkMi {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["muted", "chrome", "tall", "current", "cap", "pressed"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        if *self.wired.borrow() {
            return;
        }
        *self.wired.borrow_mut() = true;

        let root = shadow::build(this, CSS, HTML);

        // Picking a row is the stack's only verb. The bar listens for this
        // to preview, disclose in place, or close.
        if let Ok(Some(row)) = root.query_selector(".row") {
            let host = this.clone();
            self.listeners.push(shadow::on_click(&row, move || {
                // A row that carries a sub-stack opens it when picked.
                // Hover reveals it for a mouse, and that was the only way
                // in — so a click, a tap, or Enter did nothing at all.
                toggle_open(&host);
                let detail = Object::new();
                let _ = Reflect::set(&detail, &"item".into(), &host);
                shadow::emit(&host, "fabb-pick", &detail);
            }));
        }
        sync_pressed(this);

        // `.fly.sub` only applies when something is actually slotted —
        // without the check the bridge would paint across an empty gap on
        // every leaf row.
        if let Ok(Some(sub_slot)) = root.query_selector("slot[name=sub]") {
            let host = this.clone();
            self.listeners
                .push(shadow::bind(&sub_slot, "slotchange", move |_| {
                    sync_sub(&host)
                }));
        }
        sync_sub(this);

        // Aim on approach rather than on a resize observer: the decision
        // depends on where the row is at the moment it opens, and a row that
        // is never hovered never needs one.
        for event in ["pointerenter", "focusin"] {
            let host = this.clone();
            self.listeners
                .push(shadow::bind(this, event, move |_| aim_flyout(&host)));
        }

        self.listeners.push(shadow::install_visibility_pause(this));
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.listeners.clear();
        *self.wired.borrow_mut() = false;
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        if name == "pressed" {
            sync_pressed(this);
        }
    }
}

fn sync_pressed(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };
    let Ok(Some(row)) = root.query_selector(".row") else {
        return;
    };
    match this.get_attribute("pressed") {
        Some(value) => {
            let _ = row.set_attribute(
                "aria-pressed",
                if value == "true" { "true" } else { "false" },
            );
        }
        None => {
            let _ = row.remove_attribute("aria-pressed");
        }
    }
}

/// Un-hide a slotted sub-stack: a stack is hidden while it is a menu the
/// bar has closed, but as a `sub` it is governed by the flyout's own
/// `display`.
fn unhide_subs(this: &HtmlElement) {
    let Ok(subs) = this.query_selector_all("tonk-menu[slot=sub]") else {
        return;
    };
    for index in 0..subs.length() {
        let Some(node) = subs.item(index) else {
            continue;
        };
        let Ok(element) = node.dyn_into::<Element>() else {
            continue;
        };
        let _ = element.remove_attribute("hidden");
    }
}

/// Open this row's flyout, and close any sibling that was open.
///
/// Only rows that actually carry a sub-stack take the state: on a leaf
/// row there is nothing to show, and marking it open would leave the
/// attribute lying around for CSS that reads it.
fn toggle_open(this: &HtmlElement) {
    if this
        .query_selector("tonk-menu[slot=sub]")
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    let opening = !this.has_attribute("open");
    // One at a time: opening a second flyout while the first stands
    // leaves two stacks overlapping the same column.
    if let Some(parent) = this.parent_element()
        && let Ok(siblings) = parent.query_selector_all("tonk-mi[open]")
    {
        for index in 0..siblings.length() {
            if let Some(node) = siblings.item(index)
                && let Ok(sibling) = node.dyn_into::<Element>()
            {
                let _ = sibling.remove_attribute("open");
            }
        }
    }
    if opening {
        let _ = this.set_attribute("open", "");
        // Aim it too. Aiming runs on `pointerenter`/`focusin` because
        // that is when a hovered row opens — a row opened by a click
        // gets neither, and an unaimed flyout renders to the right of a
        // bar that is already at the screen edge, so it is on the page
        // and off the screen.
        aim_flyout(this);
    }
}

/// Toggle the `.sub` bridge according to whether anything is slotted.
fn sync_sub(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };
    let Ok(Some(fly)) = root.query_selector(".fly") else {
        return;
    };
    // Ask the light tree rather than the slot: `assignedElements` is behind
    // web-sys's unstable gate, and what the bridge actually depends on is a
    // sub-stack existing as a child, which is exactly this query.
    let assigned = matches!(this.query_selector("tonk-menu[slot=sub]"), Ok(Some(_)));
    let _ = fly.class_list().toggle_with_force("sub", assigned);
    // The un-hide used to ride the mode stamp; with the mode plumbing gone
    // this is its home — the same signals (connect, slotchange) cover it.
    unhide_subs(this);
}

/// Choose the side the flyout opens toward.
///
/// The boundary that matters is the nearest overflow-clipping ancestor — a
/// stage, a panel — not the viewport, because that is what actually cuts the
/// flyout off. Prefer the side that fits; if neither does, take the roomier.
fn aim_flyout(this: &HtmlElement) {
    let Some(root) = this.shadow_root() else {
        return;
    };
    let Ok(Some(fly)) = root.query_selector(".fly") else {
        return;
    };
    let Ok(Some(sub)) = this.query_selector("tonk-menu[slot=sub]") else {
        return;
    };

    // Measure the sub-stack while it is laid out. It is `display:none` until
    // hover, and a hidden element measures zero.
    let style = fly.unchecked_ref::<HtmlElement>().style();
    let _ = style.set_property("display", "block");
    let measured = sub.unchecked_ref::<HtmlElement>();
    let width = measured.offset_width() as f64;
    let height = measured.offset_height() as f64;
    let _ = style.remove_property("display");
    let width = if width > 0.0 {
        width
    } else {
        DEFAULT_MENU_WIDTH_PX
    };

    let Some(win) = window() else { return };
    let rect = this.get_bounding_client_rect();
    let viewport = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let mut clip_left = CLIP_MARGIN_PX;
    let mut clip_right = viewport - CLIP_MARGIN_PX;

    let mut ancestor = this.parent_element();
    while let Some(element) = ancestor {
        if let Ok(cs) = win.get_computed_style(&element)
            && let Some(cs) = cs
        {
            let overflow = cs.get_property_value("overflow").unwrap_or_default();
            let overflow_x = cs.get_property_value("overflow-x").unwrap_or_default();
            let combined = format!("{overflow}{overflow_x}");
            if ["hidden", "auto", "scroll", "clip"]
                .iter()
                .any(|kind| combined.contains(kind))
            {
                let bounds = element.get_bounding_client_rect();
                clip_left = bounds.left() + CLIP_MARGIN_PX;
                clip_right = bounds.right() - CLIP_MARGIN_PX;
                break;
            }
        }
        ancestor = element.parent_element();
    }

    let fits_right = rect.right() + FLYOUT_GAP_PX + width <= clip_right;
    let fits_left = rect.left() - FLYOUT_GAP_PX - width >= clip_left;
    let roomier_left = (rect.left() - clip_left) > (clip_right - rect.right());
    let flip = !fits_right && (fits_left || roomier_left);
    let _ = fly.class_list().toggle_with_force("flip", flip);

    // Vertically the flyout hangs from the row's top by default. That runs
    // off the bottom of the screen for a bar docked low — whose own stack
    // already opens upward — so when the list does not fit below, anchor its
    // BOTTOM to the row instead and let it grow up. Decided from measured
    // height rather than from the bar's `up` attribute, so an unusually long
    // list near the bottom is handled the same way.
    let viewport_bottom = win
        .inner_height()
        .ok()
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0)
        - CLIP_MARGIN_PX;
    let fits_below = rect.top() + height <= viewport_bottom;
    let fits_above = rect.bottom() - height >= CLIP_MARGIN_PX;
    let up = !fits_below && fits_above;
    let _ = fly.class_list().toggle_with_force("up", up);
}

/// Register `<tonk-mi>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else { return };
    if win.custom_elements().get("tonk-mi").is_undefined() {
        TonkMi::define("tonk-mi");
    }
}
