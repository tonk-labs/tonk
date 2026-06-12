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

pub mod events;
pub mod resolve;
// Notation-template machinery (segment parsing, chrome/repeat
// binding plan, field substitution) folded in from the retired
// `tonk-concept` crate. The pure planning fns are
// target-independent — native consumers (slide's preview
// diagnostics) call them directly — so the module is not gated;
// the DOM-walking fns live behind the wasm cfg inside the module.
pub mod template;

// `notation_tokens` and `notation_format` are the tokenizer and
// Conclusion-to-source formatter driving the wasm-only `notation`
// element and `<tonk-display>`'s carousel inspector slide. `fold`
// is the multi-row → single-conclusion collapser that handles
// cardinality-many fields. All three are target-independent so
// their tests run under `cargo test`, but their non-test consumers
// live behind the wasm cfg — gate them so a plain `cargo build`
// for the host doesn't flag every internal helper dead.
#[cfg(any(target_arch = "wasm32", test))]
mod fold;
/// Conclusion-to-source notation formatter. `format` turns an
/// entity (`this` URI + projected `fields`) into a `head!:`
/// assertion document — the text `<tonk-notation>` renders.
/// Public so `tonk-ui` can render evaluate results in the same
/// notation the inspector uses. Pure `std` + `serde_json`, so it
/// is target-independent — not gated to wasm like the
/// DOM-touching modules around it.
pub mod notation_format;
#[cfg(any(target_arch = "wasm32", test))]
mod notation_tokens;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod fallback;
#[cfg(target_arch = "wasm32")]
mod notation;
#[cfg(target_arch = "wasm32")]
mod render;
// `state::State` and its `as_str` mapping are target-independent
// — DOM-touching helpers in this module are individually gated
// to wasm32 so native test builds can still exercise the enum.
#[cfg(any(target_arch = "wasm32", test))]
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
    fallback::register();
}
