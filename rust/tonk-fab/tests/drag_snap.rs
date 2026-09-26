//! Drag release behavior in a real browser DOM.
//!
//! Pure geometry tests prove which edge point is selected. This test pins the
//! component boundary: pointer events reach the shadow handle, the release
//! writes that edge point to the host, and the public event reports it.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{
    CustomEvent, Element, Event, HtmlElement, KeyboardEvent, KeyboardEventInit, ShadowRoot, window,
};

wasm_bindgen_test_configure!(run_in_browser);

fn pointer_event(kind: &str, x: f64, y: f64, buttons: i32) -> Event {
    pointer_event_with_type(kind, x, y, buttons, "mouse")
}

fn pointer_event_with_type(kind: &str, x: f64, y: f64, buttons: i32, pointer_type: &str) -> Event {
    let init = js_sys::Object::new();
    for (name, value) in [
        ("bubbles", JsValue::TRUE),
        ("composed", JsValue::TRUE),
        ("button", 0.into()),
        ("buttons", buttons.into()),
        ("pointerId", 7.into()),
        ("pointerType", pointer_type.into()),
        ("clientX", x.into()),
        ("clientY", y.into()),
    ] {
        js_sys::Reflect::set(&init, &name.into(), &value).expect("set pointer init");
    }
    let constructor = js_sys::Reflect::get(&window().expect("window"), &"PointerEvent".into())
        .expect("PointerEvent constructor")
        .dyn_into::<js_sys::Function>()
        .expect("PointerEvent is constructable");
    let args = js_sys::Array::new();
    args.push(&kind.into());
    args.push(&init);
    js_sys::Reflect::construct(&constructor, &args)
        .expect("construct pointer event")
        .dyn_into::<Event>()
        .expect("pointer event")
}

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
    let get_animations = js_sys::Reflect::get(element, &"getAnimations".into())
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

async fn wait_for_corner_settled(fab: &HtmlElement, root: &ShadowRoot, panel_open: bool) {
    let wrapper = root.query_selector(".w").unwrap().expect("wrapper");
    for _ in 0..60 {
        yield_for(50).await;
        let open = root.query_selector(".w.has-panel").unwrap().is_some();
        if open == panel_open && active_animations(fab) == 0 && active_animations(&wrapper) == 0 {
            return;
        }
    }
    panic!("corner layout did not settle with panel_open={panel_open}");
}

fn px(style: &web_sys::CssStyleDeclaration, property: &str) -> f64 {
    style
        .get_property_value(property)
        .expect("read inline position")
        .trim_end_matches("px")
        .parse()
        .expect("pixel value")
}

#[dialog_common::test]
async fn drawer_cycles_keep_the_header_at_each_corner() {
    tonk_fab::register();
    let document = window().unwrap().document().unwrap();
    for (horizontal, vertical) in [
        ("left", "top"),
        ("right", "top"),
        ("left", "bottom"),
        ("right", "bottom"),
    ] {
        let fab = document
            .create_element("tonk-fab")
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap();
        fab.set_attribute("label", "Corner anchor").unwrap();
        document.body().unwrap().append_child(&fab).unwrap();
        yield_for(30).await;
        fab.style().set_property(horizontal, "16px").unwrap();
        fab.style()
            .set_property(
                if horizontal == "left" {
                    "right"
                } else {
                    "left"
                },
                "auto",
            )
            .unwrap();
        fab.style().set_property(vertical, "16px").unwrap();
        fab.style()
            .set_property(if vertical == "top" { "bottom" } else { "top" }, "auto")
            .unwrap();
        let root = fab.shadow_root().unwrap();
        wait_for_corner_settled(&fab, &root, false).await;
        root.query_selector(".space")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        wait_for_corner_settled(&fab, &root, false).await;
        let header = root.query_selector(".header").unwrap().unwrap();
        let initial = header.get_bounding_client_rect();
        let (left, right, top, bottom) = (
            initial.left(),
            initial.right(),
            initial.top(),
            initial.bottom(),
        );
        let agent = root
            .query_selector(".agent")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap();
        let assert_header_seat = |stage: &str, anchored_edges_only: bool| {
            let rect = header.get_bounding_client_rect();
            for (edge, start, end) in [
                ("left", left, rect.left()),
                ("right", right, rect.right()),
                ("top", top, rect.top()),
                ("bottom", bottom, rect.bottom()),
            ] {
                if anchored_edges_only && edge != horizontal && edge != vertical {
                    continue;
                }
                assert!(
                    (start - end).abs() < 0.75,
                    "{horizontal}/{vertical} {stage}: {edge} moved from {start} to {end}"
                );
            }
        };
        for cycle in 0..3 {
            agent.click();
            wait_for_corner_settled(&fab, &root, true).await;
            assert_header_seat(&format!("cycle {cycle} opened"), true);
            agent.click();
            wait_for_corner_settled(&fab, &root, false).await;
            assert_header_seat(&format!("cycle {cycle} closed"), false);
        }
        fab.remove();
    }
}

