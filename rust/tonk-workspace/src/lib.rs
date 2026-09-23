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
mod ui_sync_status;

/// Register the workspace custom elements (`<ui-sync-status>`) with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    ui_sync_status::register();
}
