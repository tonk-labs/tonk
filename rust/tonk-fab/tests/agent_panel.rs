//! Real-browser coverage for the worker-backed attached agent panel.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

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
    deliver_reset_with_mode(agent, "", status, link);
}

fn deliver_reset_with_mode(agent: &HtmlElement, mode: &str, status: &str, link: &str) {
    let row = js_sys::JSON::parse(
        &serde_json::json!({
            "this": "did:key:zAgentSpace",
            "fields": { "mode": mode, "status": status, "link": link, "account": "did:key:account" }
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

#[dialog_common::test]
async fn busy_handoff_frame_keeps_the_agent_action_pending() {
    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
    let agent: HtmlElement = bar
        .query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into();
    deliver_reset_with_mode(&agent, "busy", "creating agent invitation…", "");
    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("creating an agent invitation…")
    );
    assert!(shadow(&bar, ".agent-retry").has_attribute("hidden"));

    deliver_reset_with_mode(&agent, "retry", "agent invitation failed", "");
    assert!(!shadow(&bar, ".agent-retry").has_attribute("hidden"));

    bar.remove();
}

fn clear_tonk() {
    let win = window().unwrap();
    let _ = Reflect::delete_property(win.unchecked_ref::<Object>(), &"tonk".into());
}

#[dialog_common::test]
async fn plain_account_action_does_not_request_a_share_link() {
    let payload = Rc::new(RefCell::new(None::<String>));
    let sink = payload.clone();
    let task = Closure::<dyn FnMut(String)>::new(move |value| {
        *sink.borrow_mut() = Some(value);
    });
    let tonk = Object::new();
    Reflect::set(&tonk, &"task".into(), task.as_ref()).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".login")
        .unchecked_into::<HtmlElement>()
        .click();
    let value: serde_json::Value =
        serde_json::from_str(&payload.borrow().clone().expect("account request")).unwrap();
    assert_eq!(value["account"]["reason"], "fabb-account");
    assert_eq!(value["account"]["space"], "did:key:zAgentSpace");

    bar.remove();
    clear_tonk();
    drop(task);
}

#[dialog_common::test]
async fn rejected_invitation_request_exposes_a_retry_instead_of_staying_pending() {
    let transact = Function::new_with_args(
        "request",
        "return Promise.reject(new Error('request failed'))",
    );
    let tonk = Object::new();
    Reflect::set(&tonk, &"transact".into(), &transact).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    let _ = JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)).await;
    let _ = JsFuture::from(js_sys::Promise::resolve(&wasm_bindgen::JsValue::NULL)).await;
    assert!(
        shadow(&bar, ".agent-status")
            .text_content()
            .unwrap_or_default()
            .contains("failed")
    );
    assert!(!shadow(&bar, ".agent-retry").has_attribute("hidden"));

    bar.remove();
    clear_tonk();
}

#[dialog_common::test]
async fn signed_out_agent_uses_the_account_gate_without_minting() {
    let calls = Rc::new(RefCell::new(0));
    let sink = calls.clone();
    let transact = Closure::<dyn FnMut(wasm_bindgen::JsValue)>::new(move |_| {
        *sink.borrow_mut() += 1;
    });
    let task_payload = Rc::new(RefCell::new(None::<String>));
    let sink = task_payload.clone();
    let task = Closure::<dyn FnMut(String)>::new(move |payload| {
        *sink.borrow_mut() = Some(payload);
    });
    let tonk = Object::new();
    Reflect::set(&tonk, &"transact".into(), transact.as_ref()).unwrap();
    Reflect::set(&tonk, &"task".into(), task.as_ref()).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    assert!(!shadow(&bar, "#agent-panel").has_attribute("hidden"));
    assert!(!shadow(&bar, ".agent-gate").has_attribute("hidden"));
    assert_eq!(
        *calls.borrow(),
        0,
        "opening the gate must not mint an invitation"
    );
    shadow(&bar, ".agent-continue")
        .unchecked_into::<HtmlElement>()
        .click();
    let payload = task_payload.borrow().clone().expect("account task request");
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["account"]["reason"], "agent-invite-account");
    assert_eq!(value["account"]["space"], "did:key:zAgentSpace");
    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).unwrap();
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    let event = CustomEvent::new_with_event_init_dict("tonk:task-closed", &init).unwrap();
    window().unwrap().dispatch_event(&event).unwrap();
    assert!(!shadow(&bar, "#agent-panel").has_attribute("hidden"));
    assert_eq!(*calls.borrow(), 1, "completion starts one agent invitation");

    bar.remove();
    clear_tonk();
    drop(task);
    drop(transact);
}

#[dialog_common::test]
async fn explicit_open_mints_once_and_a_ready_frame_renders_the_complete_prompt() {
    let calls = Rc::new(RefCell::new(Vec::<(String, bool)>::new()));
    let sink = calls.clone();
    let transact = Closure::<dyn FnMut(wasm_bindgen::JsValue, wasm_bindgen::JsValue)>::new(
        move |request, context: wasm_bindgen::JsValue| {
            let request = js_sys::JSON::stringify(&request)
                .map(String::from)
                .unwrap_or_default();
            sink.borrow_mut().push((request, context.is_undefined()));
        },
    );
    let tonk = Object::new();
    Reflect::set(&tonk, &"transact".into(), transact.as_ref()).unwrap();
    Reflect::set(&window().unwrap(), &"tonk".into(), &tonk).unwrap();

    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
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
    assert!(
        calls.borrow()[0].1,
        "the command must use app chrome's profile route"
    );
    let claim: serde_json::Value = serde_json::from_str(&calls.borrow()[0].0).unwrap();
    assert_eq!(
        claim["claims"][0]["application"]["parameters"]["space"], "did:key:zAgentSpace",
        "the profile-mounted FAB must name the target space"
    );
    assert_eq!(
        claim["claims"][0]["application"]["parameters"]["fresh"], "new",
        "opening Connect Agent explicitly asks for a fresh invitation"
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
    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("copy the prompt and only share the link with the agent")
    );

    bar.remove();
    clear_tonk();
    drop(transact);
}

#[dialog_common::test]
async fn reopening_after_a_lost_invite_mints_again_without_try_again() {
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
    bar.remove_attribute("data-account-required").unwrap();
    let agent: HtmlElement = bar
        .query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into();
    deliver_reset(
        &agent,
        "an invite was already issued for this space; create a new invite to continue",
        "",
    );
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();

    assert_eq!(calls.borrow().len(), 1);
    let claim: serde_json::Value = serde_json::from_str(&calls.borrow()[0]).unwrap();
    assert_eq!(
        claim["claims"][0]["application"]["parameters"]["fresh"],
        "new"
    );
    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("creating an agent invitation…")
    );

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
    bar.remove_attribute("data-account-required").unwrap();
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
