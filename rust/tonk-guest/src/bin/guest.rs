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

    // The passive routing annotators (`<tonk-repository>` / `<tonk-branch>`).
    // These do NO IO — they only annotate `detail.space` / `detail.branch` on
    // outbound operation events as they bubble. Without them a nested
    // `<tonk-repository name=…>` inside the guest is inert, so a query can't be
    // scoped to another repo; with them, the proxy host forwards the annotated
    // route over the bridge (honored only for a privileged FAB portal).
    tonk_host::register_routing_elements();

    tonk_sigil::Sigil::install();
    // The opaque guest can't mask sigils against a cross-origin `/sigils.svg`
    // sprite (CSS `url()` is CORS-blocked at a null origin). Fetch the sprite
    // bytes over the host bridge (the overridden `window.fetch` routes
    // host-relative URLs there), mint a same-origin blob URL, and install it
    // as the sigil default — re-rendering any sigils that already painted.
    load_sigil_sprite();
    tonk_display::register();
    tonk_board::register();
    tonk_workspace::register();
    tonk_tree::register();
    tonk_fab::register();
    // The scratch inspector — a leptos-free notebook element that evaluates by
    // POSTing to the branch's `/evaluate` endpoint over the host fetch bridge.
    // Needs the `<tonk-code>` editor bundle injected too (the portal does that).
    tonk_inspector::register();
    // A view inside the guest can itself mount a `<tonk-portal>` (the Sketch
    // sheet's imperative canvas). Register it so a NESTED portal upgrades —
    // it nests cleanly since the canvas portal is plain `content=` (a
    // self-contained srcdoc), needing no runtime injection or network.
    tonk_portal::register();
}

/// Fetch the sigil sprite over the host bridge, mint a same-origin blob URL,
/// and install it as the global sigil sprite default. Best-effort: any
/// failure leaves the build-time `/sigils.svg` default in place (which a
/// sealed guest can't load, so sigils render blank — acceptable degradation,
/// not a crash).
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn load_sigil_sprite() {
    use wasm_bindgen_futures::{JsFuture, spawn_local};
    use web_sys::{Blob, Response};

    // Log failures rather than swallowing them: if the bridge can't deliver
    // the body, the sigil would otherwise silently fall back to the
    // CORS-blocked `/sigils.svg` and render blank with no clue why.
    fn warn(message: &str) {
        web_sys::console::warn_1(&JsValue::from_str(message));
    }

    spawn_local(async move {
        let Some(window) = web_sys::window() else {
            return;
        };
        // Routes through the overridden `window.fetch` → host bridge.
        let resp = match JsFuture::from(window.fetch_with_str("/sigils.svg")).await {
            Ok(resp) => resp,
            Err(e) => return warn(&format!("sigil sprite: fetch failed: {e:?}")),
        };
        let Ok(resp) = resp.dyn_into::<Response>() else {
            return warn("sigil sprite: fetch did not yield a Response");
        };
        let blob = match resp.blob() {
            Ok(promise) => match JsFuture::from(promise).await {
                Ok(blob) => blob,
                Err(e) => return warn(&format!("sigil sprite: reading body failed: {e:?}")),
            },
            Err(e) => return warn(&format!("sigil sprite: blob() failed: {e:?}")),
        };
        let Ok(blob) = blob.dyn_into::<Blob>() else {
            return warn("sigil sprite: body was not a Blob");
        };
        match web_sys::Url::create_object_url_with_blob(&blob) {
            Ok(url) => tonk_sigil::set_default_sprite_href(&url),
            Err(e) => warn(&format!("sigil sprite: createObjectURL failed: {e:?}")),
        }
    });
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}
