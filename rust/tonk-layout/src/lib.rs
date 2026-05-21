//! `<tonk-layout>` — headless workspace primitive.
//!
//! Holds the universal state and command primitives for a tile-based
//! workspace: which tiles exist, what content they render, which one
//! is focused, and what linear order they live in. The element has
//! no rendered DOM of its own; UIs that present the workspace ship
//! as `<tonk-display>` view documents that wrap this element.
//!
//! See `SPEC.md` at the crate root for the author-facing reference
//! and `plan/tonk-layout-headless-split.md` at the repo root for the
//! design.

#![warn(missing_docs)]

// Pure-logic lex-midpoint ordering keys; native-testable.
mod order;

// Pure-logic ULID encoder; native-testable. The wasm-side `new_ulid`
// (browser-supplied time + crypto randomness) is gated to wasm.
mod ulid;

// Universal `workspace` + `tile` fold; native-testable.
mod model;

// Wire-query builders for the workspace + tiles subscriptions;
// native-testable.
mod resolve;

// Notation-document builders for the six effects + the wasm-only
// `/evaluate` POST. Builders are native-testable; transport is
// wasm-gated.
mod writer;

#[cfg(target_arch = "wasm32")]
mod element;

/// Register the `<tonk-layout>` custom element with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
}
