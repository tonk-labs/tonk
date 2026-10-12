//! UCAN-authorized streaming R2 storage.
//!
//! `POST /ucan/` with the original UCAN in Authorization performs the operation
//! after verifying its chain, expiry, revocation and provisioning. Legacy CBOR
//! requests receive a POST descriptor carrying that same signed invocation.
//! `/object/*` remains a typed 401 retirement endpoint for old bearer URLs.
//! Registration, revocation, hosted-space deletion and shortcuts keep their
//! existing routes. See [`operation`] for request-local storage metadata.

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

pub mod cached;
pub mod deletion;
pub mod describe;
pub mod email;
mod error;
mod handlers;
pub mod lookup;
pub mod metering;
#[cfg(target_arch = "wasm32")]
pub mod objects;
pub mod observability;
pub mod operation;
pub mod provisioning;
pub mod registration;
pub mod revocation;
pub mod revoke;
pub mod service;
pub mod shortcut;
pub mod store;
mod upload;
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
        // Retired bearer URLs: preserve typed401 for existing clients.
        .options_async("/object/*key", handlers::object::handle_options)
        .get_async("/object/*key", handlers::object::handle_get)
        .put_async("/object/*key", handlers::object::handle_put)
        .delete_async("/object/*key", handlers::object::handle_delete)
        // Shortcut service: permissionless same-origin link shortening
        .options_async("/@", handlers::shortcut::handle_options)
        .put_async("/@", handlers::shortcut::handle_put)
        .options_async("/@/:hash", handlers::shortcut::handle_options)
        .get_async("/@/:hash", handlers::shortcut::handle_get)
        // `HEAD` redirects exactly as `GET` does. A caller expanding a
        // short link only needs the URL the redirect lands on, so
        // serving it here spares them the app shell's body; without
        // this the router answers 405 and there is no redirect to
        // follow at all.
        .head_async("/@/:hash", handlers::shortcut::handle_get)
        // 404 for everything else
        .run(req, env)
        .await
}
