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
/// Wire-query builders for the resolution pipeline, re-exported from
/// the shared [`tonk_template`] crate so `crate::resolve::*` paths
/// keep resolving.
pub use tonk_template::resolve;
// Notation-template machinery (segment parsing, chrome/repeat
// binding plan, field substitution) folded in from the retired
// `tonk-concept` crate. The pure planning fns are
// target-independent so their tests run under `cargo test`; the
// DOM-walking fns live behind the wasm cfg inside the module.
#[cfg(any(target_arch = "wasm32", test))]
pub mod template;

// `fold` (the multi-row → single-conclusion collapser for
// cardinality-many fields) moved to the shared `tonk_template` crate;
// re-export it so `crate::fold::*` paths keep resolving.
pub use tonk_template::fold;
// The Conclusion-to-source formatter and the highlighter both moved
// to `tonk-notation`, which owns the syntax tree they walk and is
// native-clean — so a terminal renderer can reach them without
// depending on this wasm-oriented crate. Re-exported under their old
// names so `crate::notation_format::…` paths keep resolving.
pub use tonk_notation::format as notation_format;
#[cfg(any(target_arch = "wasm32", test))]
use tonk_notation::highlight as notation_tokens;

#[cfg(any(target_arch = "wasm32", test))]
mod blob_url;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod fallback;
#[cfg(target_arch = "wasm32")]
mod notation;
#[cfg(target_arch = "wasm32")]
mod render;
#[cfg(target_arch = "wasm32")]
mod upload;
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
/// `<tonk-display>` or any other consumer), `<tonk-notation>`
/// (syntax-highlighted dialog-yaml notation renderer used as the
/// carousel's trailing inspection slide), and `<tonk-component>`
/// (the realm-level loader for author-defined web components).
/// Idempotent.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    view::register();
    notation::register();
    element::register();
    fallback::register();
    component::register();
    upload::register();
}
