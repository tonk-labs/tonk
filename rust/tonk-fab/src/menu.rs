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
:host{ display:block; max-width:100%; }
/* Height and scrolling live on the WRAPPER, not the host. On the host
   they clip the flyout, which is positioned outside this box on
   purpose — and an `overflow` on the host is what
   `aim_flyout` reads as the clipping boundary, so it measured the
   216px menu instead of the window and never flipped. The wrapper
   scrolls its own rows without any of that. */
.w{ max-height:var(--fabb-menu-max-h, calc(100dvh - 60px));
  overflow-y:auto; overscroll-behavior:contain; }
:host([compact]){ --_mi-min-height:44px; }
:host([hidden]){ display:none !important; }
.w{ position:relative; display:flex; flex-direction:column; gap:7px;
  width:var(--fabb-menu-w, 216px); max-width:100%; }
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

    /// The WRAPPER owns the width, and the host stays a plain block.
    ///
    /// Width, height and overflow on the host all clip the flyout — it
    /// is positioned one gap outside this box on purpose — and an
    /// `overflow` there is what `aim_flyout` reads as the clipping
    /// boundary, so it measured the menu instead of the window and
    /// never flipped the stack away from the screen edge.
    #[test]
    fn the_wrapper_owns_the_requested_menu_width() {
        assert!(CSS.contains(":host{ display:block; max-width:100%; }"));
        assert!(CSS.contains("width:var(--fabb-menu-w, 216px); max-width:100%"));
        assert!(!CSS.contains("width:min(var(--fabb-menu-w"));
    }

    /// Nothing on the host may clip: the flyout lives outside it.
    #[test]
    fn the_host_does_not_clip_its_flyout() {
        let host = &CSS[CSS.find(":host{").expect("a host rule")..];
        let host = &host[..host.find('}').expect("a closed host rule")];
        assert!(!host.contains("overflow"), "host must not clip: {host}");
        assert!(!host.contains("max-height"), "host must not cap: {host}");
    }

    #[test]
    fn tall_menus_scroll_inside_the_available_viewport() {
        // On the wrapper, which scrolls its own rows without clipping
        // what the row flies out.
        assert!(CSS.contains(".w{ max-height:var(--fabb-menu-max-h"));
        assert!(CSS.contains("overflow-y:auto"));
        assert!(CSS.contains("overscroll-behavior:contain"));
    }
}
