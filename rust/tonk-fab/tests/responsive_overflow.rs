//! Responsive v0.17 rail and attached-panel behavior in a real browser DOM.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use js_sys::{Object, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

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

fn active_animations(element: &Element) -> u32 {
    let get_animations = Reflect::get(element, &"getAnimations".into())
        .expect("getAnimations")
        .dyn_into::<js_sys::Function>()
        .expect("getAnimations is callable");
    get_animations
        .call0(element)
        .expect("read animations")
        .dyn_into::<js_sys::Array>()
        .expect("animation list")
        .length()
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
    let previous_width = parent.client_width();
    let fab = parent.query_selector("tonk-fab").unwrap().expect("fab");
    let wrapper = fab
        .shadow_root()
        .unwrap()
        .query_selector(".w")
        .unwrap()
        .expect("wrapper");
    let wrapper_style = wrapper.unchecked_ref::<HtmlElement>().style();
    let previous_room = wrapper_style.get_property_value("--_room").unwrap();
    parent
        .style()
        .set_property("width", &format!("{width}px"))
        .expect("parent width");
    // The ResizeObserver updates --_room before the width transition starts.
    // Wait for that update and two settled samples so an idle frame before
    // the observer callback cannot report the old rail as the new size.
    let mut settled_samples = 0;
    for _ in 0..60 {
        yield_for(50).await;
        let _ = wrapper.get_bounding_client_rect();
        let room = wrapper_style.get_property_value("--_room").unwrap();
        if (previous_width == width || room != previous_room)
            && active_animations(&fab) == 0
            && active_animations(&wrapper) == 0
        {
            settled_samples += 1;
            if settled_samples == 2 {
                return;
            }
        } else {
            settled_samples = 0;
        }
    }
    panic!("the {width}px rail did not finish resizing");
}

#[dialog_common::test]
async fn the_rail_uses_the_v017_anatomy_and_real_controls() {
    let (parent, fab) = mount(1440);
    yield_for(30).await;

    let wrapper = shadow(&fab, ".w");
    let header = shadow(&fab, ".header");
    let disc = shadow(&fab, ".disc");
    let agent_icon = shadow(&fab, ".agent svg");
    assert!((wrapper.get_bounding_client_rect().width() - 360.0).abs() < 1.0);
    assert!((header.get_bounding_client_rect().height() - 48.0).abs() < 1.0);
    assert!((disc.get_bounding_client_rect().width() - 18.0).abs() < 1.0);
    assert_eq!(agent_icon.get_attribute("fill").as_deref(), Some("none"));
    assert_eq!(
        agent_icon.get_attribute("stroke-width").as_deref(),
        Some("1.7")
    );

    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    let actions = shadow(&fab, ".run");
    assert!(visible(&actions));
    assert!((agent_icon.get_bounding_client_rect().width() - 21.0).abs() < 1.0);
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
async fn a_completed_account_task_retires_the_add_account_action() {
    let (parent, fab) = mount(768);
    yield_for(30).await;
    assert!(fab.has_attribute("data-account-required"));

    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&fab, ".login")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(fab.has_attribute("data-task-hosted"));

    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).expect("task result");
    let init = CustomEventInit::new();
    init.set_detail(&JsValue::from(detail));
    let event = CustomEvent::new_with_event_init_dict("tonk:task-closed", &init)
        .expect("task completion event");
    window()
        .expect("window")
        .dispatch_event(&event)
        .expect("dispatch completion");

    assert!(!fab.has_attribute("data-account-required"));
    assert!(shadow(&fab, ".login").has_attribute("hidden"));
    assert!(!fab.has_attribute("data-task-hosted"));
    parent.remove();
}

#[dialog_common::test]
async fn account_completion_returns_share_to_the_menu_without_a_drawer() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&fab, ".share")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(!shadow(&fab, "#share-panel").has_attribute("hidden"));
    shadow(&fab, ".share-continue")
        .unchecked_into::<HtmlElement>()
        .click();
    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).unwrap();
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    let event = CustomEvent::new_with_event_init_dict("tonk:task-closed", &init).unwrap();
    window().unwrap().dispatch_event(&event).unwrap();
    assert!(!fab.has_attribute("data-account-required"));
    assert!(shadow(&fab, "#share-panel").has_attribute("hidden"));
    assert_eq!(
        shadow(&fab, ".share").get_attribute("aria-expanded"),
        Some("false".into())
    );
    assert!(!shadow(&fab, ".run").has_attribute("hidden"));
    shadow(&fab, ".share")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(shadow(&fab, "#share-panel").has_attribute("hidden"));
    parent.remove();
}

