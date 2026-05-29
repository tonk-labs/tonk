//! `<tonk-column>` — vertical scroll container for column tiles.
//!
//! Used inside the column view template as the host for tile
//! children (which the template iterates from `column.tile`).
//!
//! v1 is just the scroll container. The pull-to-reveal gesture
//! lands in a follow-up — that's where this element earns its
//! existence as a custom element rather than a plain `<div>`.
//! When the gesture is wired up, this element will:
//!
//! 1. Track scroll position past the bottom of the tile stack
//!    (overscroll detection).
//! 2. Animate a `+` reveal slot with rubber-band resistance.
//! 3. On release past a threshold, dispatch a `tonk-claim` event
//!    for a `reveal-launcher` transient with the column's URI.
//!
//! The dispatch is the dialog-native handoff: continuous gesture
//! state stays in the element; the discrete commit becomes a
//! transient assertion that rules turn into a real launcher tile.

use custom_elements::CustomElement;
use web_sys::{HtmlElement, window};

/// Outer per-element struct.
#[derive(Default)]
pub(crate) struct TonkColumn;

impl CustomElement for TonkColumn {
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

/// Register `<tonk-column>` with the page. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkColumn::define("tonk-column");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-column").is_undefined()
}
