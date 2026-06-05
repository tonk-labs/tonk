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
mod binder;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod share;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod sheet;

/// Register the workspace custom elements (`<tonk-sheet>`,
/// `<tonk-sheet-binder>`, and `<tonk-share>`) with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    sheet::register();
    binder::register();
    share::register();
}
