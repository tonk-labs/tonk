//! `<tonk-table>` — an IronCalc-backed spreadsheet packaged as a
//! browser-native custom element. The element itself is implemented in
//! TypeScript (see `src-js/`) and bundled by `scripts/build.mjs` into
//! `assets/tonk-table.js`. This crate's job is to ship that bundle as a
//! Trunk asset and provide an [`install`] entry point that injects the
//! `<script type="module">` tag.
//!
//! # Lazy loading
//!
//! The main bundle is a thin shell: it registers the custom element and
//! nothing else. The grid machinery lives in two sibling chunks the
//! element pulls in on the first `connectedCallback`:
//!
//! - `tonk-table-grid.js` — the grid UI plus the IronCalc JS glue.
//! - `tonk-table-engine.js` — the IronCalc engine wasm, base64-embedded
//!   (esbuild `binary` loader) in a pure data leaf that only changes on
//!   an IronCalc version bump, so grid iteration doesn't rewrite a
//!   multi-megabyte artifact.
//!
//! Pages that ship the bundle but never render a `<tonk-table>` pay only
//! for the shell. The engine is instantiated *from bytes* — never from a
//! URL fetch — which is what lets the whole graph blob-mint into a
//! sealed, opaque-origin portal guest (tonk-portal's `bridge.rs` walks
//! the relative-import graph and rewrites the seams to blob URLs).
//!
//! # Why a JS bundle (and not a Rust-side `CustomElement` like
//! `tonk-sigil`)?
//!
//! Same trade-off as `tonk-prose`/`tonk-code`: IronCalc is a sizable
//! TypeScript + wasm library. Bundling on the JS side keeps the grid
//! authoring ergonomics native, lets us instantiate the engine wasm
//! from inlined bytes, and gives us code-splitting out of the box.

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::install;
