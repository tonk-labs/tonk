#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk, built with Leptos.

/// API client for interacting with the Tonk service worker.
pub mod api;
/// UI components for the Tonk application.
pub mod components;

/// DID parsing helpers.
pub mod did;

/// Error types for the Tonk UI.
pub mod error;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;
