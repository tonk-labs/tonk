//! `<tonk-menu>` — a stack.
//!
//! Blocks of the parent rung's width, separated by 7px gaps of pure page
//! (law 2). The bar sets `--fabb-menu-w` from whichever cell opened the
//! stack, so a stack is always exactly as wide as the thing it hangs from.
//!
//! ## One filter per stack
//!
//! Each row wearing its own `backdrop-filter` would stack N blurs and — worse
//! — blur the gaps between them, which are supposed to be pure page. So the
//! stack carries a single underlay that wears the glass once, and a mask
//! carves the gaps back out of it: one black band per row, transparent
//! between. Rows then paint their rings, washes and solid fills above it.
//!
//! Capped rows opt out (they wear their own frost): the underlay is
//! rectangular and cannot follow an 18px radius, so a mask band behind a
//! capped row would show square glass in the corners.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, ResizeObserver, window};

use crate::shadow::{self, Bound};

const CSS: &str = r#"
/* The host carries the width so its border box matches the rung that
   opened it -- a bare block would stretch to the bar instead. Width alone
   is safe: it neither clips the flyout nor creates a stacking context.
   `overflow` is the property that does both, and it stays gated below. */
:host{ display:block; width:var(--fabb-menu-w, 216px); max-width:100%; }
:host([compact]){ --_mi-min-height:44px; }
:host([hidden]){ display:none !important; }
/* A long stack scrolls rather than running off the screen, but only when no
   row of it flies a sub-stack out: an `overflow` clips the flyout, and once
   the flyout has flipped LEFT it lands before the scrollport's start edge
   where scrolling cannot reach it at all. `mark_scrollable` sets `scrolls`.
   The scrollport is its own element so `.w` stays a plain block. */
.port{ display:block; }
/* The clip region of an overflow box is its PADDING box, and every row's
   ring is a 1px box-shadow drawn just outside the row — flush rows put
   those shadows exactly on the clip edge, and the stack loses its side
   (and endmost) borders the moment it can scroll. One pixel of padding
   keeps the rings inside the clip; the negative margin hands the space
   back so the stack's geometry does not move. */
:host([scrolls]) .port{ max-height:var(--fabb-menu-max-h, calc(100dvh - 60px));
  overflow-y:auto; overscroll-behavior:contain; padding:1px; margin:-1px; }
.w{ position:relative; display:flex; flex-direction:column; gap:7px;
  width:100%; max-width:100%; }
/* The underlay sits at z-index 0 and the rows are lifted above it, rather
   than the underlay being pushed below at `z-index:-1`. A negative-z child
   paints behind its stacking context, and any `overflow` on an ancestor
   creates one -- so the underlay would vanish behind the scroller and every
   row would lose the ring it paints over the glass. Staying at 0 keeps the
   pair in the same context, so the stack looks the same scrolled or not. */
.w::before{ content:""; position:absolute; inset:0; z-index:0;
  background:var(--_bg); -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  -webkit-mask-image:var(--_maskimg, none); mask-image:var(--_maskimg, none); }
::slotted(*){ position:relative; z-index:1; }
"#;

const HTML: &str = r#"<div class="port"><div class="w"><slot></slot></div></div>"#;

/// Per-element state.
#[derive(Default)]
pub(crate) struct TonkMenu {
    listeners: Vec<Bound>,
    observer: Option<ResizeObserver>,
    /// Held for the observer's lifetime — dropping it detaches the callback.
    observer_callback: Option<Closure<dyn FnMut(JsValue, JsValue)>>,
    wired: Rc<RefCell<bool>>,
}

