//! Fit-driven FABB behavior in a real browser DOM.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
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

fn light_element(fab: &HtmlElement, selector: &str) -> Element {
    fab.query_selector(selector)
        .expect("valid selector")
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn click_item(item: &Element) {
    menu_row(item).unchecked_into::<HtmlElement>().click();
}

fn menu_row(item: &Element) -> Element {
    item.shadow_root()
        .expect("menu item shadow")
        .query_selector(".row")
        .expect("row selector")
        .expect("menu row")
}

fn width(element: &Element) -> f64 {
    element.get_bounding_client_rect().width()
}

async fn wait_for_width(element: &Element, expected: f64) {
    for _ in 0..500 {
        if (width(element) - expected).abs() < 1.0 {
            return;
        }
        yield_for(10).await;
    }
    panic!("element width did not settle at {expected}px");
}

fn computed(element: &Element, property: &str) -> String {
    window()
        .expect("window")
        .get_computed_style(element)
        .expect("computed style call")
        .expect("computed style")
        .get_property_value(property)
        .unwrap_or_default()
}

fn escape_event() -> web_sys::Event {
    let init = js_sys::Object::new();
    js_sys::Reflect::set(&init, &"bubbles".into(), &JsValue::TRUE).expect("bubbles");
    js_sys::Reflect::set(&init, &"composed".into(), &JsValue::TRUE).expect("composed");
    js_sys::Reflect::set(&init, &"key".into(), &"Escape".into()).expect("key");
    let constructor = js_sys::Reflect::get(&window().expect("window"), &"KeyboardEvent".into())
        .expect("KeyboardEvent constructor")
        .dyn_into::<js_sys::Function>()
        .expect("KeyboardEvent is constructable");
    let args = js_sys::Array::new();
    args.push(&"keydown".into());
    args.push(&init);
    js_sys::Reflect::construct(&constructor, &args)
        .expect("construct keyboard event")
        .dyn_into::<web_sys::Event>()
        .expect("keyboard event")
}

async fn set_parent_width(
    parent: &HtmlElement,
    fab: &HtmlElement,
    width: i32,
    compact: bool,
    share_visible: bool,
) {
    parent
        .style()
        .set_property("width", &format!("{width}px"))
        .expect("set parent width");
    for _ in 0..50 {
        let wrapper = shadow_element(fab, ".w");
        if wrapper.class_list().contains("compact") == compact
            && visible(&shadow_element(fab, "[data-cell=share]")) == share_visible
        {
            return;
        }
        yield_for(10).await;
    }
    panic!("responsive layout did not settle for a {width}px parent");
}

#[dialog_common::test]
async fn the_stack_uses_one_visible_gap_and_the_disc_collapses_it() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let parent = document
        .create_element("div")
        .expect("create parent")
        .dyn_into::<HtmlElement>()
        .expect("html parent");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    fab.set_attribute("label", "test").expect("label");
    parent.append_child(&fab).expect("mount fab");
    document
        .body()
        .expect("body")
        .append_child(&parent)
        .expect("mount parent");

    set_parent_width(&parent, &fab, 375, true, false).await;
    for selector in ["[data-cell=space]", "[data-cell=share]", "[data-cell=more]"] {
        let trigger = shadow_element(&fab, selector);
        assert!(
            !trigger.has_attribute("aria-haspopup"),
            "{selector} is a disclosure trigger, not a menu button"
        );
    }
    for (selector, label) in [
        ("tonk-menu[data-for=space]", "space actions"),
        ("tonk-menu[slot=sub]", "spaces"),
        ("tonk-menu[data-for=share]", "share actions"),
        ("tonk-menu[data-for=overflow]", "more actions"),
    ] {
        let group = light_element(&fab, selector);
        assert_eq!(group.get_attribute("role").as_deref(), Some("group"));
        assert_eq!(group.get_attribute("aria-label").as_deref(), Some(label));
    }
    let actions = fab.query_selector_all("tonk-mi").expect("action selector");
    for index in 0..actions.length() {
        let action = actions
            .item(index)
            .expect("action")
            .dyn_into::<Element>()
            .expect("action element");
        let row = menu_row(&action);
        assert_eq!(row.tag_name(), "BUTTON", "stack actions stay real buttons");
    }
    fab.set_attribute("up", "").expect("open upward");
    shadow_element(&fab, "[data-cell=more]")
        .unchecked_into::<HtmlElement>()
        .click();

    let overflow = light_element(&fab, "tonk-menu[data-for=overflow]");
    let share = menu_row(&light_element(&fab, "[data-overflow-share]"));
    let mode = menu_row(&light_element(&fab, "[data-overflow-mode]"));
    let internal_gap =
        mode.get_bounding_client_rect().top() - share.get_bounding_client_rect().bottom();
    let menu_wrapper = overflow
        .shadow_root()
        .expect("menu shadow")
        .query_selector(".w")
        .expect("wrapper selector")
        .expect("menu wrapper");
    let bar = shadow_element(&fab, ".bar");
    let bar_gap =
        bar.get_bounding_client_rect().top() - menu_wrapper.get_bounding_client_rect().bottom();
    assert!(
        (bar_gap - internal_gap).abs() < 0.5,
        "bar gap {bar_gap}px must match the {internal_gap}px row gap"
    );

    assert!(
        fab.query_selector("[data-overflow-collapse]")
            .expect("collapse selector")
            .is_none(),
        "collapse belongs to the sync disc, not the overflow menu"
    );
    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(
        shadow_element(&fab, ".w")
            .class_list()
            .contains("collapsed")
    );
    assert!(overflow.has_attribute("hidden"));

    parent.remove();
}