#[dialog_common::test]
async fn share_account_refusal_returns_to_an_already_open_menu() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();

    let needed = CustomEventInit::new();
    needed.set_detail(&"needs-account".into());
    fab.dispatch_event(
        &CustomEvent::new_with_event_init_dict("fabb-account-needed", &needed).unwrap(),
    )
    .unwrap();
    assert!(fab.has_attribute("data-task-hosted"));

    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).unwrap();
    let closed = CustomEventInit::new();
    closed.set_detail(&detail);
    window()
        .unwrap()
        .dispatch_event(
            &CustomEvent::new_with_event_init_dict("tonk:task-closed", &closed).unwrap(),
        )
        .unwrap();
    assert!(!shadow(&fab, ".run").has_attribute("hidden"));
    assert!(shadow(&fab, "#share-panel").has_attribute("hidden"));

    parent.remove();
}

#[dialog_common::test]
async fn long_agent_prompt_scrolls_without_growing_the_drawer() {
    let (parent, fab) = mount(1100);
    fab.remove_attribute("data-account-required").unwrap();
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&fab, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    yield_for(450).await;

    let bar = shadow(&fab, ".bar");
    let panel = shadow(&fab, "#agent-panel");
    let prompt = shadow(&fab, "#agent-panel .panel-copytext").unchecked_into::<HtmlElement>();
    let original_height = bar.get_bounding_client_rect().height();
    prompt.remove_attribute("hidden").unwrap();
    prompt.set_text_content(Some(
        &"Bearer link and complete agent instructions.\n".repeat(120),
    ));

    assert!(
        (bar.get_bounding_client_rect().height() - original_height).abs() < 1.0,
        "the menu keeps its height"
    );
    assert!(
        (panel.get_bounding_client_rect().height() - original_height).abs() < 1.0,
        "the drawer stays aligned with the menu"
    );
    assert!(
        prompt.scroll_height() > prompt.client_height(),
        "the prompt has its own scrollable area"
    );
    prompt.set_scroll_top(80.0);
    assert!(prompt.scroll_top() > 0.0, "the prompt can scroll");
    parent.remove();
}

#[dialog_common::test]
async fn clicking_an_open_drawer_action_returns_to_the_menu() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();

    for (action, panel) in [
        (".share", "#share-panel"),
        (".agent", "#agent-panel"),
        (".members", "#members-panel"),
    ] {
        let button = shadow(&fab, action).unchecked_into::<HtmlElement>();
        button.click();
        assert!(
            !shadow(&fab, panel).has_attribute("hidden"),
            "{action} opens"
        );
        button.click();
        assert_eq!(
            shadow(&fab, panel).get_attribute("aria-hidden").as_deref(),
            Some("true"),
            "{action} drawer becomes inert while closing"
        );
        yield_for(450).await;
        assert!(
            shadow(&fab, panel).has_attribute("hidden"),
            "{action} closes"
        );
        assert!(visible(&shadow(&fab, ".run")), "menu stays open");
        assert!(shadow(&fab, ".w").class_list().contains("menu-open"));
        assert_eq!(
            button.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
    }
    parent.remove();
}

#[dialog_common::test]
async fn closing_a_drawer_contracts_its_column_without_stretching_the_menu() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    let button = shadow(&fab, ".agent").unchecked_into::<HtmlElement>();
    button.click();
    yield_for(450).await;
    let wrapper = shadow(&fab, ".w");
    let bar = shadow(&fab, ".bar");
    let panel = shadow(&fab, "#agent-panel");
    button.click();
    yield_for(80).await;
    let wrapper_width = wrapper.get_bounding_client_rect().width();
    let bar_width = bar.get_bounding_client_rect().width();
    let panel_width = panel.get_bounding_client_rect().width();
    assert!(wrapper_width > 400.0, "drawer is still contracting");
    assert!(bar_width < 362.0, "menu rail remains at its resting width");
    assert!(panel_width > 40.0, "drawer column contracts with the shell");
    button.click();
    assert!(!wrapper.class_list().contains("closing-panel"));
    assert!(!panel.has_attribute("hidden"));
    yield_for(450).await;
    assert!(wrapper.get_bounding_client_rect().width() > 500.0);
    button.click();
    yield_for(500).await;
    assert!(wrapper.get_bounding_client_rect().width() < 362.0);
    assert!(panel.has_attribute("hidden"));
    parent.remove();
}

