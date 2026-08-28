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
:host{ display:block; }
/* A long stack scrolls rather than running off the screen -- but ONLY when
   no row of it flies a sub-stack out, because `overflow` clips the flyout,
   and a flyout that has flipped LEFT lands before the scroll container's
   start edge where scrolling cannot reach it at all. The cap sits on the
   HOST, not on `.w`: `.w::before` is the glass underlay at `z-index:-1`,
   and a scroll container is its own paint context, so an overflow on `.w`
   makes the underlay paint behind it instead of showing through -- every
   row loses its ring and the stack reads as one flat block.
   `mark_scrollable` sets the attribute. */
:host([scrolls]){ max-height:var(--fabb-menu-max-h, calc(100dvh - 60px));
  overflow-y:auto; overscroll-behavior:contain; }
:host([compact]){ --_mi-min-height:44px; }
:host([hidden]){ display:none !important; }
.w{ position:relative; display:flex; flex-direction:column; gap:7px;
  width:var(--fabb-menu-w, 216px); }
.w::before{ content:""; position:absolute; inset:0; z-index:-1;
  background:var(--_bg); -webkit-backdrop-filter:var(--_filter); backdrop-filter:var(--_filter);
  -webkit-mask-image:var(--_maskimg, none); mask-image:var(--_maskimg, none); }
"#;

const HTML: &str = r#"<div class="w"><slot></slot></div>"#;

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
        &["mode", "compact"]
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
        if let Some(listener) = shadow::install_system_mode(this) {
            self.listeners.push(listener);
        }
        propagate(this);
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
        if name == "mode" {
            shadow::apply_mode(this);
            propagate(this);
        } else if name == "compact" {
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

/// Pass the resolved mode to every row, then re-cut — a mode change can
/// change a row's height (nothing does today, but the mask is cheap and a
/// stale mask is a visible seam).
fn propagate(this: &HtmlElement) {
    for row in rows(this) {
        shadow::pass_mode(this, &row);
    }
    recut_mask(this);
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
    use super::CSS;

    /// The WRAPPER owns the width, and the host stays a plain block —
    /// exactly the study's shape.
    #[test]
    fn the_wrapper_owns_the_requested_menu_width() {
        assert!(CSS.contains(":host{ display:block; }"));
        assert!(CSS.contains("width:var(--fabb-menu-w, 216px);"));
        assert!(!CSS.contains("width:min(var(--fabb-menu-w"));
    }

    /// A stack with no flyout to lose scrolls, so a long roster or space
    /// list stays on screen. Gated behind `scrolls`, which
    /// `mark_scrollable` sets only when no row carries a sub-stack.
    #[test]
    fn a_stack_without_a_flyout_scrolls_instead_of_overrunning() {
        assert!(CSS.contains(":host([scrolls]){ max-height:var(--fabb-menu-max-h"));
        assert!(CSS.contains("overflow-y:auto"));
        assert!(CSS.contains("overscroll-behavior:contain"));
    }

    /// The cap lives on the HOST, never on `.w`.
    ///
    /// `.w::before` is the glass underlay at `z-index:-1`. A scroll
    /// container is its own paint context, so an `overflow` on `.w` makes
    /// that underlay paint behind the wrapper rather than showing through:
    /// every row loses its ring and the stack renders as one flat block.
    /// That shipped once and was plainly visible on screen.
    #[test]
    fn the_wrapper_never_becomes_the_scrollport() {
        let at = CSS.find(".w{").expect("the wrapper rule");
        let body = &CSS[at..];
        let body = &body[..body.find('}').expect("a closed rule")];
        assert!(
            !body.contains("overflow"),
            "overflow on .w kills the underlay"
        );
        assert!(
            !body.contains("max-height"),
            "a cap on .w brings that overflow"
        );
    }
}
