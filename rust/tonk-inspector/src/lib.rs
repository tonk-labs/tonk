//! `<tonk-inspector>` — a notebook-style scratch editor over a branch.
//!
//! A standalone, leptos-free custom element (extracted from `tonk-ui`) so it can
//! be registered inside the sealed-iframe guest, which deliberately depends on
//! neither `tonk-ui` (Leptos) nor the query engine. The inspector owns no branch
//! and never links the engine: it resolves its `(repo, branch)` from the routing
//! context and evaluates by POSTing to the branch's `/evaluate` endpoint, which
//! the guest's `window.fetch` proxy routes over the bridge.
//!
//! See [`render`] for the result rendering (an HTML-string port of the former
//! Leptos `view!` tree) and [`response`] for the engine-free wire types.

pub mod debug;

// `render` and `response` are pure logic (string building + serde) but their only
// consumer is the wasm-gated `element`, so they are gated to match — otherwise a
// native `-D warnings` build flags them as dead code. Public so the
// result-rendering port can be exercised by the wasm integration tests.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod render;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod response;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod element;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use element::TonkInspectorElement;

/// Register `<tonk-inspector>`. Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register() {
    use custom_elements::CustomElement;
    use web_sys::window;

    let registered = window()
        .map(|win| !win.custom_elements().get("tonk-inspector").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    TonkInspectorElement::define("tonk-inspector");
}

/// Off-target builds have no DOM; the element only exists in the browser.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn register() {}
