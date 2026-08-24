//! FAB telescope behavior in a real browser DOM.
//!
//! The wireframe collapses and expands the strip over 400ms, and collapsing
//! the strip dismisses any open stack before it can be stranded beside the
//! circle. This test pins both the visual transition and that interaction.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

async fn yield_for(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        window()
            .expect("window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .expect("set timeout");
    });
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("timeout resolves");
}

fn shadow_element(fab: &HtmlElement, selector: &str) -> Element {
    fab.shadow_root()
        .expect("shadow root")
        .query_selector(selector)
        .expect("valid selector")
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn width(element: &Element) -> f64 {
    element.get_bounding_client_rect().width()
}

#[dialog_common::test]
async fn collapse_and_expand_match_the_wireframe_telescope() {
    tonk_fab::register();
    let win = window().expect("window");
    let document = win.document().expect("document");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html element");
    document
        .body()
        .expect("body")
        .append_child(&fab)
        .expect("mount fab");

    // Allow the connected callback and responsive observer to establish the
    // expanded width before measuring the transition.
    yield_for(50).await;
    let tele = shadow_element(&fab, ".tele");
    let circle = shadow_element(&fab, ".fab").unchecked_into::<HtmlElement>();
    let space = shadow_element(&fab, ".space").unchecked_into::<HtmlElement>();
    let menus = shadow_element(&fab, ".mw");
    let style = win
        .get_computed_style(&tele)
        .expect("computed style call")
        .expect("computed style");
    assert_eq!(
        style
            .get_property_value("transition-property")
            .expect("transition property"),
        "max-width"
    );
    assert_eq!(
        style
            .get_property_value("transition-duration")
            .expect("transition duration"),
        "0.4s"
    );
    assert_eq!(
        style
            .get_property_value("transition-timing-function")
            .expect("transition timing"),
        "cubic-bezier(0.25, 0.46, 0.45, 0.94)"
    );

    let expanded_width = width(&tele);
    assert!(expanded_width > 100.0, "strip must start expanded");

    space.click();
    assert!(
        menus.class_list().contains("on"),
        "space stack must be open before collapse"
    );
    circle.click();
    assert!(fab.has_attribute("collapsed"));
    assert!(
        !menus.class_list().contains("on"),
        "collapsing the FAB must dismiss the open stack"
    );

    yield_for(200).await;
    let closing_width = width(&tele);
    assert!(
        closing_width > 0.0 && closing_width < expanded_width,
        "strip must be in flight halfway through close: {closing_width}"
    );
    yield_for(250).await;
    assert!(width(&tele) < 1.0, "strip must finish fully collapsed");

    circle.click();
    assert!(!fab.has_attribute("collapsed"));
    yield_for(100).await;
    let opening_width = width(&tele);
    assert!(
        opening_width > 0.0 && opening_width < expanded_width,
        "strip must be in flight shortly after open: {opening_width}"
    );
    yield_for(350).await;
    assert!(
        (width(&tele) - expanded_width).abs() < 1.0,
        "strip must return to its expanded width"
    );

    fab.remove();
}
