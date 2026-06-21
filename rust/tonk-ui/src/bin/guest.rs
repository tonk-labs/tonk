//! Sealed-guest element runtime entry.
//!
//! Compiled to its own wasm bundle (`data-bin="guest"`), this is the
//! Leptos-free registration surface the sealed iframe loads. The guest's
//! bootstrap imports the generated glue, inits the wasm, then calls
//! [`start`] — which registers the custom elements (a real `<tonk-display>`
//! and friends) plus the guest-side `<tonk-host>` proxy that relays their
//! consumer events to `window.tonk`.
//!
//! No `mount_to_body`, no `TonkShell`, no `leptos_router`: the guest paints
//! declaratively from the markup the host injects, not from a framework.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

/// Register the guest's custom elements. Call once, after wasm init, from
/// the guest bootstrap. Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();

    // The guest-side proxy host MUST be registered before the real
    // `tonk_host::register()` would be — in fact we never call the real
    // one in the guest. The proxy defines `tonk-host` to relay over
    // `window.tonk`.
    tonk_ui::guest_host::register();

    tonk_sigil::Sigil::install();
    tonk_display::register();
    tonk_board::register();
    tonk_workspace::register();
    tonk_tree::register();
}

// On wasm the entry is `start()`, called by the guest bootstrap after
// init — `main` does nothing. On native there is nothing to run.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}
