//! Native test/dev helpers for the account service.
//!
//! [`server::AccountServer`] runs the same 8-route HTTP surface as the
//! Cloudflare Worker (see `src/handlers/`) over a native `hyper` server,
//! backed by [`crate::store::sqlite::SqliteStore`],
//! [`crate::email::CapturedEmail`] — so integration tests and
//! browser-ceremony bench scenarios can drive the ceremonies without a
//! Cloudflare deployment.

pub mod server;

pub use server::AccountServer;
