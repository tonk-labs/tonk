//! `<tonk-portals>` — a React-backed grid-of-iframes portal UI
//! packaged as a browser-native custom element. The element is
//! implemented in TypeScript/React (see `src-js/`) and bundled by
//! `scripts/build.mjs` into `assets/tonk-portals.js`. This crate
//! ships that bundle as a Trunk asset and exposes [`install`]
//! which injects the loader `<script type="module">` tag.
//!
//! Why a JS bundle: this is a prototype. React + the grid-select
//! drag/resize/pack code already exists in TS form; lifting the
//! whole thing into Leptos would mean porting ~1500 LOC of hooks
//! and state machines. Hosting it as a custom element lets the
//! Leptos shell own the page chrome (banner, share, push/pull)
//! while the React app owns the panel grid inside `<main>`.

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::install;
