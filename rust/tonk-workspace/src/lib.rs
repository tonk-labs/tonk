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
mod default_remote;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod editable;
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
mod hub_account;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod invite_link;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod join_retry;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod origin;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod page;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod sheet;
// Declared on every target: the pure sync-state/preference logic is
// unit-tested natively; the custom elements inside are wasm-gated.
mod sync;
/// `<ui-copy-link>` — a verb that copies a URL and answers in place.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_copy_link;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_dropdown;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_hub_account;
/// `<ui-mode-switch>` — the light/dark cap.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_mode_switch;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_space_remove;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod ui_sync_status;

/// `<tonk-sheet-binder>`, `<tonk-page>`, `<tonk-origin>`,
/// `<tonk-sync-state>` — the status pill that doubles as the pause/resume
/// button — `<tonk-default-remote>`, and `<tonk-editable>`) with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    sheet::register();
    binder::register();
    origin::register();
    page::register();
    sync::register();
    ui_sync_status::register();
    ui_dropdown::register();
    ui_mode_switch::register();
    ui_copy_link::register();
    ui_hub_account::register();
    ui_space_remove::register();
    default_remote::register();
    editable::register();
    join_retry::register();
    invite_link::register();
}
