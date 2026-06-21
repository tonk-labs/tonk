//! Sealed-guest element runtime entry.
//!
//! Compiled to its own wasm bundle, this is the Leptos-free, worker-free
//! registration surface the sealed iframe loads. The guest's bootstrap
//! imports the generated glue, inits the wasm, then calls [`start`] — which
//! registers the custom elements (a real `<tonk-display>` and friends) plus
//! the guest-side `<tonk-host>` proxy that relays their consumer events to
//! `window.tonk`.
//!
//! It lives in its own crate (not `tonk-ui`) precisely so it does NOT link
//! `tonk-worker` / the query engine — all data/query logic stays in the
//! service worker across the bridge.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

/// Register the guest's custom elements. Call once, after wasm init, from
/// the guest bootstrap. Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();

    // The guest-side proxy host defines `tonk-host` to relay over
    // `window.tonk`; we never register the real `tonk_host`.
    tonk_guest::guest_host::register();

    tonk_sigil::Sigil::install();
    tonk_display::register();
    tonk_board::register();
    tonk_workspace::register();
    tonk_tree::register();
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}
