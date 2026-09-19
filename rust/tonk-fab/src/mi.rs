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
//! ## Hover is forgiving
//!
//! The gaps a stack is built from are pure page (law 2), so hover alone made
//! the flyout twitchy: a pointer that clipped the 7px above a row lost the
//! row, and the stack it was reaching for vanished mid-approach. Two answers,
//! both here:
//!
//! * a row's hit area reaches half a gap past its box, so the gaps between
//!   rows are live ground rather than a trap; and
//! * a flyout opened by hover keeps standing for [`FLYOUT_GRACE_MS`] after
//!   the pointer leaves ([`hold`]/[`release`] on the `hot` attribute), so a
//!   pointer that strays and comes back finds it where it left it.
//!
//! Attributes: `muted` `chrome` `tall` `current` `cap=left|right` `label`.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use web_sys::{Element, HtmlElement, window};

use crate::shadow::{self, Bound};

/// The gap between a row and its flyout — one stack gap, so the flyout reads
/// as a sibling stack rather than a nested panel.
const FLYOUT_GAP_PX: f64 = 7.0;

/// Breathing room kept between a flyout and the edge that would clip it.
const CLIP_MARGIN_PX: f64 = 8.0;

/// The width a flyout is assumed to need before it has been measured.
const DEFAULT_MENU_WIDTH_PX: f64 = 216.0;

/// How long a hover-opened flyout stands after the pointer leaves the row.
///
/// Long enough to cross a gap, a corner or a slip of the wrist and come
/// back; short enough that a flyout the pointer has genuinely abandoned is
/// gone before it is in the way.
const FLYOUT_GRACE_MS: i32 = 320;

const CSS: &str = r#"
:host{ display:block; position:relative; }
:host([hidden]){ display:none !important; }
.row{ position:relative; width:100%; min-height:var(--_mi-min-height, 36px); display:flex; align-items:flex-end; justify-content:flex-end;
  gap:8px; padding:0 10px 9px 22px;
  font-size:13px; line-height:1; font-weight:500; color:var(--_ink);
  background:transparent; /* the stack's underlay wears the glass */
  box-shadow:var(--_ring); }
.row:hover{ background:var(--_hover); }
.row:active{ background:var(--_press); }
/* The 7px between rows is pure page (law 2), which also made it dead
   ground: a pointer that clipped it dropped the row and took the flyout the
   row was holding open with it. Two transparent strips let a row answer the
   half-gap above and below it, so the whole column is live and the gaps
   still read as page. They sit OUTSIDE the row's border box on purpose —
   over it they would cover the label, and a renaming row's label is a caret
   target. The stack's own ends keep theirs to themselves: past them lies
   the bar or the page, and neither wants a row reaching into it. */
.row::before, .row::after{ content:""; position:absolute; left:0; right:0;
  height:var(--_mi-bridge, 4px); }
.row::before{ top:calc(-1 * var(--_mi-bridge, 4px)); }
.row::after{ bottom:calc(-1 * var(--_mi-bridge, 4px)); }
:host(:first-child) .row::before, :host([cap]) .row::before{ display:none; }
:host(:last-child) .row::after, :host([cap]) .row::after{ display:none; }
/* capped rows keep their own frost — the underlay is rectangular and cannot
   follow the 18px radii (dialog rails only, and the mask skips them so no
   square glass shows behind the curve) */
:host([cap]) .row{ background:var(--_bg);
  -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter); }
:host([cap]) .row:hover{ background:linear-gradient(var(--_hover),var(--_hover)), var(--_bg); }
:host([cap]) .row:active{ background:linear-gradient(var(--_press),var(--_press)), var(--_bg); }
:host([cap]) .row:focus-visible{ background:linear-gradient(var(--_press),var(--_press)), var(--_bg); }
:host([chrome]) .row{ text-transform:lowercase; }
:host([muted]) .row{ color:var(--_soft); }
/* current wears near-ink — the CTA register keeps solid ink */
:host([current]) .row{ background:var(--_cur); color:var(--_on); font-weight:600; }
:host([current]) .row:focus-visible{
  background:linear-gradient(var(--_wash-on),var(--_wash-on)), var(--_cur); }
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
/* Keep the visual gap as pure page while making it safe to cross. The 9px
   corridor spans the 7px gap and overlaps each adjacent surface by 1px. */
