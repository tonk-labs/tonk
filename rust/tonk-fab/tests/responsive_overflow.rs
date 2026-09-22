//! Responsive v0.17 rail and attached-panel behavior in a real browser DOM.

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

fn shadow(fab: &HtmlElement, selector: &str) -> Element {
    fab.shadow_root()
        .expect("shadow root")
        .query_selector(selector)
        .expect("valid selector")
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn visible(element: &Element) -> bool {
    window()
        .expect("window")
        .get_computed_style(element)
        .expect("computed style call")
        .expect("computed style")
        .get_property_value("display")
        .expect("display")
        != "none"
}

fn mount(width: i32) -> (HtmlElement, HtmlElement) {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let parent = document
        .create_element("div")
        .expect("parent")
        .dyn_into::<HtmlElement>()
        .expect("HTML parent");
    parent
        .style()
        .set_property("width", &format!("{width}px"))
        .expect("parent width");
    let fab = document
        .create_element("tonk-fab")
        .expect("fab")
        .dyn_into::<HtmlElement>()
        .expect("HTML fab");
    fab.set_attribute("label", "Project Atlas").expect("label");
    parent.append_child(&fab).expect("mount fab");
    document
        .body()
        .expect("body")
        .append_child(&parent)
        .expect("mount fixture");
    (parent, fab)
}

async fn resize(parent: &HtmlElement, width: i32) {
    parent
        .style()
        .set_property("width", &format!("{width}px"))
        .expect("parent width");
    // The v0.17 shell deliberately morphs its outer geometry over 400ms.
    yield_for(450).await;
}

#[dialog_common::test]
async fn the_rail_uses_the_v017_anatomy_and_real_controls() {
    let (parent, fab) = mount(1440);
    yield_for(30).await;

    let wrapper = shadow(&fab, ".w");
    let header = shadow(&fab, ".header");
    let disc = shadow(&fab, ".disc");
    assert!((wrapper.get_bounding_client_rect().width() - 360.0).abs() < 1.0);
    assert!((header.get_bounding_client_rect().height() - 48.0).abs() < 1.0);
    assert!((disc.get_bounding_client_rect().width() - 18.0).abs() < 1.0);

    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    let actions = shadow(&fab, ".run");
    assert!(visible(&actions));
    assert_eq!(
        actions.get_attribute("aria-label").as_deref(),
        Some("space actions")
    );
    for selector in [".login", ".share", ".members", ".agent", ".home"] {
        let control = shadow(&fab, selector);
        assert_eq!(control.tag_name(), "BUTTON");
        assert!((control.get_bounding_client_rect().height() - 48.0).abs() < 1.0);
    }
    fab.remove_attribute("data-account-required")
        .expect("ready account fixture");
    shadow(&fab, ".share")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(
        shadow(&fab, "#share-panel").has_attribute("hidden"),
        "a ready share answers in its action label without opening a panel"
    );
    assert_eq!(
        shadow(&fab, ".share").get_attribute("aria-expanded"),
        Some("false".into())
    );
    assert!(
        fab.query_selector("tonk-menu")
            .expect("legacy selector")
            .is_none()
    );

    parent.remove();
}

#[dialog_common::test]
async fn it_adapts_at_320_390_768_and_1440_pixels() {
    let (parent, fab) = mount(320);
    for (width, expected_width, stacked) in [
        (320, 288.0, true),
        (390, 358.0, true),
        (768, 360.0, false),
        (1440, 360.0, false),
    ] {
        resize(&parent, width).await;
        let wrapper = shadow(&fab, ".w");
        assert_eq!(
            wrapper.class_list().contains("stacked"),
            stacked,
            "{width}px"
        );
        let actual_width = wrapper.get_bounding_client_rect().width();
        assert!(
            (actual_width - expected_width).abs() < 1.0,
            "{width}px rail: expected {expected_width}px, got {actual_width}px"
        );
    }
    parent.remove();
}

#[dialog_common::test]
async fn panels_join_inward_and_stack_on_short_room() {
    let (parent, fab) = mount(768);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&fab, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    yield_for(300).await;
    let wrapper = shadow(&fab, ".w");
    let bar = shadow(&fab, ".bar");
    let panel = shadow(&fab, "#agent-panel");
    assert!(!wrapper.class_list().contains("stacked"));
    let bar_rect = bar.get_bounding_client_rect();
    let panel_rect = panel.get_bounding_client_rect();
    let initially_flipped = wrapper.class_list().contains("flip");
    if initially_flipped {
        assert!(
            panel_rect.right() <= bar_rect.left() + 1.0,
            "right seat: bar=[{}, {}], panel=[{}, {}]",
            bar_rect.left(),
            bar_rect.right(),
            panel_rect.left(),
            panel_rect.right(),
        );
        fab.remove_attribute("flip").expect("left-side seat");
    } else {
        assert!(
            panel_rect.left() >= bar_rect.right() - 1.0,
            "left seat: bar=[{}, {}], panel=[{}, {}]",
            bar_rect.left(),
            bar_rect.right(),
            panel_rect.left(),
            panel_rect.right(),
        );
        fab.set_attribute("flip", "").expect("right-side seat");
    }
    yield_for(300).await;
    let bar_rect = bar.get_bounding_client_rect();
    let panel_rect = panel.get_bounding_client_rect();
    if initially_flipped {
        assert!(panel_rect.left() >= bar_rect.right() - 1.0);
    } else {
        assert!(panel_rect.right() <= bar_rect.left() + 1.0);
    }

    resize(&parent, 390).await;
    fab.set_attribute("up", "").expect("bottom seat");
    yield_for(0).await;
    assert!(wrapper.class_list().contains("stacked"));
    assert!(
        panel.get_bounding_client_rect().bottom() <= bar.get_bounding_client_rect().top() + 1.0
    );
    assert!(visible(&shadow(&fab, ".back")));

    parent.remove();
}

#[dialog_common::test]
async fn the_circle_collapses_and_expands_at_every_width() {
    let (parent, fab) = mount(320);
    for width in [320, 390, 768, 1440] {
        resize(&parent, width).await;
        let circle = shadow(&fab, ".fab").unchecked_into::<HtmlElement>();
        circle.click();
        yield_for(0).await;
        let wrapper = shadow(&fab, ".w");
        assert!(
            wrapper.class_list().contains("collapsed"),
            "collapse at {width}px"
        );
        assert!(wrapper.get_bounding_client_rect().width() >= 48.0);
        circle.click();
        yield_for(0).await;
        assert!(
            !wrapper.class_list().contains("collapsed"),
            "expand at {width}px"
        );
