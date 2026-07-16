//! `<tonk-fab>` stylesheet injection.
//!
//! `<tonk-fab>` has no shadow root (`element.rs`'s `shadow()` returns
//! `false`), so its CSS is injected as a global `<style id="tonk-fab-styles">`
//! in `connected_callback`, guarded by that stable id rather than the
//! `__tonkFabBound` expando — a clone landing in a fresh document (as
//! `tonk-display` produces when it clones the chrome view) still needs the
//! stylesheet, but must never get a second copy of it. This proves the guard:
//! mounting more than once must still leave exactly one `<style>` in the
//! document.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::window;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
}

/// Create and mount a bare `<tonk-fab>`, running `connected_callback`.
fn mount() -> web_sys::HtmlElement {
    tonk_fab::register();
    let el = document()
        .create_element("tonk-fab")
        .expect("create")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("html element");
    document()
        .body()
        .expect("body")
        .append_child(el.as_ref())
        .expect("append");
    el
}

fn injected_style_count() -> u32 {
    document()
        .query_selector_all("#tonk-fab-styles")
        .expect("query")
        .length()
}

#[dialog_common::test]
async fn it_injects_the_stylesheet_exactly_once_across_multiple_mounts() {
    let first = mount();
    assert_eq!(
        injected_style_count(),
        1,
        "the first mount must inject exactly one stylesheet"
    );

    // A second, independent `<tonk-fab>` mounting into the SAME document —
    // the shape `tonk-display` produces when it clones the chrome view and
    // mounts the clone. The guard is keyed off the stable element id, not the
    // `__tonkFabBound` expando, precisely so this second mount still finds the
    // stylesheet already present and does not append a duplicate.
    let second = mount();
    assert_eq!(
        injected_style_count(),
        1,
        "a second mount in the same document must not duplicate the stylesheet"
    );

    // Disconnect/reconnect of the SAME element must not duplicate it either.
    first.remove();
    document()
        .body()
        .expect("body")
        .append_child(first.as_ref())
        .expect("reconnect");
    assert_eq!(
        injected_style_count(),
        1,
        "a reconnect must not duplicate the stylesheet"
    );

    second.remove();
    first.remove();
}
