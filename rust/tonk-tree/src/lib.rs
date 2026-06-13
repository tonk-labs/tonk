//! `<tonk-tree>` — a tree inspector for a dialog index tree,
//! packaged as a browser-native custom element. The element is
//! implemented in TypeScript (see `src-js/`) and bundled by
//! `scripts/build.mjs` into `assets/tonk-tree.js`. This crate's
//! job is to ship that bundle as a Trunk asset and provide an
//! [`install`] entry point that injects the `<script type="module">`
//! tag, exactly like `tonk-code`.
//!
//! Why a JS bundle (and not a Rust-side `CustomElement` like
//! `tonk-inspector`)? The inspector builds on Web Awesome's `<wa-tree>`
//! and is wholly DOM/JS — reaching it from `wasm-bindgen` would forfeit
//! the native authoring ergonomics for no gain. Bundling on the JS side
//! keeps it framework-agnostic and liftable into dialog-db.

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::install;
