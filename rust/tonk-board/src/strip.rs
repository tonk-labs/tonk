//! `<tonk-strip>` — horizontal scroll container for board columns.
//!
//! Used inside the board view template as the host for column
//! children (which the template iterates from `board.column`).
//! No data of its own — pure presentation container. CSS lives
//! in the consuming app's stylesheet; the element provides the
//! tag name and the structural identity views target.
//!
//! Could be a plain `<div class="strip">` today. Making it a
//! custom element keeps the dispatch namespace tidy and gives us
//! a place to add strip-level behavior later (global focus
//! management, keyboard nav across columns, etc).

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

/// Outer per-element struct.
#[derive(Default)]
pub(crate) struct TonkStrip;

impl CustomElement for TonkStrip {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}
    fn connected_callback(&mut self, _this: &HtmlElement) {}
    fn disconnected_callback(&mut self, _this: &HtmlElement) {}
}

/// Register `<tonk-strip>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkStrip::define("tonk-strip");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-strip").is_undefined()
}
