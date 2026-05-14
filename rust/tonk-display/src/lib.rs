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

// `notation_tokens` and `notation_format` are the tokenizer and
// Conclusion-to-source formatter driving the wasm-only `notation`
// element and `<tonk-display>`'s carousel inspector slide. Both
// are target-independent so their tests run under `cargo test`,
// but their non-test consumers live behind the wasm cfg — gate
// them so a plain `cargo build` for the host doesn't flag every
// internal helper dead.
#[cfg(any(target_arch = "wasm32", test))]
mod notation_format;
#[cfg(any(target_arch = "wasm32", test))]
mod notation_tokens;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod notation;
#[cfg(target_arch = "wasm32")]
mod render;
#[cfg(target_arch = "wasm32")]
mod state;
#[cfg(target_arch = "wasm32")]
mod view;

/// Register every custom element this crate ships:
/// `<tonk-display>` (the orchestrator with subscriptions),
/// `<tonk-view>` (the dumb single-template renderer driven by
/// `<tonk-display>` or any other consumer), and `<tonk-notation>`
/// (syntax-highlighted dialog-yaml notation renderer used as the
/// carousel's trailing inspection slide). Idempotent.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    view::register();
    notation::register();
    element::register();
}
