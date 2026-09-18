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
mod analytics;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ancestors;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod editable;
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
mod hub_account;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod page;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod space_login;
// Declared on every target: the pure sync-state/preference logic is
// unit-tested natively; the custom elements inside are wasm-gated.
mod sync;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_account_settings;
/// `<ui-copy-link>` — the Hub verb that mints and copies a space invite.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_copy_link;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_hub_account;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_space_remove;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_sync_status;

/// `<tonk-page>`, `<tonk-sync-state>` — the status pill that doubles as
/// the pause/resume button — and `<tonk-editable>`) with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    space_login::register();
    page::register();
    sync::register();
    ui_sync_status::register();
    ui_copy_link::register();
    ui_hub_account::register();
    ui_account_settings::register();
    ui_space_remove::register();
    editable::register();
}
