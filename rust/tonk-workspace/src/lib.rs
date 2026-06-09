//! Viewer workspace custom elements.
//!
//! Home for the workspace-surface web components. The built-in
//! `workspace`/`artifact`/`view` concepts and their views ship in
//! the standard library (`tonk-core/assets/library/core.yaml`),
//! seeded by the service worker at repository creation rather than
//! embedded here. The elements that present those concepts live in
//! this crate.
//!
//! See `plan/tonk-viewer.md` at the repository root for the design.

#![warn(missing_docs)]

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ancestors;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod binder;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod share;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod sheet;
// Declared on every target: the pure sync-state/preference logic is
// unit-tested natively; the custom elements inside are wasm-gated.
mod sync;

/// Register the workspace custom elements (`<tonk-sheet>`,
/// `<tonk-sheet-binder>`, `<tonk-share>`, and `<tonk-sync-state>` — the
/// status pill that doubles as the pause/resume button) with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    sheet::register();
    binder::register();
    share::register();
    sync::register();
}
