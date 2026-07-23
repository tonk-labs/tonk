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

pub mod auth;
pub mod chains;
pub mod core;
pub mod email;
pub mod error;
mod handlers;
#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub mod helpers;
pub mod store;

/// Worker entrypoint: the full HTTP surface, backed by D1, R2, and
/// Resend.
#[cfg(target_arch = "wasm32")]
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .get_async("/", handlers::info::handle)
        .get_async("/health", handlers::health::handle)
        .post_async("/codes", handlers::codes::handle)
        .options_async("/codes", handlers::codes::handle_options)
        .post_async("/accounts", handlers::accounts::handle)
        .options_async("/accounts", handlers::accounts::handle_options)
        .post_async("/devices/list", handlers::devices::handle_list)
        .options_async("/devices/list", handlers::devices::handle_options)
        .post_async("/devices/register", handlers::devices::handle_register)
        .options_async("/devices/register", handlers::devices::handle_options)
        .post_async("/devices/link", handlers::devices::handle_link)
        .options_async("/devices/link", handlers::devices::handle_options)
        .post_async("/devices/revoke", handlers::devices::handle_revoke)
        .options_async("/devices/revoke", handlers::devices::handle_options)
        .post_async("/links", handlers::links::handle_create)
        .options_async("/links", handlers::links::handle_options)
        .post_async("/links/resolve", handlers::links::handle_resolve)
        .options_async("/links/resolve", handlers::links::handle_options)
        .post_async("/links/complete", handlers::links::handle_complete)
        .options_async("/links/complete", handlers::links::handle_options)
        .post_async("/links/consume", handlers::links::handle_consume)
        .options_async("/links/consume", handlers::links::handle_options)
        .post_async("/chains/put", handlers::chains::handle_put)
        .options_async("/chains/put", handlers::chains::handle_options)
        .post_async("/chains/list", handlers::chains::handle_list)
        .options_async("/chains/list", handlers::chains::handle_options)
        .post_async("/chains/get", handlers::chains::handle_get)
        .options_async("/chains/get", handlers::chains::handle_options)
        .run(req, env)
        .await
}

/// Worker entrypoint (native stub): the D1/R2/Resend-backed routes are
/// wasm-only adapters (see `src/handlers/`, `src/store/d1.rs`,
/// `src/chains/r2.rs`, `src/email/resend.rs`), so only the
/// binding-free routes are registered when this crate is checked
/// natively.
#[cfg(not(target_arch = "wasm32"))]
#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .get_async("/", handlers::info::handle)
        .get_async("/health", handlers::health::handle)
        .run(req, env)
        .await
}
