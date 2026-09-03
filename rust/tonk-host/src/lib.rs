//! IO ownership and `with` routing context for tonk custom
//! elements.
//!
//! See `plan/tonk-routing-attributes.md` at the repository root for
//! the design.
//!
//! Architecture:
//!
//! - **`install()`** — called once at app boot. Attaches the
//!   operation-event listeners to `document` (the host has no
//!   element; bubbling consumer events reach the document for
//!   free), owns the subscription registry, and installs the
//!   navigate provider, the idle-sync heartbeat, and the `with`
//!   observer.
//! - **`with="branch@repo"`** — the routing context, carried on a
//!   consumer or any ancestor. Resolved at handle time via
//!   [`resolve_with`]; see [`location`] for the grammar.
//!
//! Consumer elements (e.g. `<tonk-display>`) dispatch one of five
//! operation events on themselves; the events bubble to the
//! document, where the installed host performs IO.
//!
//! Event names: `tonk-subscribe`, `tonk-query`, `tonk-claim`,
//! `tonk-evaluate`, `tonk-unsubscribe`. All bubble + composed.

#![warn(missing_docs)]

// Target-independent — `ErrorDetail` / `ErrorKind` are plain
// data types used by both native tests in consumer crates and
// the wasm-side host. The event-name constants are likewise pure
// data.
pub mod error;
/// Light/dark across the whole frame tree.
pub mod theme;
// Target-independent — the `branch@repo` / `allow` grammar for the
// routing attributes. Pure data + parsing, natively testable.
pub mod location;

/// Event names that form the wire contract between consumers
/// and the host.
pub mod events {
    /// Open a live subscription.
    pub const SUBSCRIBE: &str = "tonk-subscribe";
    /// One-shot read.
    pub const QUERY: &str = "tonk-query";
    /// Write a structured `TransactRequest`.
    pub const CLAIM: &str = "tonk-claim";
    /// Write a raw asserted-notation document.
    pub const EVALUATE: &str = "tonk-evaluate";
    /// Close a previously-opened subscription.
    pub const UNSUBSCRIBE: &str = "tonk-unsubscribe";

    /// All four operation event names. Consumer crates install the
    /// depth annotator on this set.
    pub const OPERATIONS: &[&str] = &[SUBSCRIBE, QUERY, CLAIM, EVALUATE];
}

// Wasm-only — the installed host, transport, and event-dispatch
// glue all assume a browser environment.
#[cfg(target_arch = "wasm32")]
pub mod bridge;
#[cfg(target_arch = "wasm32")]
pub mod consumer;
#[cfg(target_arch = "wasm32")]
mod context;
#[cfg(target_arch = "wasm32")]
mod depth;
#[cfg(target_arch = "wasm32")]
mod display_name;
#[cfg(target_arch = "wasm32")]
pub use display_name::set_account_display_name;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod http;
#[cfg(target_arch = "wasm32")]
pub use http::{delete_json, get_json, post_json};
#[cfg(target_arch = "wasm32")]
mod navigate;
#[cfg(target_arch = "wasm32")]
mod page_effect;
#[cfg(target_arch = "wasm32")]
pub use navigate::{navigate_to, reload_page, request_registration};
#[cfg(target_arch = "wasm32")]
mod open;
#[cfg(target_arch = "wasm32")]
pub use open::open_external;
#[cfg(target_arch = "wasm32")]
mod title;
#[cfg(target_arch = "wasm32")]
pub use title::set_title;
#[cfg(target_arch = "wasm32")]
mod ops;
#[cfg(target_arch = "wasm32")]
pub use context::{resolve_with, route_of};
// Service-worker readiness gate. The wasm implementation awaits
// `globalThis.serviceWorkerActivates`; on native the module
// exposes the same `wait()` symbol as an immediate no-op so
// shared code paths (e.g. `api.rs` in the UI crate, which is
// wasm-targeted in practice but pulled into native test builds
// via Cargo's metadata graph) don't fail to resolve.
pub mod ready;
#[cfg(target_arch = "wasm32")]
mod registry;
// The per-space synthetic origin a sealed guest believes it lives at.
// Pure string logic (no wasm), so it compiles + tests on native too.
pub mod space_origin;
#[cfg(target_arch = "wasm32")]
pub mod sse;
#[cfg(target_arch = "wasm32")]
mod url;

#[cfg(target_arch = "wasm32")]
pub use depth::{DepthAnnotator, install_depth_annotator};

/// Install the host on the top page: document-level operation
/// listeners, the navigate provider, the idle-sync heartbeat, and
/// the `with` observer. Idempotent — calling more than once is
/// harmless.
#[cfg(target_arch = "wasm32")]
pub fn install() {
    host::install();
}

/// Install only the IO surface (operation listeners + the `with`
/// observer) — the sealed guest's host. IO goes through plain
/// `fetch`, which the portal bootstrap's override relays to the
/// outer frame; the top-page-only effects (navigate provider,
/// idle-sync heartbeat) are skipped. Idempotent.
#[cfg(target_arch = "wasm32")]
pub fn install_io() {
    host::install_io();
}
