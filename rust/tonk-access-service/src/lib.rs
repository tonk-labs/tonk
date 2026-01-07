//! UCAN Access Service
//!
//! This service provides UCAN-authorized access to R2 storage.
//! It verifies UCAN invocations and returns pre-signed URLs for
//! blob read/write operations.

use worker::*;

mod error;
mod handlers;
mod identity;
mod r2;
mod ucan;

/// Worker entrypoint
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // Panic hook for better error messages
    console_error_panic_hook::set_once();

    // Route requests
    let router = Router::new();

    router
        // Service info endpoint
        .get_async("/", handlers::info::handle)
        // Health check
        .get_async("/health", handlers::health::handle)
        // UCAN invocation endpoint
        .post_async("/", handlers::invocation::handle)
        // 404 for everything else
        .run(req, env)
        .await
}
