//! `<tonk-layout>` — a niri-style tiling workspace custom element.
//!
//! The strip of columns and the tiles inside them are persisted as
//! normalized entities on the branch, so the workspace reconstructs
//! identically on reload or another device.
//!
//! See `SPEC.md` at the crate root for the author-facing reference
//! and `plan/tonk-layout.md` at the repo root for the design.

#![warn(missing_docs)]

// `state::State` and its `as_str` mapping are target-independent so
// the enum can be unit-tested natively; the DOM-touching `set`
// helper is wasm-gated inside the module.
#[cfg(any(target_arch = "wasm32", test))]
mod state;

// Pure-logic ordering keys; native-testable.
mod order;

// Pure-logic ULID encoder; native-testable. (The wasm-side mint
// that supplies current time + crypto randomness lands when the
// writer module needs it.)
mod ulid;

// Layout tree + frame folding; native-testable.
mod model;

// Wire-query builders; native-testable.
mod resolve;

#[cfg(target_arch = "wasm32")]
mod element;

#[cfg(target_arch = "wasm32")]
mod reconcile;

/// Register the `<tonk-layout>` custom element with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
}
