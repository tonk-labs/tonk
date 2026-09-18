//! Sealed-guest element runtime entry.
//!
//! Compiled to its own wasm bundle, this is the Leptos-free, worker-free
//! registration surface the sealed iframe loads. The guest's bootstrap
//! imports the generated glue, inits the wasm, then calls [`start`] — which
//! installs the guest relay (document-level listeners forwarding consumer
//! events to `window.tonk`) and registers the custom elements (a real
//! `<tonk-display>` and friends).
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

    // The guest host: the REAL host IO surface (document listeners
    // servicing consumer events over plain fetch/SSE — the portal
    // bootstrap's `window.fetch` override relays each request to the outer
    // frame), plus the guest-only navigation click relay and site-entity
    // binding. `window.tonk` stays app sugar, not the elements' transport.
    tonk_guest::guest_host::install();

    // Author elements: the runtime that ANNOUNCES an undefined custom
    // element, and the listener that ANSWERS by resolving the tag and
    // registering it. Installed here, at the one place every sealed
    // guest boots through, so no view or page can forget it — a tag
    // renders and is registered because it rendered, never because
    // something was mounted ahead of time.
    //
    // `tonk_display::register()` installs it too (it is idempotent);
    // naming it here is the guarantee that it happens.
    tonk_display::registry::install();
    tonk_display::register();
    tonk_board::register();
    tonk_workspace::register();
    tonk_tree::register();
    tonk_fab::register();
    // The scratch inspector — a leptos-free notebook element that evaluates over
    // the host fetch bridge; its `<tonk-code>` editor + diagnostics provider are
    // injected by the portal.
    tonk_inspector::register();
    // `<tonk-notebook>` — the same evaluate path as the inspector, hosted on a
    // `<tonk-prose>` document whose ```dialog fences are the cells.
    tonk_inspector::register_notebook();
    // A view inside the guest can itself mount a `<tonk-portal>` (the Sketch
    // sheet's imperative canvas). Register it so a NESTED portal upgrades —
    // it nests cleanly since the canvas portal is plain `content=` (a
    // self-contained srcdoc), needing no runtime injection or network.
    tonk_portal::register();
    // The nested router: a space's chrome runs a `<tonk-site>` inside the guest
    // to route the sub-path against the space's route table, rendering its match
    // in a further-nested sealed iframe.
    tonk_portal::register_site();
    // `<tonk-title>` names the browser tab. Headless: it renders nothing
    // and pushes its text to the host page, which owns `document.title`.
    tonk_portal::register_title();
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}
