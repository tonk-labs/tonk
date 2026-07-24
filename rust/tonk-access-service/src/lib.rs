//! UCAN Access Service
//!
//! This service provides UCAN-authorized access to R2 storage.
//! It receives UCAN invocation containers, verifies them using `UcanAuthorizer`,
//! and returns pre-signed S3 request descriptors.
//!
//! ## Endpoints
//!
//! - `POST /ucan/` - Authorize a UCAN invocation container
//! - `PUT /@` - Store a same-origin shortcut target (see the
//!   [`shortcut`] module)
//! - `GET /@/{hash}` - Permanent relative redirect to the stored
//!   target
//!
//! ## Request Format
//!
//! The request body should be a CBOR-encoded UCAN container following the
//! [UCAN Container spec](https://github.com/ucan-wg/container):
//!
//! ```text
//! { "ctn-v1": [invocation_bytes, delegation_0_bytes, ..., delegation_n_bytes] }
//! ```
//!
//! ## Response Format
//!
//! On success, returns a CBOR-encoded `AuthorizedRequest` with:
//! - `url`: Pre-signed S3 URL
//! - `method`: HTTP method (GET, PUT, DELETE)
//! - `headers`: Headers to include in the request
//!
//! On failure, returns an error response with appropriate HTTP status code.

use worker::*;

mod error;
mod handlers;
#[cfg(any(target_arch = "wasm32", test))]
mod revocation;
pub mod shortcut;

/// Test helpers for integration testing.
#[cfg(feature = "helpers")]
pub mod helpers;

/// Worker entrypoint
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let router = Router::new();

    router
        // Service info endpoint
        .get_async("/", handlers::info::handle)
        // Health check
        .get_async("/health", handlers::health::handle)
        // UCAN authorization endpoint (with CORS preflight support)
        .options_async("/ucan/", handlers::ucan::handle_options)
        .post_async("/ucan/", handlers::ucan::handle)
        // Shortcut service: permissionless same-origin link shortening
        .options_async("/@", handlers::shortcut::handle_options)
        .put_async("/@", handlers::shortcut::handle_put)
        .options_async("/@/:hash", handlers::shortcut::handle_options)
        .get_async("/@/:hash", handlers::shortcut::handle_get)
        // 404 for everything else
        .run(req, env)
        .await
}
