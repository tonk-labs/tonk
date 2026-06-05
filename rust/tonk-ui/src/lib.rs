#![warn(missing_docs)]
//! Tonk UI web application.
//!
//! This crate provides the web-based user interface for Tonk, built with Leptos.

/// API client for interacting with the Tonk service worker.
pub mod api;

/// Subscribing to `BroadcastChannel` notifications emitted by the
/// service worker.
pub mod broadcast;

/// UI components for the Tonk application.
pub mod components;

/// Background sync controller — automatic push/pull for the active
/// repository's upstream branches.
pub mod sync_controller;
/// Leptos bridge over [`broadcast`] — exposes a channel's latest
/// message as a reactive signal.
pub mod watch;

/// DID parsing helpers.
pub mod did;

/// Error types for the Tonk UI.
pub mod error;

/// Test helpers for integration testing.
#[cfg(any(test, feature = "helpers"))]
pub mod helpers;
