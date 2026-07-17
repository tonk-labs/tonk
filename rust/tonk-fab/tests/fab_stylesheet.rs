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

/// The chevron cap is a compact-only control, but it carries `fab__seg`
/// alongside `fab__more` — and `.fab__seg { display: inline-flex }` sits
/// LATER in the stylesheet than a bare `.fab__more { display: none }`, so a
/// same-specificity hide rule loses the tie and the chevron leaks into the
/// wide bar (where clicking it opens the vertical menu on desktop). This
/// pins the COMPUTED style with the real injected stylesheet, in both modes
/// — exactly what the class-toggle unit tests cannot see.
#[dialog_common::test]
async fn it_hides_the_chevron_cap_outside_compact_mode() {
    let host = mount();
    let more = host
        .query_selector(".fab__more")
        .expect("query")
        .expect("chevron authored");
    let display = |el: &web_sys::Element| {
        window()
            .expect("window")
            .get_computed_style(el)
            .expect("computed style")
            .expect("style declaration")
            .get_property_value("display")
            .expect("display value")
    };

    assert_eq!(
        display(&more),
        "none",
        "the wide bar must not render the compact chevron cap"
    );

    let fab = host
        .query_selector(".fab")
        .expect("query")
        .expect("bar authored");
    fab.class_list()
        .add_1("fab--compact")
        .expect("enter compact");
    // Collapsed-compact retracts the chevron with the strip: the end tile
    // clamps to zero width (a transitionable clamp, not display:none). This
    // is the "button hides when the fab is collapsed" contract.
    // `fab--settled` comes off with the collapse — its unclamp rule
    // (`max-width: none` on shown tiles) outranks the collapse clamp, and
    // set_telescope enforces the exclusivity (pinned by the element test
    // `it_collapses_the_compact_bar_with_a_dropdown_open`).
    fab.class_list().remove_1("fab--settled").expect("unsettle");
    fab.class_list().add_1("fab--collapsed").expect("collapse");
    let end_tile = host
        .query_selector(".fab__tele--end")
        .expect("query")
        .expect("end tile authored");
    assert_eq!(
        window()
            .expect("window")
            .get_computed_style(&end_tile)
            .expect("computed style")
            .expect("style declaration")
            .get_property_value("max-width")
            .expect("max-width value"),
        "0px",
        "collapsing the compact bar must clamp the chevron's tile away"
    );
    fab.class_list().remove_1("fab--collapsed").expect("expand");
    fab.class_list().add_1("fab--settled").expect("resettle");
    // Not a literal `inline-flex` check: the chevron is a flex ITEM (its
    // tile is `display: flex`), so browsers blockify the computed value to
    // plain `flex`. What matters is that compact mode shows it at all.
    assert_ne!(
        display(&more),
        "none",
        "compact mode must show the chevron cap"
    );

    host.remove();
}
