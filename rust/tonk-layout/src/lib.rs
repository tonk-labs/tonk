//! `<tonk-layout>` — a niri-style tiling window manager custom
//! element.
//!
//! Arranges tiles on an infinite horizontal scrollable strip of
//! columns, each column a vertical stack of tiles. Every tile
//! mounts a `<tonk-display>` pointed at a branch entity; the
//! layout itself (columns, tiles, sizes, focus) is persisted to
//! the branch as normalized `workspace` / `column` / `tile`
//! entities.
//!
//! See `/plan/tonk-layout.md` at the repo root for the rationale,
//! data model, and interaction design.

#![warn(missing_docs)]

// `State` and its `as_str` mapping are target-independent — the
// DOM-touching `set` helper is individually gated to wasm32 so
// native test builds can still exercise the enum.
#[cfg(any(target_arch = "wasm32", test))]
mod state;

#[cfg(target_arch = "wasm32")]
mod element;

/// Register the `<tonk-layout>` custom element. Idempotent — the
/// shell calls this once at startup. Pages don't have to.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    element::register();
}