impl CustomElement for TonkMenu {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["compact"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        if *self.wired.borrow() {
            // Re-cut on re-entry: the stack may have been moved between a
            // `sub` slot and the bar's menu slot, which changes its geometry.
            recut_mask(this);
            return;
        }
        *self.wired.borrow_mut() = true;
        if !this.has_attribute("role") {
            let _ = this.set_attribute("role", "group");
        }

        let root = shadow::build(this, CSS, HTML);

        if let Ok(Some(wrapper)) = root.query_selector(".w") {
            let host = this.clone();
            let callback: Closure<dyn FnMut(JsValue, JsValue)> =
                Closure::wrap(Box::new(move |_: JsValue, _: JsValue| {
                    recut_mask(&host);
                }));
            if let Ok(observer) = ResizeObserver::new(callback.as_ref().unchecked_ref()) {
                observer.observe(&wrapper);
                self.observer = Some(observer);
            }
            self.observer_callback = Some(callback);
        }

        if let Ok(Some(slot)) = root.query_selector("slot") {
            let host = this.clone();
            self.listeners
                .push(shadow::bind(&slot, "slotchange", move |_| {
                    recut_mask(&host)
                }));
        }

        self.listeners.push(shadow::install_visibility_pause(this));
        recut_mask(this);
        recut_mask(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        if let Some(observer) = self.observer.take() {
            observer.disconnect();
        }
        self.observer_callback = None;
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
        if name == "compact" {
            recut_mask(this);
        }
    }
}

/// Cap the stack only when no row of it flies a sub-stack out.
///
/// The cap brings `overflow`, and `overflow` clips the flyout -- which sits
/// outside this box, and once flipped sits before its start edge, where
/// scrolling cannot reach it. The stacks that actually grow (a member
/// roster, a space list) carry no flyout, so the two needs never collide on
/// one stack.
fn mark_scrollable(this: &HtmlElement) {
    let carries_flyout = rows(this)
        .iter()
        .any(|row| matches!(row.query_selector("tonk-menu[slot=sub]"), Ok(Some(_))));
    if carries_flyout {
        let _ = this.remove_attribute("scrolls");
    } else {
        let _ = this.set_attribute("scrolls", "");
    }
}

/// The rows the mask cuts bands for: direct `tonk-mi` children that are not
/// capped.
fn rows(this: &HtmlElement) -> Vec<Element> {
    let children = this.children();
    let mut out = Vec::new();
    for index in 0..children.length() {
        let Some(child) = children.item(index) else {
            continue;
        };
        if child.tag_name().eq_ignore_ascii_case("tonk-mi") {
            out.push(child);
        }
    }
    out
}

/// Rebuild the underlay's mask: one black band per uncapped row, transparent
/// across the gaps.
///
/// Public to the crate because the bar cuts it manually the moment a stack
/// opens — the observer fires on its own schedule, and a stack that paints
/// one frame with a stale mask shows glass across its gaps.
pub(crate) fn recut_mask(this: &HtmlElement) {
    mark_scrollable(this);
    let Some(root) = this.shadow_root() else {
        return;
    };
    let Ok(Some(wrapper)) = root.query_selector(".w") else {
        return;
    };
    let wrapper: HtmlElement = wrapper.unchecked_into();
    if wrapper.offset_height() == 0 {
        return;
    }

    let banded: Vec<Element> = rows(this)
        .into_iter()
        .filter(|row| !row.has_attribute("cap"))
        .collect();
    let style = wrapper.style();
    if banded.is_empty() {
        let _ = style.remove_property("--_maskimg");
        return;
    }

    let base = wrapper.get_bounding_client_rect().top();
    let stops: Vec<String> = banded
        .iter()
        .map(|row| {
            let rect = row.get_bounding_client_rect();
            let top = rect.top() - base;
            let bottom = rect.bottom() - base;
            format!(
                "transparent {top:.1}px, black {top:.1}px, black {bottom:.1}px, transparent {bottom:.1}px"
            )
        })
        .collect();
    let _ = style.set_property(
        "--_maskimg",
        &format!("linear-gradient(to bottom, {})", stops.join(", ")),
    );
}

/// Register `<tonk-menu>`. Idempotent.
pub(crate) fn register() {
    let Some(win) = window() else { return };
    if win.custom_elements().get("tonk-menu").is_undefined() {
        TonkMenu::define("tonk-menu");
    }
}

#[cfg(test)]
mod tests {
    use super::{CSS, HTML};

    /// The HOST carries the width, so its border box matches the rung that
    /// opened the stack. A bare block stretches to the bar instead, and
    /// `the_share_stack_matches_its_rung_and_scrolls_with_a_long_roster`
    /// measures the host, not the wrapper.
    ///
    /// Width is safe here in a way `overflow` is not: it neither clips the
    /// flyout nor creates a stacking context that kills the glass.
    #[test]
    fn the_host_carries_the_requested_menu_width() {
        assert!(
            CSS.contains(
                ":host{ display:block; width:var(--fabb-menu-w, 216px); max-width:100%; }"
            )
        );
        assert!(CSS.contains("width:100%; max-width:100%;"));
    }

    /// A stack with no flyout to lose scrolls, so a long roster or space
    /// list stays on screen. Gated behind `scrolls`, which
    /// `mark_scrollable` sets only when no row carries a sub-stack.
    #[test]
    fn a_stack_without_a_flyout_scrolls_instead_of_overrunning() {
        assert!(CSS.contains(":host([scrolls]) .port{ max-height:var(--fabb-menu-max-h"));
        assert!(CSS.contains("overflow-y:auto"));
        assert!(CSS.contains("overscroll-behavior:contain"));
    }

    /// The scroll is gated, never unconditional.
    ///
    /// `overflow` kills the underlay's `backdrop-filter` outright: the glass
    /// stops painting and every row loses the ring it draws over it, so the
    /// stack reads as one flat block. Measured in Chrome 152 -- removing
    /// only `overflow-y` restores the borders, while the port element, the
    /// `max-height`, `isolation` and the z-order change nothing. It also
    /// clips the flyout. So a stack that flies a sub-stack out never scrolls.
    #[test]
    fn only_a_flyout_free_stack_ever_scrolls() {
        assert!(HTML.contains(r#"<div class="port"><div class="w">"#));
        assert!(CSS.contains(":host([scrolls]) .port{"));
        for rule in [":host{", ".w{", ".port{"] {
            let at = CSS.find(rule).expect("the rule");
            let body = &CSS[at..];
            let body = &body[..body.find('}').expect("a closed rule")];
            assert!(
                !body.contains("overflow"),
                "{rule} must not scroll ungated: {body}"
            );
        }
    }
}