.fly::before{ content:""; position:absolute; top:0; bottom:0; left:-8px; width:9px;
  background:transparent; border:0; -webkit-backdrop-filter:none; backdrop-filter:none;
  pointer-events:auto; }
.fly.flip::before{ left:auto; right:-8px; }
/* `hot` is hover with a grace period: set on `pointerenter`, dropped a beat
   after `pointerleave` (see `release`). Hover itself still opens the flyout
   on the spot, so the opening never waits on a listener — `hot` only
   governs how it goes away. */
@media (hover:hover) and (pointer:fine){
  :host(:hover) .fly, :host(:focus-within) .fly, :host([hot]) .fly{ display:block; }
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

/// The grace period a hover-opened flyout coasts on after the pointer
/// leaves the row.
///
/// The expiry callback is made once per element and reused: a stack is
/// hovered across dozens of times in a sitting, and a closure leaked per
/// pass would be a slow drip for no reason.
#[derive(Clone)]
struct Grace {
    /// The timer still to fire, so re-entering the row can cancel it.
    pending: Rc<RefCell<Option<i32>>>,
    /// Drops `hot` when it fires.
    expire: Rc<Closure<dyn FnMut()>>,
}

impl Grace {
    fn new(this: &HtmlElement) -> Self {
        let host = this.clone();
        let pending = Rc::new(RefCell::new(None));
        let slot = pending.clone();
        let expire = Closure::<dyn FnMut()>::new(move || {
            *slot.borrow_mut() = None;
            let _ = host.remove_attribute("hot");
        });
        Self {
            pending,
            expire: Rc::new(expire),
        }
    }

    /// Cancel the pending expiry, if any.
    fn cancel(&self) {
        let Some(id) = self.pending.borrow_mut().take() else {
            return;
        };
        if let Some(win) = window() {
            win.clear_timeout_with_handle(id);
        }
    }

    /// Start the countdown, replacing any already running.
    fn start(&self) {
        self.cancel();
        let Some(win) = window() else { return };
        if let Ok(id) = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            self.expire.as_ref().as_ref().unchecked_ref(),
            FLYOUT_GRACE_MS,
        ) {
            *self.pending.borrow_mut() = Some(id);
        }
    }
}

/// Per-element state — listeners kept alive for the element's lifetime.
#[derive(Default)]
pub(crate) struct TonkMi {
    listeners: Vec<Bound>,
    /// Retained so repeated `slotchange` wiring is not re-installed.
    wired: Rc<RefCell<bool>>,
    /// Built on connect, so it can hold this element's host and timer.
    grace: Option<Grace>,
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

        // A sub-stack is hidden while it is a menu the bar has closed. Once
        // slotted here, the flyout owns its visibility instead.
        if let Ok(Some(sub_slot)) = root.query_selector("slot[name=sub]") {
            let host = this.clone();
            self.listeners
                .push(shadow::bind(&sub_slot, "slotchange", move |_| {
                    unhide_subs(&host)
                }));
        }
        unhide_subs(this);

        // Aim on approach rather than on a resize observer: the decision
        // depends on where the row is at the moment it opens, and a row that
        // is never hovered never needs one.
        for event in ["pointerenter", "focusin"] {
            let host = this.clone();
            self.listeners
                .push(shadow::bind(this, event, move |_| aim_flyout(&host)));
        }

        // Hover, held. `pointerenter` takes `hot`; `pointerleave` only starts
        // the countdown to dropping it. A flyout is a descendant of the row,
        // so crossing into one fires no leave at all — the grace is for the
        // pointer that misses, clips a gap, or rounds a corner on its way.
        //
        // Focus is deliberately not wired here: `:focus-within` already
        // shows the flyout and gives it back the moment focus moves, and a
        // `hot` taken on `focusin` would need a matching `focusout` to ever
        // come off.
        let grace = Grace::new(this);
        {
            let host = this.clone();
            let grace = grace.clone();
            self.listeners
                .push(shadow::bind(this, "pointerenter", move |_| {
                    hold(&host, &grace)
                }));
        }
        {
            let host = this.clone();
            let grace = grace.clone();
            self.listeners
                .push(shadow::bind(this, "pointerleave", move |_| {
                    release(&host, &grace)
                }));
        }
        self.grace = Some(grace);

