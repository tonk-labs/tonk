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
use web_sys::{CustomEvent, Event, HtmlElement, window};

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

fn px(style: &web_sys::CssStyleDeclaration, property: &str) -> f64 {
    style
        .get_property_value(property)
        .expect("read inline position")
        .trim_end_matches("px")
        .parse()
        .expect("pixel value")
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
    assert!(
        (top - (target_y - 18.0)).abs() < 1.0,
        "the release must keep its free y coordinate: expected about {}, got {top}",
        target_y - 18.0
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
    assert!(wrapper.class_list().contains("compact-collapsed"));

    let rect = circle.get_bounding_client_rect();
    let x = rect.left() + rect.width() / 2.0;
    let y = rect.top() + rect.height() / 2.0;
    circle
        .dispatch_event(&pointer_event_with_type("pointerdown", x, y, 1, "touch"))
        .expect("tap down");
    win.dispatch_event(&pointer_event_with_type("pointerup", x, y, 0, "touch"))
        .expect("tap up");
    circle.click();
    assert!(!wrapper.class_list().contains("compact-collapsed"));

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

    assert!(wrapper.class_list().contains("compact-collapsed"));
    assert_eq!(*snaps.borrow(), 1);
    assert!(!fab.has_attribute("collapsed"));

    parent.remove();
    drop(on_snap);
}
