//! Standalone web-target entry point.
//!
//! Trunk builds this binary into `dist/tonk-concept.js` (+
//! `tonk-concept_bg.wasm`) so the tonk-worker HTML wrapper can load
//! the element from a stable URL instead of inlining a vanilla-JS
//! port. The parent shell already registers `<tonk-concept>` from
//! its own bundle (see `rust/tonk-ui/src/bin/ui.rs`); this entry
//! exists purely so the iframe's `Document` — which has a separate
//! `customElements` registry — can also register.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

/// Module-load hook: `init()` runs this after wasm instantiation,
/// registering `<tonk-concept>` in the host document.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {
    tonk_concept::register();
}

/// Native stub so `cargo check` / `cargo build` on a non-wasm host
/// still succeeds.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
