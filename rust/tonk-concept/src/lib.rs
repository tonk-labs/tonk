//! `<tonk-concept>` — a custom element that subscribes to a
//! tonk-worker query and renders each match into an
//! author-supplied template.
//!
//! Two phases:
//! 1. Resolve `source` (a bookmark name or concept entity URI)
//!    into a [`tonk_schema::query::Query`] for the live data.
//! 2. Open a streaming `/query` subscription and diff each frame
//!    into the live DOM.
//!
//! See `SPEC.md` at the crate root for the author-facing
//! reference: source-attribute syntax, template detection,
//! `{field}` substitution rules, update behaviour, and the
//! custom events the element fires.

#![warn(missing_docs)]

pub mod error;
pub mod resolve;
pub mod template;

#[cfg(target_arch = "wasm32")]
pub mod bridge;
#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
pub mod fetch;
#[cfg(target_arch = "wasm32")]
mod render;
#[cfg(target_arch = "wasm32")]
pub mod sse;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Register the `<tonk-concept>` custom element with the page.
/// Idempotent — calling more than once is harmless.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn register() {
    element::register();
}
