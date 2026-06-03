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

    tonk_sigil::Sigil::install();
    tonk_host::register();
    tonk_concept::register();
    tonk_display::register();
    tonk_layout::register();
    tonk_board::register();
    tonk_portal::register();
    tonk_ui::components::register_inspector();
    // Inject the `<tonk-code>` element's JS bundle. The path
    // matches the `copy-dir` target in `index.html` — both must
    // change together if the deployment layout shifts.
    tonk_code::install("/tonk-code/tonk-code.js");

    // Dev-only hot reload client. `debug_assertions` is on under
    // `trunk serve` (debug profile) and off for `trunk build
    // --release`, so the pill, the trunk-WS tap, and the in-place
    // library re-seed it drives never load in production. The file is
    // copied to dist by `index.html` but stays unreferenced in
    // release builds.
    #[cfg(debug_assertions)]
    inject_hot_swap();

    leptos::mount::mount_to_body(TonkShell);
}

/// Append `<script type="module" src="/hot-swap.js">` to the document
/// head. Debug-only — see the call site.
#[cfg(all(target_arch = "wasm32", target_os = "unknown", debug_assertions))]
fn inject_hot_swap() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(head) = document.head() else {
        return;
    };
    let Ok(script) = document.create_element("script") else {
        return;
    };
    let _ = script.set_attribute("type", "module");
    let _ = script.set_attribute("src", "/hot-swap.js");
    let _ = head.append_child(&script);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