#[dialog_common::test]
async fn an_edge_docked_open_panel_stays_inside_the_viewport() {
    tonk_fab::register();
    let win = window().expect("window");
    let document = win.document().expect("document");
    let vw = win.inner_width().unwrap().as_f64().unwrap();
    let vh = win.inner_height().unwrap().as_f64().unwrap();
    for (x, y, bottom) in [(vw * 0.78, 40.0, false), (vw * 0.22, vh - 40.0, true)] {
        let fab = document
            .create_element("tonk-fab")
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap();
        fab.set_attribute("label", "Viewport fit").unwrap();
        document.body().unwrap().append_child(&fab).unwrap();
        yield_for(30).await;
        let root = fab.shadow_root().unwrap();
        let circle = root.query_selector(".fab").unwrap().unwrap();
        let circle_rect = circle.get_bounding_client_rect();
        circle
            .dispatch_event(&pointer_event(
                "pointerdown",
                circle_rect.left() + circle_rect.width() / 2.0,
                circle_rect.top() + circle_rect.height() / 2.0,
                1,
            ))
            .unwrap();
        win.dispatch_event(&pointer_event("pointermove", x, y, 1))
            .unwrap();
        win.dispatch_event(&pointer_event("pointerup", x, y, 0))
            .unwrap();
        yield_for(500).await;
        assert_eq!(fab.has_attribute("up"), bottom);
        root.query_selector(".space")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        root.query_selector(".agent")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        yield_for(450).await;
        let outer = fab.get_bounding_client_rect();
        let header = root
            .query_selector(".header")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect();
        let actions = root
            .query_selector(".run")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect();
        assert_eq!(fab.has_attribute("up"), bottom);
        if bottom {
            assert!(
                actions.bottom() <= header.top() + 1.0,
                "bottom seat must open upward"
            );
        } else {
            assert!(
                actions.top() >= header.bottom() - 1.0,
                "top seat must open downward"
            );
        }
        assert!(outer.left() >= 15.0, "left overflow: {}", outer.left());
        assert!(
            outer.right() <= vw - 15.0,
            "right overflow: {}",
            outer.right()
        );
        assert!(outer.top() >= 15.0, "top overflow: {}", outer.top());
        assert!(
            outer.bottom() <= vh - 15.0,
            "bottom overflow: {}",
            outer.bottom()
        );
        fab.remove();
    }
}

#[dialog_common::test]
async fn release_glides_to_the_nearest_edge_without_losing_its_free_coordinate() {
    tonk_fab::register();
    let document = window().expect("window").document().expect("document");
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

    let snapped = Rc::new(RefCell::new(None));
    let sink = snapped.clone();
    let on_snap = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
        let detail = event.detail();
        let edge = js_sys::Reflect::get(&detail, &"edge".into())
            .ok()
            .and_then(|value| value.as_string());
        *sink.borrow_mut() = edge;
    });
    fab.add_event_listener_with_callback("fabb-snap", on_snap.as_ref().unchecked_ref())
        .expect("listen for snap");

    let circle = fab
        .shadow_root()
        .expect("shadow root")
        .query_selector(".fab")
        .expect("query handle")
        .expect("handle");
    let handle = circle.get_bounding_client_rect();
    let down_x = handle.left() + handle.width() / 2.0;
    let down_y = handle.top() + handle.height() / 2.0;
    circle
        .dispatch_event(&pointer_event("pointerdown", down_x, down_y, 1))
        .expect("pointer down");

    // Put the handle near the left edge but halfway down the viewport. The
    // old four-corner behavior rewrote this y coordinate to 16px.
    let target_y = window()
        .expect("window")
        .inner_height()
        .expect("inner height")
        .as_f64()
        .expect("numeric height")
        / 2.0;
    window()
        .expect("window")
        .dispatch_event(&pointer_event("pointermove", 80.0, target_y, 1))
        .expect("pointer move");
    window()
        .expect("window")
        .dispatch_event(&pointer_event("pointerup", 80.0, target_y, 0))
        .expect("pointer up");

    assert_eq!(snapped.borrow().as_deref(), Some("left"));
    assert_eq!(px(&fab.style(), "left"), 16.0);
    let top = px(&fab.style(), "top");
    let half_height = fab.get_bounding_client_rect().height() / 2.0;
    assert!(
        (top - (target_y - half_height)).abs() < 1.0,
        "the release must keep its free y coordinate: expected about {}, got {top}",
        target_y - half_height
    );

    fab.remove();
    drop(on_snap);
}

