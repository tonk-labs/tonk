#![warn(missing_docs)]
//! Account service: verified email → root DID, device registry, and
//! chain backup for tonk accounts.
//!
//! Authentication is UCAN invocation containers signed by a device key
//! with the `root → device` chain attached; the invocation subject is
//! the account's root DID. The two bootstrap ceremonies (code request,
//! account creation) use email codes instead, because no delegation
//! exists yet.

use worker::*;

pub mod core;
pub mod email;
pub mod error;
mod handlers;
pub mod store;

/// Worker entrypoint
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .get_async("/", handlers::info::handle)
        .get_async("/health", handlers::health::handle)
        .run(req, env)
        .await
}
