//! `<tonk-prose>` — a ProseMirror-backed, Typora-style markdown editor
//! packaged as a browser-native custom element. The element itself is
//! implemented in TypeScript (see `src-js/`) and bundled by
//! `scripts/build.mjs` into `assets/tonk-prose.js`. This crate's job is
//! to ship that bundle as a Trunk asset and provide an [`install`]
//! entry point that injects the `<script type="module">` tag.
//!
//! # Lazy loading
//!
//! The main bundle is a thin shell: it registers the custom element and
//! nothing else. The ProseMirror machinery (schema, plugins, markdown
//! round-trip) lives in a separate `tonk-prose-editor.js` chunk that the
//! element imports on the first `connectedCallback`. Pages that ship the
//! bundle but never render a `<tonk-prose>` pay only for the shell.
//! Code blocks embed `<tonk-code>` (when defined) which lazy-loads its
//! per-language chunks the same way.
//!
//! # Why a JS bundle (and not a Rust-side `CustomElement` like
//! `tonk-sigil`)?
//!
//! Same trade-off as `tonk-code`: ProseMirror is a sizable TypeScript
//! library composed of many ES modules. Bundling on the JS side keeps
//! the editor authoring ergonomics native and gives us code-splitting
//! out of the box.

#[cfg(feature = "web")]
mod web;

#[cfg(feature = "web")]
pub use web::install;
