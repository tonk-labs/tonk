//! `<tonk-notebook>`: a prose document whose ```dialog fences are live
//! query cells, and `<tonk-notebook-index>`, its directory's search box.
//!
//! Leptos-free custom elements, so they register inside the sealed-iframe
//! guest, which deliberately depends on neither `tonk-ui` (Leptos) nor the
//! query engine. A notebook owns no branch and never links the engine: it
//! resolves its `(repo, branch)` from its routing context and evaluates
//! through the host, which the guest relays over the bridge.
//!
//! The console's query inspector that this crate was named for is now a
//! library component (`rust/tonk-core/assets/library/inspector.yaml`).

/// The engine-free mirror of the worker's `/evaluate` response. Pure serde
/// types with no DOM, so they build everywhere, which is what lets the
/// renderers that read them be tested off-target.
pub mod response;

/// A schema result's stored descriptor as the notation that would write it.
/// Pure, so it builds everywhere; `cell_output` draws schema rows with it.
pub mod descriptor;

/// Projecting ordered blocks into one markdown document and back. Pure
/// string logic with no DOM, so unlike the elements it builds and tests on
/// every target.
pub mod blocks;

/// Rendering a notebook cell's result: compact, capped, and drawn by
/// `<tonk-display>` where the result names a model. Pure string logic, so it
/// builds and tests on every target.
pub mod cell_output;

/// Notation documents the elements evaluate: pure, target-agnostic.
pub mod notation;

/// The branch an element's `with` names, and evaluating against it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod host;

/// `<tonk-notebook-index>`: the directory's search-and-create box.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod index;

/// `<tonk-notebook>`: a prose document whose ```dialog fences are live
/// query cells.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod notebook;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use index::TonkNotebookIndexElement;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use notebook::TonkNotebookElement;

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
pub fn register_notebook() {}
