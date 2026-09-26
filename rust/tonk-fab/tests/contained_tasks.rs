//! The contained FABB task owns one request and restores the exact surface.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, Event, EventInit, HtmlDialogElement, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
}

fn mount_bar() -> HtmlElement {
    tonk_fab::register();
    let bar = document()
        .create_element("tonk-fab")
        .expect("create bar")
        .dyn_into::<HtmlElement>()
        .expect("HTML bar");
    bar.set_attribute("label", "Project").expect("label");
    document()
        .body()
        .expect("body")
        .append_child(&bar)
        .expect("mount bar");
    bar
}

fn shadow(bar: &HtmlElement, selector: &str) -> Element {
    bar.shadow_root()
        .expect("shadow root")
        .query_selector(selector)
        .expect("selector")
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn method(host: &HtmlElement, name: &str) -> Function {
    Reflect::get(host, &JsValue::from_str(name))
        .unwrap_or_else(|_| panic!("read {name}"))
        .dyn_into()
        .unwrap_or_else(|_| panic!("{name} function"))
}

fn present(bar: &HtmlElement, content: &HtmlElement, heading: &str, dismissible: bool) -> Promise {
    let options = Object::new();
    Reflect::set(&options, &"heading".into(), &heading.into()).expect("heading");
    Reflect::set(
        &options,
        &"dismissible".into(),
        &JsValue::from_bool(dismissible),
    )
    .expect("dismissible");
    method(bar, "present")
        .call2(bar, content, &options)
        .expect("present call")
        .dyn_into()
        .expect("present promise")
}

fn notify(bar: &HtmlElement, message: &str, heading: &str) -> Promise {
    let options = Object::new();
    Reflect::set(&options, &"heading".into(), &heading.into()).expect("heading");
    method(bar, "notify")
        .call2(bar, &message.into(), &options)
        .expect("notify call")
        .dyn_into()
        .expect("notify promise")
}

fn requesting(bar: &HtmlElement) -> bool {
    Reflect::get(bar, &"requesting".into())
        .expect("requesting")
        .as_bool()
        .expect("boolean requesting")
}

fn request_id(bar: &HtmlElement) -> f64 {
    Reflect::get(bar, &"requestId".into())
        .expect("request ID")
        .as_f64()
        .expect("numeric request ID")
}

fn decision() -> HtmlElement {
    let content = document()
        .create_element("section")
        .expect("content")
        .dyn_into::<HtmlElement>()
        .expect("HTML content");
    content.set_inner_html(
        r#"<p>Connect this space?</p>
           <div data-fabb-actions>
             <button data-fabb-result="cancel">keep it here</button>
             <button data-fabb-result="continue">connect</button>
           </div>"#,
    );
    content
}

fn cancel_event() -> Event {
    let init = EventInit::new();
    init.set_bubbles(false);
    init.set_cancelable(true);
    Event::new_with_event_init_dict("cancel", &init).expect("cancel event")
}

#[dialog_common::test]
async fn a_decision_restores_its_open_panel_origin_and_focus() {
    let bar = mount_bar();
    method(&bar, "open")
        .call1(&bar, &"space".into())
        .expect("open space panel");
    assert_eq!(
        shadow(&bar, "[data-cell=space]").get_attribute("aria-expanded"),
        Some("true".into())
    );

    let opener = document()
        .create_element("button")
        .expect("opener")
        .dyn_into::<HtmlElement>()
        .expect("HTML opener");
    opener.set_text_content(Some("open task"));
    document()
        .body()
        .expect("body")
        .append_child(&opener)
        .expect("mount opener");
    opener.focus().expect("focus opener");

    let origin = document().create_element("div").expect("origin");
    let content = decision();
    content.set_attribute("slot", "original").expect("slot");
    content.set_hidden(true);
    origin.append_child(&content).expect("content in origin");
    document()
        .body()
        .expect("body")
        .append_child(&origin)
        .expect("mount origin");

    let result = present(&bar, &content, "connect this space", true);
    assert!(requesting(&bar));
    assert!(
        shadow(&bar, ".request-layer")
            .dyn_ref::<HtmlDialogElement>()
            .is_some_and(HtmlDialogElement::open),
        "a native modal owns the request"
    );
    let primary = content
        .query_selector("[data-fabb-result=continue]")
        .expect("primary selector")
        .expect("primary")
        .dyn_into::<HtmlElement>()
        .expect("HTML primary");
    primary.click();
    let result = JsFuture::from(result).await.expect("decision result");

    assert_eq!(result.as_string().as_deref(), Some("continue"));
    assert!(!requesting(&bar));
    assert!(
        content
            .parent_node()
            .is_some_and(|parent| parent.is_same_node(Some(origin.unchecked_ref())))
    );
    assert_eq!(content.get_attribute("slot").as_deref(), Some("original"));
    assert!(content.hidden());
    assert_eq!(
        shadow(&bar, "[data-cell=space]").get_attribute("aria-expanded"),
        Some("true".into()),
        "the exact panel remains open behind the task"
    );
    assert!(
        document()
            .active_element()
            .is_some_and(|active| active.is_same_node(Some(&opener))),
        "focus returns to the deepest connected opener"
    );
    bar.remove();
    opener.remove();
    origin.remove();
}

#[dialog_common::test]
async fn a_required_notification_ignores_escape_and_backdrop_until_acknowledged() {
    let bar = mount_bar();
    let result = notify(&bar, "Your agent connected.", "connect agent");
    let dialog = shadow(&bar, ".request-layer")
        .dyn_into::<HtmlDialogElement>()
        .expect("native dialog");

    assert!(
        !dialog
            .dispatch_event(&cancel_event())
            .expect("cancel event")
    );
    assert!(requesting(&bar), "Escape cannot dismiss required feedback");
    dialog.click();
    assert!(requesting(&bar), "the backdrop never acknowledges a task");

    shadow(&bar, ".task-ack")
        .dyn_into::<HtmlElement>()
        .expect("ack button")
        .click();
    let result = JsFuture::from(result).await.expect("notification result");
    assert_eq!(result.as_string().as_deref(), Some("acknowledged"));
    assert!(!requesting(&bar));
    bar.remove();
}

#[dialog_common::test]
async fn a_second_request_is_busy_and_a_stale_result_cannot_finish_the_next_one() {
    let bar = mount_bar();
    let first_content = decision();
    let first = present(&bar, &first_content, "first", true);
    let first_id = request_id(&bar);

    let rejected_content = decision();
    let rejected = present(&bar, &rejected_content, "replacement", true);
    assert!(
        JsFuture::from(rejected).await.is_err(),
        "a second request returns busy instead of replacing the first"
    );

    method(&bar, "resolve")
        .call1(&bar, &"cancelled".into())
        .expect("resolve first");
    assert_eq!(
        JsFuture::from(first)
            .await
            .expect("first result")
            .as_string()
            .as_deref(),
        Some("cancelled")
    );

    let second_content = decision();
    let second = present(&bar, &second_content, "second", true);
    let second_id = request_id(&bar);
    assert_ne!(first_id, second_id);
    method(&bar, "resolveRequest")
        .call2(&bar, &first_id.into(), &"stale".into())
        .expect("stale resolve");
    assert!(requesting(&bar));
    assert_eq!(request_id(&bar), second_id);

    method(&bar, "resolveRequest")
        .call2(&bar, &second_id.into(), &"complete".into())
        .expect("resolve second");
    assert_eq!(
        JsFuture::from(second)
            .await
            .expect("second result")
            .as_string()
            .as_deref(),
        Some("complete")
    );
    bar.remove();
}

#[dialog_common::test]
async fn disconnect_releases_the_modal_and_restores_moved_content() {
    let bar = mount_bar();
    let origin = document().create_element("div").expect("origin");
    let content = decision();
    origin.append_child(&content).expect("content in origin");
    document()
        .body()
        .expect("body")
        .append_child(&origin)
        .expect("mount origin");
    let result = present(&bar, &content, "disconnect", true);

    bar.remove();
    let result = JsFuture::from(result).await.expect("disconnect result");
    assert_eq!(result.as_string().as_deref(), Some("disconnected"));
    assert!(
        content
            .parent_node()
            .is_some_and(|parent| parent.is_same_node(Some(origin.unchecked_ref())))
    );
    assert!(
        !document()
            .query_selector(":modal")
            .expect("modal selector")
            .is_some(),
        "disconnect closes the native modal"
    );
    origin.remove();
}
