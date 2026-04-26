//! Test helpers for the UCAN access service.
//!
//! This module provides a local UCAN access service for integration testing.
//! It mirrors the behavior of the Cloudflare Worker but runs as a native HTTP
//! server, allowing tests to run without deploying to Cloudflare.

use serde::{Deserialize, Serialize};

/// Connection info for the UCAN access service test server.
///
/// Contains all information needed to configure `ucan::Credentials` and
/// connect to the backing S3 server for test verification.
///
/// This struct is available on all platforms so it can be used as a test
/// parameter in WASM tests, even though the server only runs natively.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessServiceAddress {
    /// URL of the UCAN access service (e.g., "http://127.0.0.1:8080")
    pub access_service_url: String,
    /// URL of the backing S3 server (for test verification)
    pub s3_endpoint: String,
    /// The bucket name
    pub bucket: String,
    /// AWS access key ID (used by access service, exposed for verification)
    pub access_key_id: String,
    /// AWS secret access key (used by access service, exposed for verification)
    pub secret_access_key: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub mod server;

#[cfg(not(target_arch = "wasm32"))]
pub use server::*;

// Re-export SignerCredential for convenience in tests
#[cfg(not(target_arch = "wasm32"))]
pub use dialog_credentials::SignerCredential;
