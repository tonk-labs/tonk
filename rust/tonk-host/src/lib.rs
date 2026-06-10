//! `<tonk-host>` + `<tonk-repository>` + `<tonk-branch>` — IO
//! ownership and routing context for tonk custom elements.
//!
//! See `plan/tonk-host.md` at the repository root for the design.
//!
//! Architecture:
//!
//! - **`<tonk-host>`** owns transport, phase-1 cache, subscription
//!   dedup, and the registry of live consumer subscriptions.
//!   Page-level singleton. Lives outside `<Routes>`.
//! - **`<tonk-repository name="…">`** annotates `detail.space` on
//!   outbound consumer events as they bubble.
//! - **`<tonk-branch name="…">`** annotates `detail.branch` on
//!   outbound consumer events as they bubble.
//!
//! Consumer elements (e.g. `<tonk-display>`) dispatch one of five
//! operation events on
//! themselves; the events bubble up through routing elements
//! (which annotate context) to the host (which performs IO).
//!
//! Event names: `tonk-subscribe`, `tonk-query`, `tonk-claim`,
//! `tonk-evaluate`, `tonk-unsubscribe`. All bubble + composed.

#![warn(missing_docs)]

// Target-independent — `ErrorDetail` / `ErrorKind` are plain
// data types used by both native tests in consumer crates and
// the wasm-side host element. The event-name constants are
// likewise pure data.
pub mod error;

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
    /// Dispatched by routing elements (`<tonk-branch>`,
    /// `<tonk-repository>`) on `attributeChangedCallback`. The host
    /// catches it and orchestrates a depth-staggered refresh of
    /// affected subscriptions.
    pub const CONTEXT_REFRESH: &str = "tonk-context-refresh";

    /// All four operation event names. Consumer crates install the
    /// depth annotator on this set.
    pub const OPERATIONS: &[&str] = &[SUBSCRIBE, QUERY, CLAIM, EVALUATE];
}

// Wasm-only — the actual element implementations, transport,
// and event-dispatch glue all assume a browser environment.
#[cfg(target_arch = "wasm32")]
mod branch;
#[cfg(target_arch = "wasm32")]
pub mod bridge;
#[cfg(target_arch = "wasm32")]
pub mod consumer;
#[cfg(target_arch = "wasm32")]
mod depth;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod http;
#[cfg(target_arch = "wasm32")]
mod ops;
// LRU for `tonk-query` responses. The production callers live in
// `host.rs` / `ops.rs` (both wasm-only), but the unit tests run
// natively via `dialog_common::test`, so the module is also
// pulled in for `cfg(test)` builds.
#[cfg(any(target_arch = "wasm32", test))]
mod query_cache;
// Service-worker readiness gate. The wasm implementation awaits
// `globalThis.serviceWorkerActivates`; on native the module
// exposes the same `wait()` symbol as an immediate no-op so
// shared code paths (e.g. `api.rs` in the UI crate, which is
// wasm-targeted in practice but pulled into native test builds
// via Cargo's metadata graph) don't fail to resolve.
pub mod ready;
#[cfg(target_arch = "wasm32")]
mod registry;
#[cfg(target_arch = "wasm32")]
mod repository;
#[cfg(target_arch = "wasm32")]
pub mod sse;
#[cfg(target_arch = "wasm32")]
mod url;

#[cfg(target_arch = "wasm32")]
pub use depth::{DepthAnnotator, install_depth_annotator};

/// Register all three custom elements with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
pub fn register() {
    host::register();
    repository::register();
    branch::register();
}