#[dialog_common::test]
async fn account_prompts_appear_after_the_drawer_finishes_widening() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    shadow(&fab, ".space")
        .unchecked_into::<HtmlElement>()
        .click();

    for (action, prompt) in [(".share", ".share-continue"), (".agent", ".agent-continue")] {
        shadow(&fab, action).unchecked_into::<HtmlElement>().click();
        yield_for(80).await;
        let prompt = shadow(&fab, prompt);
        let style = window()
            .unwrap()
            .get_computed_style(&prompt)
            .unwrap()
            .unwrap();
        let early_opacity: f64 = style
            .get_property_value("opacity")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            early_opacity < 0.05,
            "{action} text stays hidden while widening"
        );
        let mut late_opacity = 0.0;
        for _ in 0..20 {
            yield_for(50).await;
            late_opacity = style
                .get_property_value("opacity")
                .unwrap()
                .parse()
                .unwrap();
            if late_opacity > 0.95 {
                break;
            }
        }
        assert!(
            late_opacity > 0.95,
            "{action} text appears at full width; opacity={late_opacity}"
        );
        shadow(&fab, action).unchecked_into::<HtmlElement>().click();
    }
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
async fn header_follows_the_horizontal_dock_and_menu_follows_the_vertical_dock() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    let wrapper = shadow(&fab, ".w");
    let header = shadow(&fab, ".header");
    let disc = shadow(&fab, ".fab");
    let name = shadow(&fab, ".space");
    let bar = shadow(&fab, ".bar");
    let actions = shadow(&fab, ".run");
    let panel = shadow(&fab, "#share-panel");

    for flipped in [false, true] {
        if flipped {
            fab.set_attribute("flip", "").unwrap();
            fab.style().set_property("right", "16px").unwrap();
            fab.style().set_property("left", "auto").unwrap();
        } else {
            fab.remove_attribute("flip").unwrap();
            fab.style().set_property("left", "16px").unwrap();
            fab.style().set_property("right", "auto").unwrap();
        }
        // Docking applies this class directly alongside the host attribute.
        wrapper
            .class_list()
            .toggle_with_force("flip", flipped)
            .unwrap();
        shadow(&fab, ".space")
            .unchecked_into::<HtmlElement>()
            .click();
        shadow(&fab, ".share")
            .unchecked_into::<HtmlElement>()
            .click();
        yield_for(450).await;
        let disc_rect = disc.get_bounding_client_rect();
        let name_rect = name.get_bounding_client_rect();
        let header_rect = header.get_bounding_client_rect();
        let style = window()
            .unwrap()
            .get_computed_style(&name)
            .unwrap()
            .unwrap();
        assert_eq!(
            style.get_property_value("text-align").unwrap(),
            if flipped { "right" } else { "left" }
        );
        if flipped {
            assert!(disc_rect.right() > name_rect.right());
            assert!(disc_rect.right() <= header_rect.right() + 1.0);
        } else {
            assert!(disc_rect.left() < name_rect.left());
            assert!(disc_rect.left() >= header_rect.left() - 1.0);
        }
        let bar_style = window().unwrap().get_computed_style(&bar).unwrap().unwrap();
        let divider = if flipped {
            "border-left-width"
        } else {
            "border-right-width"
        };
        assert!(
            bar_style
                .get_property_value(divider)
                .unwrap()
                .trim_end_matches("px")
                .parse::<f64>()
                .unwrap()
                > 0.0
        );
        assert!(
            (bar.get_bounding_client_rect().height() - panel.get_bounding_client_rect().height())
                .abs()
                < 1.0,
            "the drawer fills the full height of the menu"
        );

        for up in [false, true] {
            if up {
                fab.set_attribute("up", "").unwrap();
            } else {
                fab.remove_attribute("up").unwrap();
            }
            let header_rect = header.get_bounding_client_rect();
            let actions_rect = actions.get_bounding_client_rect();
            if up {
                assert!(actions_rect.bottom() <= header_rect.top() + 1.0);
            } else {
                assert!(actions_rect.top() >= header_rect.bottom() - 1.0);
            }
        }
        fab.remove_attribute("up").unwrap();
        shadow(&fab, ".space")
            .unchecked_into::<HtmlElement>()
            .click();
        assert!(!wrapper.class_list().contains("has-panel"));
    }
    parent.remove();
}

#[dialog_common::test]
async fn space_name_lines_up_with_menu_labels_on_both_sides() {
    let (parent, fab) = mount(1100);
    yield_for(30).await;
    let wrapper = shadow(&fab, ".w");
    let name = shadow(&fab, ".space .n");
    shadow(&fab, ".login").remove_attribute("hidden").unwrap();

    for flipped in [false, true] {
        if flipped {
            fab.set_attribute("flip", "").unwrap();
            fab.style().set_property("right", "16px").unwrap();
            fab.style().set_property("left", "auto").unwrap();
        } else {
            fab.remove_attribute("flip").unwrap();
            fab.style().set_property("left", "16px").unwrap();
            fab.style().set_property("right", "auto").unwrap();
        }
        wrapper
            .class_list()
            .toggle_with_force("flip", flipped)
            .unwrap();
        shadow(&fab, ".space")
            .unchecked_into::<HtmlElement>()
            .click();
        let name_rect = name.get_bounding_client_rect();
        for selector in [
            ".login span",
            ".share span",
            ".members span",
            ".agent span",
            ".home span",
        ] {
            let label_rect = shadow(&fab, selector).get_bounding_client_rect();
            let offset = if flipped {
                (name_rect.right() - label_rect.right()).abs()
            } else {
                (name_rect.left() - label_rect.left()).abs()
            };
            assert!(
                offset <= 2.0,
                "{selector} is {offset}px from the space name (flip={flipped}, name={}..{}, label={}..{})",
                name_rect.left(),
                name_rect.right(),
                label_rect.left(),
                label_rect.right()
            );
        }
        shadow(&fab, ".space")
            .unchecked_into::<HtmlElement>()
            .click();
    }

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
    }
    parent.remove();
}
