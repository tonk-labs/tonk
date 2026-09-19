//! `<tonk-table>` — an IronCalc-backed spreadsheet.
//!
//! The ELEMENT is branch data: an `element!: &tonk-table` in
//! `tonk-core/assets/library/table.yaml`, resolved by the element
//! registry the first time the tag is rendered. Its methods are the
//! shell — mount, teardown, the light-DOM observer, the outbound
//! events — and its getters and setters are `value` / `content` /
//! `version` / `grid`. Editing it is a fact write, not a rebuild.
//!
//! This crate ships what cannot be a fact, and nothing else:
//!
//! - `tonk-table-grid.js` — the grid UI, the IronCalc JS glue, and the
//!   shell-support surface (`grid/host.ts`: the HLC clock, the content
//!   envelope, base64, the claim-row readers, the host stylesheet).
//!   The shell imports this ONE module and reaches everything through
//!   it, because a notation method has no module scope to hold a
//!   second import in.
//! - `tonk-table-engine.js` — the IronCalc engine wasm, base64-embedded
//!   (esbuild `binary` loader) in a pure data leaf that only changes on
//!   an IronCalc version bump, so grid iteration doesn't rewrite a
//!   multi-megabyte artifact.
//!
//! Both are lazy: the shell imports the grid on its first
//! `connected`, and the grid pulls the engine leaf the same way. A page
//! that never renders a `<tonk-table>` fetches neither. The engine is
//! instantiated *from bytes* — never from a URL fetch — which is what
//! lets the whole graph blob-mint into a sealed, opaque-origin portal
//! guest (tonk-portal's `bridge.rs` walks the relative-import graph and
//! rewrites the seams to blob URLs).
//!
//! [`install`] is the seam: it tells the shell where the grid chunk
//! lives.
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
