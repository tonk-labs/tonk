//! `<tonk-sheet>` — a single sheet, like `<wa-tab>`/`<wa-tab-panel>`
//! rolled into one.
//!
//! A passive container the [`super::binder`] reads. The view that
//! mounts a sheet sets its attributes from the sheet's fields:
//!
//! - `sheet` — the sheet's entity id (the binder's key).
//! - `order` — a lexicographic sort key for the tab strip.
//! - `title` — the tab label.
//! - `icon`  — an optional tab icon name.
//!
//! Its children are the sheet's content (the panel shown in the
//! canvas when this sheet is active). The element ships no behaviour
//! and no shadow DOM; it exists so the binder can discover sheets and
//! their metadata declaratively, the way `<wa-tab-group>` discovers
//! `<wa-tab>` children.

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

/// `<tonk-sheet>` — a structural container, no behaviour of its own.
#[derive(Default)]
pub(crate) struct TonkSheet;

impl CustomElement for TonkSheet {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // The binder reacts to attribute changes via its own
        // MutationObserver, so the element needs no callbacks here.
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}
    fn connected_callback(&mut self, _this: &HtmlElement) {}
    fn disconnected_callback(&mut self, _this: &HtmlElement) {}
}

/// Register `<tonk-sheet>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkSheet::define("tonk-sheet");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-sheet").is_undefined()
}
