//! UI binary entrypoint.
//!
//! This binary initializes and mounts the Tonk UI component to the browser DOM.
//! It is compiled to Wasm by Trunk as configured in [`index.html`](../../../index.html)
//! (see the `data-bin="ui"` link tag).

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
async fn main() {
    use tonk_ui::components::TonkShell;

    console_error_panic_hook::set_once();

    leptos::mount::mount_to_body(TonkShell);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
