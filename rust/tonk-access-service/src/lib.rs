//! UCAN Access Service.
//!
//! This service provides UCAN-authorized access to R2 storage.
//! It verifies UCAN invocations and returns pre-signed URLs for
//! storage read/write operations.
//!
//! Supported storage paths:
//! - `index/*` - Content-addressed tree nodes (blobs)
//! - `local/*` - Local branch state (e.g., `local/main`)
//! - `remote/*` - Remote branch cache
//! - `site/*` - Remote configuration

use worker::*;

mod cors;
mod error;
mod handlers;
mod r2;
mod ucan;

/// Worker entrypoint
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // Handle CORS preflight requests
    if req.method() == Method::Options {
        return cors::preflight_response();
    }

    // Route requests
    let router = Router::new();

    let response = router
        // Service info endpoint
        .get_async("/", handlers::info::handle)
        // Health check
        .get_async("/health", handlers::health::handle)
        // Storage routes with 307 redirects (catch-all for any path under space_did)
        .get_async("/:space_did/*path", handlers::storage::handle_get)
        .put_async("/:space_did/*path", handlers::storage::handle_put)
        // 404 for everything else
        .run(req, env)
        .await?;

    // Add CORS headers to all responses
    cors::with_cors(response)
}
