#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk.

/// API client for interacting with the Tonk service worker.
pub mod api;

/// PostHog wiring for the shell page: panic hook, pageviews, and
/// DOM-event listeners. Wasm-only — depends on `tonk_analytics::web`,
/// which only exists for `wasm32-unknown-unknown`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod analytics;

/// Error types for the Tonk UI.
pub mod error;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;

/// Real-browser passkey ceremony tests.
#[cfg(test)]
mod identity;
