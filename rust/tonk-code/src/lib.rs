//! `<tonk-code>` — a CodeMirror 6-backed code editor packaged as a
//! browser-native custom element. The element itself is implemented in
//! TypeScript (see `src-js/`) and bundled by `scripts/build.mjs` into
//! `assets/tonk-code.js`. This crate's job is to ship that bundle as a
//! Trunk asset and provide an [`install`] entry point that injects the
//! `<script type="module">` tag.
//!
//! # Why a JS bundle (and not a Rust-side `CustomElement` like
//! `tonk-sigil`)?
//!
//! CodeMirror 6 is a sizable TypeScript library composed of many ES
//! modules (`@codemirror/state`, `@codemirror/view`, language packs,
//! …). Reaching it from `wasm-bindgen` is technically possible but
//! costs a vendor-prefixed `js_namespace` per import and forfeits
//! tree-shaking. Bundling on the JS side keeps the editor authoring
//! ergonomics native and gives us code-splitting for language modes
//! out of the box.

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::install;
