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
/// The engine-free mirror of the worker's `/evaluate` response. Pure serde
/// types with no DOM, so they build everywhere — which is what lets the
/// renderers that read them be tested off-target.
pub mod response;

/// Projecting ordered blocks into one markdown document and back. Pure
/// string logic with no DOM, so unlike the elements it builds and tests on
/// every target.
pub mod blocks;

/// Rendering a notebook cell's result: compact, capped, and drawn by
/// `<tonk-display>` where the result names a model. Pure string logic, so it
/// builds and tests on every target.
pub mod cell_output;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod element;

/// `<tonk-notebook-index>` — the directory's search-and-create box.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod index;

/// `<tonk-notebook>` — a prose document whose ```dialog fences are live
/// query cells. Shares this crate's evaluate path and result rendering with
/// `<tonk-inspector>`; see [`notebook`] for why it lives here rather than in
/// a crate of its own.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod notebook;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use element::TonkInspectorElement;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use index::TonkNotebookIndexElement;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use notebook::TonkNotebookElement;

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

/// Register `<tonk-notebook>`. Idempotent.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub fn register_notebook() {
    use custom_elements::CustomElement;
    use web_sys::window;

    let registered = window()
        .map(|win| !win.custom_elements().get("tonk-notebook").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    TonkNotebookElement::define("tonk-notebook");

    let indexed = window()
        .map(|win| {
            !win.custom_elements()
                .get("tonk-notebook-index")
                .is_undefined()
        })
        .unwrap_or(false);
    if !indexed {
        TonkNotebookIndexElement::define("tonk-notebook-index");
    }
}

/// Off-target builds have no DOM; the element only exists in the browser.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn register() {}

/// Off-target builds have no DOM; the element only exists in the browser.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn register_notebook() {}
