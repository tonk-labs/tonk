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

/// How long a browser may cache a CORS preflight for this service, in
/// seconds.
///
/// Every content-addressed fetch is a `POST` carrying
/// `Content-Type: application/cbor`, which is not CORS-safelisted, so
/// each one is preflighted. Without a `Max-Age`, Chrome discards the
/// preflight result after five seconds and re-issues `OPTIONS`
/// mid-load, roughly doubling the requests a cold load pays for.
pub(crate) const PREFLIGHT_MAX_AGE: &str = "86400";

pub mod deletion;
pub mod email;
mod error;
mod handlers;
pub mod lookup;
pub mod metering;
pub mod observability;
pub mod provisioning;
pub mod registration;
pub mod revocation;
pub mod revoke;
pub mod service;
pub mod shortcut;
pub mod store;
pub mod vault;

/// Test helpers for integration testing.
#[cfg(feature = "helpers")]
pub mod helpers;

/// Worker entrypoint
#[event(fetch)]
async fn main(req: Request, env: Env, ctx: Context) -> Result<Response> {
    // POST /ucan/ is served outside the Router: recording an invocation
    // must outlive the response, and only the fetch event's `Context`
    // can extend the isolate's life for that write.
    if matches!(req.method(), Method::Post) && req.path() == "/ucan/" {
        return handlers::ucan::serve(req, env, ctx).await;
    }
    let router = Router::new();

    router
        // Browser deployment configuration must run before static assets.
        .get_async("/.well-known/tonk", handlers::config::handle)
        // The service's DID document: its ed25519 key under the host's
        // did:web name.
        .get_async(
            "/.well-known/did.json",
            handlers::registration::handle_did_document,
        )
        // Registration state probe, polled by enrolling clients.
        .get_async("/customer/:did", handlers::registration::handle_customer)
        // Lookup by email address: the `did:web` document naming the
        // customer registered under one. Two segments, so it does not
        // collide with the single-segment probe above.
        .get_async(
            "/customer/:domain/:local/did.json",
            handlers::lookup::handle,
        )
        // Service info endpoint
        .get_async("/", handlers::info::handle)
        // Health check
        .get_async("/health", handlers::health::handle)
        // UCAN authorization CORS preflight; POST is served above.
        .options_async("/ucan/", handlers::ucan::handle_options)
        // Shortcut service: permissionless same-origin link shortening
        .options_async("/@", handlers::shortcut::handle_options)
        .put_async("/@", handlers::shortcut::handle_put)
        .options_async("/@/:hash", handlers::shortcut::handle_options)
        .get_async("/@/:hash", handlers::shortcut::handle_get)
        // 404 for everything else
        .run(req, env)
        .await
}