#[dialog_common::test]
async fn the_action_partition_follows_usable_width_without_a_fold() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let parent = document
        .create_element("div")
        .expect("create parent")
        .dyn_into::<HtmlElement>()
        .expect("html parent");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    fab.set_attribute("label", "A deliberately long space name")
        .expect("label");
    parent.append_child(&fab).expect("mount fab");
    document
        .body()
        .expect("body")
        .append_child(&parent)
        .expect("mount parent");

    assert!(
        fab.shadow_root()
            .expect("shadow")
            .query_selector("[data-cell=fold]")
            .expect("selector")
            .is_none()
    );

    // 446 - 2*16 is the inclusive 414px exact fit. Repeating the same
    // delivery must not flap back to compact.
    set_parent_width(&parent, &fab, 446, false, true).await;
    assert!(!shadow_element(&fab, ".w").class_list().contains("compact"));
    set_parent_width(&parent, &fab, 446, false, true).await;
    assert!(!shadow_element(&fab, ".w").class_list().contains("compact"));

    set_parent_width(&parent, &fab, 500, false, true).await;
    assert!(!shadow_element(&fab, ".w").class_list().contains("compact"));
    assert!(visible(&shadow_element(&fab, "[data-cell=share]")));
    assert!(visible(&shadow_element(&fab, "[data-cell=toggle]")));
    assert!(!visible(&shadow_element(&fab, "[data-cell=more]")));
    assert!((width(&shadow_element(&fab, ".bar")) - 414.0).abs() < 0.1);
    let full_label = shadow_element(&fab, ".fab")
        .get_attribute("aria-label")
        .expect("full disc label");
    assert!(!full_label.contains("expand"));
    assert!(!full_label.contains("collapse"));

    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(
        shadow_element(&fab, ".w")
            .class_list()
            .contains("collapsed"),
        "the sync disc must collapse the FABB in a full-width space too"
    );
    wait_for_width(&shadow_element(&fab, ".bar"), 36.0).await;
    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(
        !shadow_element(&fab, ".w")
            .class_list()
            .contains("collapsed")
    );
    wait_for_width(&shadow_element(&fab, ".bar"), 414.0).await;

    shadow_element(&fab, "[data-cell=toggle]")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(fab.get_attribute("mode").is_some());

    set_parent_width(&parent, &fab, 390, true, true).await;
    assert!(shadow_element(&fab, ".w").class_list().contains("compact"));
    assert!(visible(&shadow_element(&fab, "[data-cell=share]")));
    assert!(!visible(&shadow_element(&fab, "[data-cell=toggle]")));
    assert!(visible(&shadow_element(&fab, "[data-cell=more]")));
    assert!((width(&shadow_element(&fab, ".bar")) - 358.0).abs() < 1.0);
    assert!((width(&shadow_element(&fab, ".fab")) - 44.0).abs() < 0.1);
    assert!((width(&shadow_element(&fab, ".disc")) - 14.0).abs() < 0.1);

    // With share visible, compact overflow contains only appearance. The
    // visible share cell opens the canonical stack with no back.
    shadow_element(&fab, "[data-cell=more]")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(light_element(&fab, "[data-overflow-share]").has_attribute("hidden"));
    assert!(!light_element(&fab, "[data-overflow-mode]").has_attribute("hidden"));
    let mode_row = light_element(&fab, "[data-overflow-mode]")
        .shadow_root()
        .expect("mode row shadow")
        .query_selector(".row")
        .expect("row selector")
        .expect("mode row");
    assert!(
        computed(&mode_row, "min-height")
            .trim_end_matches("px")
            .parse::<f64>()
            .expect("row min height")
            >= 44.0
    );
    assert_eq!(
        light_element(&fab, "[data-overflow-mode]")
            .get_attribute("role")
            .as_deref(),
        None
    );
    let mode_is_dark = (fab.get_attribute("mode").as_deref() == Some("dark")).to_string();
    assert_eq!(
        mode_row.get_attribute("aria-pressed").as_deref(),
        Some(mode_is_dark.as_str())
    );
    shadow_element(&fab, "[data-cell=share]")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(!light_element(&fab, "tonk-menu[data-for=share]").has_attribute("hidden"));
    assert!(light_element(&fab, "[data-mi-back]").has_attribute("hidden"));

    set_parent_width(&parent, &fab, 375, true, false).await;
    assert!(shadow_element(&fab, ".w").class_list().contains("compact"));
    assert!(!visible(&shadow_element(&fab, "[data-cell=share]")));
    assert!(visible(&shadow_element(&fab, "[data-cell=more]")));

    let more = shadow_element(&fab, "[data-cell=more]").unchecked_into::<HtmlElement>();
    more.click();
    let overflow = light_element(&fab, "tonk-menu[data-for=overflow]");
    assert!(!overflow.has_attribute("hidden"));
    assert!(!light_element(&fab, "[data-overflow-share]").has_attribute("hidden"));
    assert_eq!(
        fab.query_selector_all("tonk-menu[data-for=share]")
            .expect("canonical share selector")
            .length(),
        1
    );
    let menus = shadow_element(&fab, ".mw").unchecked_into::<HtmlElement>();
    let menu_left = menus.style().get_property_value("left").expect("menu left");
    let menu_right = menus
        .style()
        .get_property_value("right")
        .expect("menu right");

    click_item(&light_element(&fab, "[data-overflow-share]"));
    assert!(overflow.has_attribute("hidden"));
    assert!(!light_element(&fab, "tonk-menu[data-for=share]").has_attribute("hidden"));
    assert!(!light_element(&fab, "[data-mi-back]").has_attribute("hidden"));
    assert_eq!(
        menus.style().get_property_value("left").expect("menu left"),
        menu_left
    );
    assert_eq!(
        menus
            .style()
            .get_property_value("right")
            .expect("menu right"),
        menu_right
    );

    click_item(&light_element(&fab, "[data-mi-back]"));
    assert!(!overflow.has_attribute("hidden"));
    assert!(light_element(&fab, "tonk-menu[data-for=share]").has_attribute("hidden"));

    let mode_before = fab.get_attribute("mode");
    click_item(&light_element(&fab, "[data-overflow-mode]"));
    let mode_after = fab.get_attribute("mode");
    assert_ne!(mode_after, mode_before);
    assert!(
        mode_after
            .as_deref()
            .is_some_and(|mode| mode == "dark" || mode == "light")
    );
    let mode_button = menu_row(&light_element(&fab, "[data-overflow-mode]"));
    assert_eq!(
        mode_button.get_attribute("aria-pressed"),
        Some((mode_after.as_deref() == Some("dark")).to_string())
    );
    assert!(overflow.has_attribute("hidden"));

    more.click();
    document
        .dispatch_event(&escape_event())
        .expect("dispatch Escape");
    assert!(overflow.has_attribute("hidden"));

    more.click();
    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    let wrapper = shadow_element(&fab, ".w");
    assert!(wrapper.class_list().contains("collapsed"));
    assert_eq!(
        shadow_element(&fab, ".fab")
            .get_attribute("aria-label")
            .as_deref(),
        Some("expand FABB · sync: synced · drag to move")
    );
    assert_eq!(
        shadow_element(&fab, "[data-cell=more]")
            .get_attribute("tabindex")
            .as_deref(),
        Some("-1")
    );
    wait_for_width(&shadow_element(&fab, ".bar"), 44.0).await;

    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    yield_for(220).await;
    assert!(!wrapper.class_list().contains("collapsed"));
    assert!(!visible(&shadow_element(&fab, "[data-cell=share]")));

    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    set_parent_width(&parent, &fab, 500, false, true).await;
    assert!(!wrapper.class_list().contains("compact"));
    assert!(wrapper.class_list().contains("collapsed"));
    set_parent_width(&parent, &fab, 375, true, false).await;
    assert!(wrapper.class_list().contains("compact"));
    assert!(wrapper.class_list().contains("collapsed"));

    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(!fab.has_attribute("collapsed"));
    assert!(!wrapper.class_list().contains("collapsed"));
    shadow_element(&fab, ".fab")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(wrapper.class_list().contains("collapsed"));

    let mw = shadow_element(&fab, ".mw");
    assert_eq!(computed(&mw, "transition-property"), "opacity");
    assert_eq!(computed(&mw, "pointer-events"), "none");

    parent.remove();
}

