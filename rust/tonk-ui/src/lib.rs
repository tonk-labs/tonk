#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk.

/// Customer activation page reached from the activation email.
pub mod activate;
/// Top-document account creation and self-link element.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod ceremony;

mod account_observability;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use account_observability::spawn_settle_probe;

/// The account panel's registration row, kept live by a subscription to
/// the fact rather than a fetch, so an activation performed elsewhere
/// reaches this tab.

/// Top-document gate sending a signed-out user to sign up, and replaying
/// what they were doing when it fired.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]

/// Running a WebAuthn ceremony on the service worker's behalf.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod custody_relay;

/// The registration dialog raised when sharing needs an account.
pub mod register_dialog;

/// API client for interacting with the Tonk service worker.
pub mod api;

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
/// PostHog wiring for the shell page: panic hook, pageviews, and
/// DOM-event listeners. Wasm-only — depends on `tonk_analytics::web`,
/// which only exists for `wasm32-unknown-unknown`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod analytics;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod device_name;

/// Error types for the Tonk UI.
pub mod error;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod identity_bridge;

mod user_error;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;

/// Real-browser account-panel and CLI roundtrip tests.
#[cfg(test)]
mod account_flow;

/// Real-browser passkey ceremony tests.
#[cfg(test)]
mod identity;

/// Real-browser service-worker load-time upgrade tests.
#[cfg(test)]
mod service_worker_upgrade;
