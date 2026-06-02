//! Board layout custom elements.
//!
//! Three elements ship from this crate:
//!
//! - **`<tonk-board entity="…">`** — the outer wrapper. Takes a board
//!   entity URI and mounts a `<tonk-display>` against it. Resolution
//!   of `:board` name → entity URI happens at the route layer; this
//!   element receives the already-resolved URI.
//! - **`<tonk-strip>`** — horizontal scroll container; used inside
//!   the board view template as the host for column children.
//! - **`<tonk-column>`** — vertical scroll container with
//!   pull-to-reveal gesture; used inside the column view template
//!   as the host for tile children.
//!
//! All three are presentation containers. They do not subscribe to
//! data themselves — view templates (rendered by `<tonk-display>`)
//! supply children. Custom-element behavior is limited to layout
//! and gestures; data flows through the host abstraction defined
//! in `tonk-host`.
//!
//! See `plan/tonk-board.md` at the repository root for the design.

#![warn(missing_docs)]

#[cfg(target_arch = "wasm32")]
mod board;
#[cfg(target_arch = "wasm32")]
mod column;
#[cfg(target_arch = "wasm32")]
mod strip;

/// Register `<tonk-board>`, `<tonk-strip>`, `<tonk-column>` with
/// the page. Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    board::register();
    strip::register();
    column::register();
}
