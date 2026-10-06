#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk.

/// Running a WebAuthn ceremony on the service worker's behalf.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod custody_relay;

#[cfg(any(all(target_arch = "wasm32", target_os = "unknown"), test))]
/// PostHog wiring for the shell page: panic hook, pageviews, and
/// DOM-event listeners. Wasm-only — depends on `tonk_analytics::web`,
/// which only exists for `wasm32-unknown-unknown`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod analytics;

/// Error types for the Tonk UI.
pub mod error;

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
