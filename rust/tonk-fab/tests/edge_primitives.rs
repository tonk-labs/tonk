//! Edge-grammar components in a real browser DOM.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{
    CustomEvent, Element, HtmlElement, HtmlInputElement, MouseEvent, MouseEventInit, window,
};

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    window().expect("window").document().expect("document")
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

fn keyboard_event(key: &str, shift: bool) -> web_sys::Event {
    let init = js_sys::Object::new();
    js_sys::Reflect::set(&init, &"bubbles".into(), &JsValue::TRUE).expect("bubbles");
    js_sys::Reflect::set(&init, &"composed".into(), &JsValue::TRUE).expect("composed");
    js_sys::Reflect::set(&init, &"key".into(), &key.into()).expect("key");
    js_sys::Reflect::set(&init, &"shiftKey".into(), &JsValue::from_bool(shift)).expect("shift key");
    let constructor = js_sys::Reflect::get(&window().expect("window"), &"KeyboardEvent".into())
        .expect("KeyboardEvent constructor")
        .dyn_into::<js_sys::Function>()
        .expect("KeyboardEvent function");
    let args = js_sys::Array::new();
    args.push(&"keydown".into());
    args.push(&init);
    js_sys::Reflect::construct(&constructor, &args)
        .expect("construct keyboard event")
        .dyn_into::<web_sys::Event>()
        .expect("keyboard event")
}

fn input_event() -> web_sys::Event {
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    web_sys::Event::new_with_event_init_dict("input", &init).expect("input event")
}

fn shadow(host: &HtmlElement, selector: &str) -> Element {
    host.shadow_root()
        .expect("shadow root")
        .query_selector(selector)
        .expect("selector")
        .unwrap_or_else(|| panic!("missing {selector}"))
}

fn mount(tag: &str) -> HtmlElement {
    tonk_fab::register();
    let host = document()
        .create_element(tag)
        .expect("create host")
        .dyn_into::<HtmlElement>()
        .expect("html host");
    document()
        .body()
        .expect("body")
        .append_child(&host)
        .expect("mount host");
    host
}

#[dialog_common::test]
fn a_space_typed_while_renaming_does_not_commit_the_name() {
    let bar = mount("tonk-fab");
    bar.set_attribute("label", "Project").expect("label");
    let edit_space = js_sys::Reflect::get(&bar, &"editSpace".into())
        .expect("editSpace member")
        .dyn_into::<js_sys::Function>()
        .expect("editSpace function");
    edit_space.call0(&bar).expect("start rename");

    let cell = shadow(&bar, "[data-cell=space]");
    let edit = shadow(&bar, ".space .edit");
    edit.set_text_content(Some("Project "));

    // Browsers synthesize a detail-zero click on a focused button for the
    // Space key. The editable lives inside that button, so this is the click
    // that used to commit the name at its first word.
    let init = MouseEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    let click = MouseEvent::new_with_mouse_event_init_dict("click", &init).expect("keyboard click");
    cell.dispatch_event(&click)
        .expect("dispatch keyboard click");

    assert!(
        cell.class_list().contains("editing"),
        "a Space-key click must leave the rename active",
    );
    assert_eq!(edit.text_content().as_deref(), Some("Project "));
    bar.remove();
}

fn set_context_origin(value: &str) {
    let win = window().expect("window");
    let tonk = js_sys::Reflect::get(&win, &"tonk".into())
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| js_sys::Object::new().into());
    let context = js_sys::Object::new();
    js_sys::Reflect::set(&context, &"origin".into(), &value.into()).expect("context origin");
    js_sys::Reflect::set(&tonk, &"context".into(), &context).expect("context");
    js_sys::Reflect::set(&win, &"tonk".into(), &tonk).expect("tonk");
}

#[dialog_common::test]
async fn field_filters_commits_rejects_and_changes_its_noun() {
    let host = mount("tonk-field");
    host.set_attribute("noun", "activation code").expect("noun");
    host.set_attribute("value", "").expect("value");
    host.set_attribute("filter", "digits").expect("filter");
    host.set_attribute("autolen", "6").expect("autolen");
    host.set_attribute("changeable", "").expect("changeable");

    let commits = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = commits.clone();
    let on_commit = Closure::<dyn FnMut(CustomEvent)>::new(move |event: CustomEvent| {
        let value = js_sys::Reflect::get(&event.detail(), &"value".into())
            .expect("detail value")
            .as_string()
            .expect("string value");
        sink.borrow_mut().push(value);
    });
    host.add_event_listener_with_callback("fabb-commit", on_commit.as_ref().unchecked_ref())
        .expect("commit listener");

    let changes = Rc::new(Cell::new(0));
    let sink = changes.clone();
    let on_change = Closure::<dyn FnMut(CustomEvent)>::new(move |_| sink.set(sink.get() + 1));
    host.add_event_listener_with_callback("fabb-change-noun", on_change.as_ref().unchecked_ref())
        .expect("change listener");

    let input = shadow(&host, ".value")
        .dyn_into::<HtmlInputElement>()
        .expect("value input");
    input.set_value("1a2b3");
    input
        .dispatch_event(&input_event())
        .expect("dispatch input");
    assert_eq!(input.value(), "123");
    assert!(commits.borrow().is_empty(), "short value does not commit");

    input.set_value("12x3456");
    input
        .dispatch_event(&input_event())
        .expect("dispatch input");
    assert_eq!(commits.borrow().as_slice(), &["123456"]);

    input.set_value("765432");
    input
        .dispatch_event(&keyboard_event("Enter", false))
        .expect("dispatch Enter");
    assert_eq!(commits.borrow().last().map(String::as_str), Some("765432"));

    let reject = js_sys::Reflect::get(&host, &"reject".into())
        .expect("reject member")
        .dyn_into::<js_sys::Function>()
        .expect("reject function");
    reject.call0(&host).expect("reject call");
    assert_eq!(input.selection_start().expect("selection start"), Some(0));
    assert_eq!(input.selection_end().expect("selection end"), Some(6));
    assert!(shadow(&host, ".row").class_list().contains("rejecting"));

    shadow(&host, ".noun")
        .dyn_into::<HtmlElement>()
        .expect("noun control")
        .click();
    assert_eq!(changes.get(), 1);

    host.remove();
    drop(on_commit);
    drop(on_change);
}

