#![warn(missing_docs)]
//! Account service: verified email → root DID and device registry for
//! tonk accounts.
//!
//! Authentication is UCAN invocation containers signed by a device key
//! with the `root → device` chain attached; the invocation subject is
//! the account's root DID. Account creation is the one ceremony with no
//! delegation to present yet, so it is signed by the root key itself and
//! proves nothing about the address; control of the address is proven
//! afterwards, by activating the customer at the access service.

use worker::*;

pub mod auth;
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
        .post_async("/accounts", handlers::accounts::handle)
        .options_async("/accounts", handlers::accounts::handle_options)
        .post_async(
            "/accounts/setup-status",
            handlers::accounts::handle_setup_status,
        )
        .options_async("/accounts/setup-status", handlers::accounts::handle_options)
        .post_async("/account/summary", handlers::accounts::handle_summary)
        .options_async("/account/summary", handlers::accounts::handle_options)
        .post_async("/account/delete", handlers::accounts::handle_delete)
        .options_async("/account/delete", handlers::accounts::handle_options)
        .post_async(
            "/account/repository/establish",
            handlers::repository::handle,
        )
        .options_async(
            "/account/repository/establish",
            handlers::repository::handle_options,
        )
        .post_async("/devices/list", handlers::devices::handle_list)
        .options_async("/devices/list", handlers::devices::handle_options)
        .post_async("/devices/register", handlers::devices::handle_register)
        .options_async("/devices/register", handlers::devices::handle_options)
        .post_async("/devices/link", handlers::devices::handle_link)
        .options_async("/devices/link", handlers::devices::handle_options)
        .post_async("/devices/detach", handlers::devices::handle_detach)
        .options_async("/devices/detach", handlers::devices::handle_options)
        .post_async("/devices/revoke", handlers::devices::handle_revoke)
        .options_async("/devices/revoke", handlers::devices::handle_options)
        .run(req, env)
        .await
}

/// Worker entrypoint (native stub): the D1/R2/Resend-backed routes are
/// wasm-only adapters (see `src/handlers/`, `src/store/d1.rs`,
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