        self.listeners.push(shadow::install_visibility_pause(this));
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        if let Some(grace) = self.grace.take() {
            grace.cancel();
        }
        // A row taken off the page mid-grace must not come back hot: the bar
        // moves stacks between parents to disclose a sub in place, and the
        // row would return with a flyout standing that nothing is pointing at.
        let _ = this.remove_attribute("hot");
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

/// Take `hot`: the pointer is on this row, so its flyout stands until the
/// grace period says otherwise.
///
/// A sibling still coasting on its own grace loses `hot` here rather than
/// when its timer fires — two flyouts standing over the same column is the
/// one thing the grace period could otherwise introduce. Its timer fires
/// later and finds nothing to do.
fn hold(this: &HtmlElement, grace: &Grace) {
    grace.cancel();
    let _ = this.set_attribute("hot", "");
    let Some(parent) = this.parent_element() else {
        return;
    };
    let Ok(siblings) = parent.query_selector_all("tonk-mi[hot]") else {
        return;
    };
    for index in 0..siblings.length() {
        if let Some(node) = siblings.item(index)
            && let Ok(sibling) = node.dyn_into::<Element>()
            && !sibling.is_same_node(Some(this))
        {
            let _ = sibling.remove_attribute("hot");
        }
    }
}

/// Start the countdown to dropping `hot`.
///
/// Nothing is hidden here. The flyout goes when the timer fires, and a
/// pointer that comes back before then cancels it in [`hold`].
fn release(this: &HtmlElement, grace: &Grace) {
    if !this.has_attribute("hot") {
        return;
    }
    grace.start();
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

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{Element, Event, HtmlElement, window};

    wasm_bindgen_test_configure!(run_in_browser);

    /// The stack the tests drive: a row carrying a flyout, then two plain
    /// rows, with the usual 7px between them.
    const STACK: &str = r#"
        <tonk-mi id="opener"><span>open</span>
          <tonk-menu slot="sub">
            <tonk-mi><span>tonk team</span></tonk-mi>
            <tonk-mi><span>Notebook</span></tonk-mi>
          </tonk-menu>
        </tonk-mi>
        <tonk-mi id="second"><span>rename</span></tonk-mi>
        <tonk-mi id="third"><span>settings</span></tonk-mi>"#;

    fn mount() -> HtmlElement {
        super::register();
        crate::menu::register();
        let document = window().expect("window").document().expect("document");
        let stack: HtmlElement = document
            .create_element("tonk-menu")
            .expect("stack")
            .dyn_into()
            .expect("HtmlElement");
        // Fixed and away from the page edges, so a point taken just outside
        // a row is still a point in the viewport.
        stack
            .set_attribute("style", "position:fixed; left:60px; top:60px; width:216px;")
            .expect("place the stack");
        stack.set_inner_html(STACK);
        document
            .body()
            .expect("body")
            .append_child(&stack)
            .expect("append the stack");
        stack
    }

    fn row(stack: &HtmlElement, selector: &str) -> HtmlElement {
        stack
            .query_selector(selector)
            .expect("valid selector")
            .unwrap_or_else(|| panic!("missing {selector}"))
            .dyn_into()
            .expect("HtmlElement")
    }

    fn pointer(target: &HtmlElement, kind: &str) {
        let event = Event::new(kind).expect("pointer event");
        target.dispatch_event(&event).expect("dispatch");
    }

    async fn rest(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            window()
                .expect("window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .expect("set timeout");
        });
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .expect("the timeout resolves");
    }

    /// What the document finds under a point — a hit inside a row's shadow
    /// retargets to the `tonk-mi` host.
    fn hit(x: f64, y: f64) -> Option<Element> {
        window()
            .expect("window")
            .document()
            .expect("document")
            .element_from_point(x as f32, y as f32)
    }

    /// `hot` is asserted rather than the flyout's `display` on purpose: the
    /// flyout is gated behind `(hover:hover) and (pointer:fine)`, which a
    /// headless browser reports as false, and a test that reads `display`
    /// there would quietly pass on a stack that never opens. The CSS half of
    /// the bargain is pinned by
    /// `the_flyout_stands_on_hot_wherever_hover_opens_it`.
    fn is_hot(row: &HtmlElement) -> bool {
        row.has_attribute("hot")
    }