#[dialog_common::test]
async fn cluster_bails_only_from_escape_or_ghost_and_loops_focus() {
    let host = mount("tonk-cluster");
    host.set_inner_html(
        r#"<p slot="statement">connect this space</p>
           <button id="first" slot="run">quiet</button>
           <button id="last" slot="run">solid</button>
           <span slot="ghost">keep it here</span>"#,
    );
    yield_for(0).await;

    let bails = Rc::new(Cell::new(0));
    let sink = bails.clone();
    let on_bail = Closure::<dyn FnMut(CustomEvent)>::new(move |_| sink.set(sink.get() + 1));
    host.add_event_listener_with_callback("fabb-bail", on_bail.as_ref().unchecked_ref())
        .expect("bail listener");

    shadow(&host, ".dim")
        .dyn_into::<HtmlElement>()
        .expect("dim")
        .click();
    assert_eq!(bails.get(), 0, "the dim is inert");

    host.dispatch_event(&keyboard_event("Escape", false))
        .expect("dispatch Escape");
    assert_eq!(bails.get(), 1);

    shadow(&host, ".ghost")
        .dyn_into::<HtmlElement>()
        .expect("ghost")
        .click();
    assert_eq!(bails.get(), 2);

    let first = host
        .query_selector("#first")
        .expect("first selector")
        .expect("first")
        .dyn_into::<HtmlElement>()
        .expect("first html");
    let ghost = shadow(&host, ".ghost")
        .dyn_into::<HtmlElement>()
        .expect("ghost html");
    ghost.focus().expect("focus ghost");
    ghost
        .dispatch_event(&keyboard_event("Tab", false))
        .expect("dispatch Tab");
    assert!(
        document()
            .active_element()
            .is_some_and(|active| active.is_same_node(Some(&first))),
        "Tab from the last focusable loops to the first"
    );

    host.remove();
    drop(on_bail);
}

#[dialog_common::test]
async fn banner_beats_opens_and_retires() {
    let host = mount("tonk-banner");
    host.set_inner_html("connect this space<span slot=door>connect</span>");

    assert!(!shadow(&host, ".w").class_list().contains("live"));
    yield_for(500).await;
    assert!(shadow(&host, ".w").class_list().contains("live"));

    let opens = Rc::new(Cell::new(0));
    let sink = opens.clone();
    let on_open = Closure::<dyn FnMut(CustomEvent)>::new(move |_| sink.set(sink.get() + 1));
    host.add_event_listener_with_callback("fabb-open", on_open.as_ref().unchecked_ref())
        .expect("open listener");
    shadow(&host, ".door")
        .dyn_into::<HtmlElement>()
        .expect("door")
        .click();
    assert_eq!(opens.get(), 1);

    let retire = js_sys::Reflect::get(&host, &"retire".into())
        .expect("retire member")
        .dyn_into::<js_sys::Function>()
        .expect("retire function");
    retire.call0(&host).expect("retire call");
    yield_for(200).await;
    assert!(!host.is_connected());

    drop(on_open);
}

#[dialog_common::test]
async fn local_only_bar_opens_the_shared_connect_ceremony() {
    set_context_origin("https://local.tonk.test");
    let bar = mount("tonk-fab");
    bar.set_attribute("space", "did:key:zLocal").expect("space");
    bar.set_attribute("state", "offline")
        .expect("offline state");
    bar.set_attribute("data-sync-status", "sync:local")
        .expect("precise status");
    yield_for(0).await;

    let banner = document()
        .get_element_by_id("fabb-connect-banner")
        .expect("local-only banner")
        .dyn_into::<HtmlElement>()
        .expect("banner html");
    assert_eq!(
        banner.text_content().as_deref(),
        Some("connect this spaceconnect")
    );

    shadow(&banner, ".door")
        .dyn_into::<HtmlElement>()
        .expect("banner door")
        .click();
    let cluster = document()
        .get_element_by_id("fabb-connect-cluster")
        .expect("connect ceremony");
    assert!(!cluster.has_attribute("hidden"));
    assert!(banner.has_attribute("hidden"));
    // The field stands but names nothing. The bar used to derive the
    // sync endpoint itself and prefill it here, which asked a sealed
    // guest for an origin it does not have — its document is
    // `about:srcdoc`, so `location.origin` is the opaque "null" until
    // the bridge injects the real one, and a share dispatched before
    // that arrived returned having done nothing. The worker resolves
    // where a space syncs from the account's own registration now, so
    // there is no address for the page to fill in.
    let field = cluster
        .query_selector("[data-enable-sync-remote]")
        .expect("remote selector")
        .expect("remote field");
    assert_eq!(
        field.get_attribute("value").as_deref(),
        Some(""),
        "the page names no endpoint; the worker resolves it",
    );

    shadow(
        &cluster
            .clone()
            .dyn_into::<HtmlElement>()
            .expect("cluster html"),
        ".ghost",
    )
    .dyn_into::<HtmlElement>()
    .expect("ghost")
    .click();
    assert!(cluster.has_attribute("hidden"));
    assert!(!banner.has_attribute("hidden"));

    bar.remove();
    cluster.remove();
}
