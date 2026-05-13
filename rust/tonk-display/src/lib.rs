//! `<tonk-display>` — a custom element that renders a single
//! entity using a view template stored on the branch.
//!
//! The element coordinates three live data flows:
//! 1. One-shot resolution of the `model` concept descriptor.
//! 2. A live subscription on the matching `view` row so a template
//!    edited on the branch swaps the rendered DOM.
//! 3. A live subscription on the entity's attributes that patches
//!    the bound DOM in place when fields change.
//!
//! See `/plan/tonk-display.md` at the repo root for the rationale,
//! data-state signalling, and fallback rendering behaviour.

#![warn(missing_docs)]

pub mod resolve;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod render;
#[cfg(target_arch = "wasm32")]
mod state;

/// Register the `<tonk-display>` custom element with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
}
