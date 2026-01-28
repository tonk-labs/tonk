//! Test helpers for the UCAN access service.
//!
//! This module provides a local UCAN access service for integration testing.
//! It mirrors the behavior of the Cloudflare Worker but runs as a native HTTP
//! server, allowing tests to run without deploying to Cloudflare.

#[cfg(not(target_arch = "wasm32"))]
pub mod server;

#[cfg(not(target_arch = "wasm32"))]
pub use server::*;

// Re-export Operator from dialog-storage for convenience in tests
pub use dialog_storage::s3::helpers::Operator;