#[dialog_common::test]
async fn the_share_stack_matches_its_rung_and_scrolls_with_a_long_roster() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
    let parent = document
        .create_element("div")
        .expect("create parent")
        .dyn_into::<HtmlElement>()
        .expect("html parent");
    parent
        .style()
        .set_property("position", "fixed")
        .expect("fix parent position");
    parent
        .style()
        .set_property("bottom", "8px")
        .expect("dock parent at bottom");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    fab.set_attribute("label", "test").expect("label");
    fab.set_attribute("up", "").expect("open upward");
    parent.append_child(&fab).expect("mount fab");
    document
        .body()
        .expect("body")
        .append_child(&parent)
        .expect("mount parent");

    set_parent_width(&parent, &fab, 500, false, true).await;
    let share_rung = shadow_element(&fab, "[data-cell=share]");
    share_rung.clone().unchecked_into::<HtmlElement>().click();

    let menu = light_element(&fab, "tonk-menu[data-for=share]");
    let copy = light_element(&fab, "[data-share-link]");
    copy.remove_attribute("hidden").expect("show copy row");
    for index in 0..40 {
        let member = document.create_element("tonk-mi").expect("member row");
        member.set_text_content(Some(&format!("member {index}")));
        menu.append_child(&member).expect("append member row");
    }
    yield_for(20).await;

    assert!(
        (width(&menu) - width(&share_rung)).abs() < 0.5,
        "the share stack border box must match its rung"
    );
    assert_eq!(
        menu.unchecked_ref::<HtmlElement>()
            .style()
            .get_property_value("--fabb-menu-w")
            .expect("menu width"),
        "144px"
    );

    // The scrollport is a dedicated element wrapping the stack, not the
    // menu itself: `.w::before` is the glass underlay at `z-index:-1`, and
    // a negative-z child cannot escape a scroll container's paint context
    // -- put the overflow on `.w` or on the host and the underlay paints
    // behind the scroller, costing every row its ring.
    let scrollport: HtmlElement = menu
        .shadow_root()
        .expect("menu shadow root")
        .query_selector(".port")
        .ok()
        .flatten()
        .expect("the scrollport")
        .unchecked_into();
    assert_eq!(computed(scrollport.unchecked_ref(), "overflow-y"), "auto");
    assert!(
        scrollport.client_height() > 0,
        "an upward stack must have space above its bottom-docked bar"
    );
    assert!(
        scrollport.scroll_height() > scrollport.client_height(),
        "a long member roster must scroll inside the share stack"
    );
    let copy_row = menu_row(&copy).get_bounding_client_rect();
    let viewport = menu.get_bounding_client_rect();
    assert!(copy_row.top() >= viewport.top());
    assert!(copy_row.bottom() <= viewport.bottom());

    parent.remove();
}
