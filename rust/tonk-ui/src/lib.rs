#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk.

/// Top-document account creation and self-link element.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod account;

/// Top-document gate sending a signed-out user to sign up, and replaying
/// what they were doing when it fired.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod account_gate;

/// API client for interacting with the Tonk service worker.
pub mod api;

/// PostHog wiring for the shell page: panic hook, pageviews, and
/// DOM-event listeners. Wasm-only — depends on `tonk_analytics::web`,
/// which only exists for `wasm32-unknown-unknown`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod analytics;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod deployment;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod device_name;

/// Error types for the Tonk UI.
pub mod error;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod identity_bridge;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;

/// Real-browser account-panel and CLI roundtrip tests.
#[cfg(test)]
mod account_flow;

/// Real-browser passkey ceremony tests.
#[cfg(test)]
mod identity;