#[dialog_common::test]
async fn a_touch_tap_expands_but_a_nine_pixel_drag_preserves_the_collapsed_atom() {
    tonk_fab::register();
    let win = window().expect("window");
    let document = win.document().expect("document");
    let parent = document
        .create_element("div")
        .expect("create parent")
        .dyn_into::<HtmlElement>()
        .expect("html parent");
    parent
        .style()
        .set_property("width", "375px")
        .expect("parent width");
    let fab = document
        .create_element("tonk-fab")
        .expect("create fab")
        .dyn_into::<HtmlElement>()
        .expect("html fab");
    parent.append_child(&fab).expect("mount fab");
    document
        .body()
        .expect("body")
        .append_child(&parent)
        .expect("mount parent");
    yield_for(50).await;

    let shadow = fab.shadow_root().expect("shadow root");
    let circle = shadow
        .query_selector(".fab")
        .expect("circle selector")
        .expect("circle")
        .unchecked_into::<HtmlElement>();
    let collapse = || circle.click();
    collapse();
    yield_for(220).await;
    let wrapper = shadow
        .query_selector(".w")
        .expect("wrapper selector")
        .expect("wrapper");
    assert!(wrapper.class_list().contains("collapsed"));

    let rect = circle.get_bounding_client_rect();
    let x = rect.left() + rect.width() / 2.0;
    let y = rect.top() + rect.height() / 2.0;
    circle
        .dispatch_event(&pointer_event_with_type("pointerdown", x, y, 1, "touch"))
        .expect("tap down");
    win.dispatch_event(&pointer_event_with_type("pointerup", x, y, 0, "touch"))
        .expect("tap up");
    circle.click();
    assert!(!wrapper.class_list().contains("collapsed"));

    collapse();
    yield_for(220).await;
    let snaps = Rc::new(RefCell::new(0_u32));
    let sink = snaps.clone();
    let on_snap = Closure::<dyn FnMut(CustomEvent)>::new(move |_| {
        *sink.borrow_mut() += 1;
    });
    fab.add_event_listener_with_callback("fabb-snap", on_snap.as_ref().unchecked_ref())
        .expect("listen for snap");

    let rect = circle.get_bounding_client_rect();
    let x = rect.left() + rect.width() / 2.0;
    let y = rect.top() + rect.height() / 2.0;
    circle
        .dispatch_event(&pointer_event_with_type("pointerdown", x, y, 1, "touch"))
        .expect("drag down");
    win.dispatch_event(&pointer_event_with_type(
        "pointermove",
        x + 9.0,
        y,
        1,
        "touch",
    ))
    .expect("drag move");
    win.dispatch_event(&pointer_event_with_type(
        "pointerup",
        x + 9.0,
        y,
        0,
        "touch",
    ))
    .expect("drag up");
    circle.click();

    assert!(wrapper.class_list().contains("collapsed"));
    assert_eq!(*snaps.borrow(), 1);
    assert!(!fab.has_attribute("collapsed"));

    parent.remove();
    drop(on_snap);
}

#[dialog_common::test]
async fn shift_space_and_a_500ms_hold_dispatch_pause_without_toggling_collapse() {
    tonk_fab::register();
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = calls.clone();
    let transact = Closure::<dyn FnMut(JsValue)>::new(move |request| {
        sink.borrow_mut().push(
            js_sys::JSON::stringify(&request)
                .map(String::from)
                .unwrap_or_default(),
        );
    });
    let win = window().expect("window");
    let tonk = js_sys::Object::new();
    js_sys::Reflect::set(&tonk, &"transact".into(), transact.as_ref()).unwrap();
    js_sys::Reflect::set(&win, &"tonk".into(), &tonk).unwrap();

    let document = win.document().expect("document");
    let fab = document
        .create_element("tonk-fab")
        .unwrap()
        .dyn_into::<HtmlElement>()
        .unwrap();
    fab.set_attribute("space", "did:key:zPauseSpace").unwrap();
    document.body().unwrap().append_child(&fab).unwrap();
    let root = fab.shadow_root().unwrap();
    let circle = root
        .query_selector(".fab")
        .unwrap()
        .unwrap()
        .unchecked_into::<HtmlElement>();
    let wrapper = root.query_selector(".w").unwrap().unwrap();

    let init = KeyboardEventInit::new();
    init.set_key(" ");
    init.set_shift_key(true);
    circle
        .dispatch_event(
            &KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
    assert_eq!(calls.borrow().len(), 1);

    let rect = circle.get_bounding_client_rect();
    let x = rect.left() + rect.width() / 2.0;
    let y = rect.top() + rect.height() / 2.0;
    circle
        .dispatch_event(&pointer_event_with_type("pointerdown", x, y, 1, "touch"))
        .unwrap();
    yield_for(550).await;
    win.dispatch_event(&pointer_event_with_type("pointerup", x, y, 0, "touch"))
        .unwrap();
    circle.click();

    assert_eq!(calls.borrow().len(), 2);
    assert!(calls.borrow().iter().all(|request| {
        request.contains("xyz.tonk.pause-sync/space") && request.contains("did:key:zPauseSpace")
    }));
    assert!(!wrapper.class_list().contains("collapsed"));

    fab.remove();
    let _ = js_sys::Reflect::delete_property(win.unchecked_ref::<js_sys::Object>(), &"tonk".into());
    drop(transact);
}
