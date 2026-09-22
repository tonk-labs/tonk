//! Real-browser coverage for the worker-backed attached agent panel.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

fn mount() -> HtmlElement {
    tonk_fab::register();
    let document = window().unwrap().document().unwrap();
    let bar: HtmlElement = document
        .create_element("tonk-fab")
        .unwrap()
        .unchecked_into();
    bar.set_attribute("space", "did:key:zAgentSpace").unwrap();
    bar.set_attribute("label", "Project Atlas").unwrap();
    document.body().unwrap().append_child(&bar).unwrap();
    bar
}

fn shadow(bar: &HtmlElement, selector: &str) -> Element {
    bar.shadow_root()
        .unwrap()
        .query_selector(selector)
        .unwrap()
        .unwrap()
}

fn deliver_reset(agent: &HtmlElement, status: &str, link: &str) {
    let row = js_sys::JSON::parse(
        &serde_json::json!({
            "this": "did:key:zAgentSpace",
            "fields": { "status": status, "link": link, "account": "did:key:account" }
        })
        .to_string(),
    )
    .unwrap();
    let rows = Array::new();
    rows.push(&row);
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &"tonk-agent-panel".into()).unwrap();
    Reflect::get(agent, &"reset".into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(agent, &rows, &opts)
        .unwrap();
}

fn clear_tonk() {
    let win = window().unwrap();
    let _ = Reflect::delete_property(win.unchecked_ref::<Object>(), &"tonk".into());
}

#[dialog_common::test]
async fn explicit_open_mints_once_and_a_ready_frame_renders_the_complete_prompt() {
    let calls = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = calls.clone();
    let transact = Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |request| {
        sink.borrow_mut().push(
            js_sys::JSON::stringify(&request)
                .map(String::from)
                .unwrap_or_default(),
        );
    });
    let tonk = Object::new();
    Reflect::set(&tonk, &"transact".into(), transact.as_ref()).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    let agent_button = shadow(&bar, ".agent").unchecked_into::<HtmlElement>();
    agent_button.click();
    agent_button.click();
    agent_button.click();
    assert_eq!(
        calls.borrow().len(),
        1,
        "an in-flight mint is not duplicated"
    );

    let agent: HtmlElement = bar
        .query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into();
    let bearer = "https://example.test/#tonk-agent-v2=complete-secret";
    deliver_reset(&agent, "ready", bearer);
    let prompt = shadow(&bar, "#agent-panel .panel-copytext")
        .text_content()
        .unwrap_or_default();
    assert!(prompt.contains(bearer), "the bearer is never truncated");
    assert!(prompt.contains("Agent connection confirmed"));
    assert!(!shadow(&bar, "#agent-panel .panel-copy").has_attribute("hidden"));

    bar.remove();
    clear_tonk();
    drop(transact);
}

#[dialog_common::test]
async fn an_account_refusal_uses_the_typed_task_and_retains_the_space() {
    let task_payload = Rc::new(RefCell::new(None::<String>));
    let sink = task_payload.clone();
    let task = Closure::<dyn FnMut(String)>::new(move |payload| {
        *sink.borrow_mut() = Some(payload);
    });
    let transact = Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(|_| {});
    let tonk = Object::new();
    Reflect::set(&tonk, &"transact".into(), transact.as_ref()).unwrap();
    Reflect::set(&tonk, &"task".into(), task.as_ref()).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    let agent: HtmlElement = bar
        .query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into();
    deliver_reset(
        &agent,
        "create an account or sign in to invite an agent",
        "",
    );
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent-retry")
        .unchecked_into::<HtmlElement>()
        .click();

    let payload = task_payload.borrow().clone().expect("typed task request");
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["purpose"], "account");
    assert_eq!(value["account"]["reason"], "agent-invite-account");
    assert_eq!(value["account"]["space"], "did:key:zAgentSpace");
    assert!(bar.has_attribute("data-task-hosted"));

    bar.remove();
    clear_tonk();
    drop(task);
    drop(transact);
}