    /// The pointer leaves the row and comes back — across a gap, around a
    /// corner, or by a slip of the wrist. The flyout it was reaching for has
    /// to still be there.
    #[wasm_bindgen_test]
    async fn a_flyout_outlives_a_pointer_that_strays_and_comes_back() {
        let stack = mount();
        let opener = row(&stack, "#opener");

        pointer(&opener, "pointerenter");
        assert!(is_hot(&opener), "a pointer on the row opens its flyout");

        pointer(&opener, "pointerleave");
        assert!(
            is_hot(&opener),
            "leaving the row must not take the flyout with it on the spot"
        );

        pointer(&opener, "pointerenter");
        rest(super::FLYOUT_GRACE_MS + 120).await;
        assert!(
            is_hot(&opener),
            "coming back inside the grace period cancels the expiry"
        );

        pointer(&opener, "pointerleave");
        rest(super::FLYOUT_GRACE_MS + 120).await;
        assert!(
            !is_hot(&opener),
            "a flyout the pointer has genuinely left still goes"
        );

        stack.remove();
    }

    /// One flyout at a time: entering another row ends the neighbour's grace
    /// period there and then, rather than leaving two stacks standing over
    /// the same column.
    #[wasm_bindgen_test]
    async fn entering_a_sibling_ends_the_grace_period_early() {
        let stack = mount();
        let opener = row(&stack, "#opener");
        let second = row(&stack, "#second");

        pointer(&opener, "pointerenter");
        pointer(&opener, "pointerleave");
        pointer(&second, "pointerenter");
        assert!(
            !is_hot(&opener),
            "the neighbour goes the moment another row is entered"
        );

        stack.remove();
    }

    /// A row taken off the page mid-grace must not come back holding a
    /// flyout: the bar moves stacks between parents to disclose a sub-stack
    /// in place.
    #[wasm_bindgen_test]
    async fn a_row_does_not_come_back_hot() {
        let stack = mount();
        let opener = row(&stack, "#opener");

        pointer(&opener, "pointerenter");
        pointer(&opener, "pointerleave");
        stack.remove();
        assert!(!is_hot(&opener), "leaving the page drops the grace period");

        window()
            .expect("window")
            .document()
            .expect("document")
            .body()
            .expect("body")
            .append_child(&stack)
            .expect("re-append the stack");
        rest(super::FLYOUT_GRACE_MS + 120).await;
        assert!(
            !is_hot(&opener),
            "and the expired timer cannot bring it back"
        );

        stack.remove();
    }

    /// The other half of the grace period: what `hot` is worth once the
    /// browser does report a hover pointer.
    #[wasm_bindgen_test]
    fn the_flyout_stands_on_hot_wherever_hover_opens_it() {
        let at = super::CSS
            .find("@media (hover:hover) and (pointer:fine){")
            .expect("the hover gate");
        let block = &super::CSS[at..];
        let block = &block[..block.find('}').expect("a closed block")];
        assert!(
            block.contains(":host([hot]) .fly"),
            "hot must open the flyout on exactly the pointers hover does: {block}"
        );
        assert!(
            block.contains(":host(:hover) .fly"),
            "and hover must still open it directly, without waiting on a listener: {block}"
        );
    }

    /// The 7px between two rows answers to the row it is nearest, so a
    /// pointer crossing it never falls through to the page.
    #[wasm_bindgen_test]
    async fn the_gap_between_two_rows_belongs_to_them() {
        let stack = mount();
        let opener = row(&stack, "#opener");
        let second = row(&stack, "#second");
        let above = opener.get_bounding_client_rect();
        let below = second.get_bounding_client_rect();
        let x = below.left() + below.width() / 2.0;

        let mut y = above.bottom() + 1.0;
        while y < below.top() {
            let under = hit(x, y).expect("something under the gap");
            assert_eq!(
                under.tag_name(),
                "TONK-MI",
                "the gap at {y} must land on a row, not fall through the stack"
            );
            y += 1.0;
        }

        stack.remove();
    }

    /// The reach stops at the stack's ends. Past them lies the bar or the
    /// page, and a row that took presses there would be stealing them.
    #[wasm_bindgen_test]
    async fn the_ends_of_a_stack_do_not_reach_past_it() {
        let stack = mount();
        let first = row(&stack, "#opener").get_bounding_client_rect();
        let last = row(&stack, "#third").get_bounding_client_rect();
        let x = first.left() + first.width() / 2.0;

        for y in [first.top() - 3.0, last.bottom() + 3.0] {
            assert!(
                hit(x, y).is_none_or(|element| element.tag_name() != "TONK-MI"),
                "the stack must leave the page at {y} alone"
            );
        }

        stack.remove();
    }
}
