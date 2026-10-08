//! Real-browser coverage for the worker-backed attached agent panel.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{CustomEvent, CustomEventInit, Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

// In a subdirectory, so Cargo does not build it as a test suite of its own.
#[path = "support/settle.rs"]
mod settle;

#[path = "support/profile_fetch.rs"]
mod profile_fetch;

/// What `fetch` answers a claim the worker accepted.
const ACCEPTED: &str = "new Response('{}', { status: 200 })";

/// Where a bar with no `with` of its own sends its claims.
const PROFILE_TRANSACT: &str = "/api/repository/profile:tonk/branch/main/transact";

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
    deliver_handoff(agent, status, link, "");
}

fn deliver_handoff(agent: &HtmlElement, status: &str, link: &str, receipt: &str) {
    let row = js_sys::JSON::parse(
        &serde_json::json!({
            "this": "did:key:zAgentSpace",
            "fields": { "status": status, "link": link, "account": "did:key:account", "receipt": receipt }
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
    deliver_reset(&agent, "creating agent invitation…", "");
    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("creating an agent invitation…")
    );
    assert!(shadow(&bar, ".agent-retry").has_attribute("hidden"));

    deliver_reset(&agent, "agent invitation failed", "");
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
    let profile = profile_fetch::install("Promise.reject(new Error('request failed'))");

    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    assert_eq!(
        profile.requests().await.len(),
        1,
        "the invitation was requested"
    );
    assert!(
        shadow(&bar, ".agent-status")
            .text_content()
            .unwrap_or_default()
            .contains("failed")
    );
    assert!(!shadow(&bar, ".agent-retry").has_attribute("hidden"));

    bar.remove();
}

#[dialog_common::test]
async fn signed_out_agent_uses_the_account_gate_without_minting() {
    let profile = profile_fetch::install(ACCEPTED);
    let task_payload = Rc::new(RefCell::new(None::<String>));
    let sink = task_payload.clone();
    let task = Closure::<dyn FnMut(String)>::new(move |payload| {
        *sink.borrow_mut() = Some(payload);
    });
    let tonk = Object::new();
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
        profile.requests().await.len(),
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
    let agent: HtmlElement = bar
        .query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into();
    deliver_reset(&agent, "add an account to connect an agent", "");
    let detail = Object::new();
    Reflect::set(&detail, &"result".into(), &"completed".into()).unwrap();
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    let event = CustomEvent::new_with_event_init_dict("tonk:task-closed", &init).unwrap();
    window().unwrap().dispatch_event(&event).unwrap();
    assert!(!shadow(&bar, "#agent-panel").has_attribute("hidden"));
    assert_eq!(
        profile.requests().await.len(),
        1,
        "completion starts one agent invitation"
    );

    bar.remove();
    clear_tonk();
    drop(task);
}

#[dialog_common::test]
async fn explicit_open_mints_once_and_a_ready_frame_renders_the_complete_prompt() {
    let profile = profile_fetch::install(ACCEPTED);

    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    let agent_button = shadow(&bar, ".agent").unchecked_into::<HtmlElement>();
    agent_button.click();
    agent_button.click();
    agent_button.click();
    let calls = profile.requests().await;
    assert_eq!(calls.len(), 1, "an in-flight mint is not duplicated");
    assert_eq!(
        calls[0].0, PROFILE_TRANSACT,
        "the command must use app chrome's profile route"
    );
    let claim = &calls[0].1;
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
    assert!(!shadow(&bar, "#agent-panel .agent-copy-prompt").has_attribute("hidden"));
    assert!(!shadow(&bar, "#agent-panel .agent-copy-link").has_attribute("hidden"));
    let clipboard = window().unwrap().navigator().clipboard();
    let previous_write = Reflect::get(&clipboard, &"writeText".into()).unwrap();
    let copied = Rc::new(RefCell::new(String::new()));
    let copied_sink = copied.clone();
    let write = Closure::<dyn FnMut(String) -> js_sys::Promise>::new(move |text| {
        *copied_sink.borrow_mut() = text;
        js_sys::Promise::resolve(&wasm_bindgen::JsValue::UNDEFINED)
    });
    Reflect::set(&clipboard, &"writeText".into(), write.as_ref()).unwrap();
    shadow(&bar, "#agent-panel .agent-copy-link")
        .unchecked_into::<HtmlElement>()
        .click();
    assert_eq!(
        copied.borrow().as_str(),
        bearer,
        "copy link writes only the complete bearer"
    );
    Reflect::set(&clipboard, &"writeText".into(), &previous_write).unwrap();

    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("copy the link and give it to your agent")
    );

    bar.remove();
}

#[dialog_common::test]
async fn reopening_after_a_lost_invite_mints_again_without_try_again() {
    let profile = profile_fetch::install(ACCEPTED);

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

    let calls = profile.requests().await;
    assert_eq!(calls.len(), 1);
    let claim = &calls[0].1;
    assert_eq!(
        claim["claims"][0]["application"]["parameters"]["fresh"],
        "new"
    );
    assert_eq!(
        shadow(&bar, ".agent-status").text_content().as_deref(),
        Some("creating an agent invitation…")
    );

    bar.remove();
}

#[dialog_common::test]
async fn an_account_refusal_uses_the_typed_task_and_retains_the_space() {
    let task_payload = Rc::new(RefCell::new(None::<String>));
    let sink = task_payload.clone();
    let task = Closure::<dyn FnMut(String)>::new(move |payload| {
        *sink.borrow_mut() = Some(payload);
    });
    let _profile = profile_fetch::install(ACCEPTED);
    let tonk = Object::new();
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
}

fn agent_child(bar: &HtmlElement) -> HtmlElement {
    bar.query_selector("tonk-agent-panel")
        .unwrap()
        .unwrap()
        .unchecked_into()
}

fn deliver_receipts(agent: &HtmlElement, receipts: &[&str], update: bool) {
    let rows = serde_json::json!(receipts.iter().map(|receipt| {
        serde_json::json!({ "this": receipt, "fields": { "status": "Agent connection confirmed" } })
    }).collect::<Vec<_>>());
    let payload = if update {
        serde_json::json!({ "asserted": rows, "retracted": [] })
    } else {
        rows
    };
    let payload = js_sys::JSON::parse(&payload.to_string()).unwrap();
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &"tonk-agent-receipts".into()).unwrap();
    Reflect::get(agent, &if update { "update" } else { "reset" }.into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(agent, &payload, &opts)
        .unwrap();
}

fn assert_width(bar: &HtmlElement, expected: f64) {
    let width = shadow(bar, ".w").get_bounding_client_rect().width();
    assert!(
        (width - expected).abs() < 1.0,
        "FAB settles at {expected}px, not {width}px"
    );
}

fn assert_feedback_height(bar: &HtmlElement, expanded: bool) {
    let height = shadow(bar, ".agent-feedback")
        .get_bounding_client_rect()
        .height();
    assert!(
        if expanded {
            height >= 90.0
        } else {
            height < 1.0
        },
        "connection popup reaches expanded={expanded}: {height}px"
    );
}

#[dialog_common::test]
async fn connection_feedback_is_local_transient_and_restores_the_collapsed_fab() {
    let _profile = profile_fetch::install(ACCEPTED);
    let bar = mount();
    let viewer = mount();
    let agent = agent_child(&bar);
    let other = agent_child(&viewer);
    let mine = "id:tonk:agent-connection:mine";
    let bearer = "https://example.test/#tonk-agent-v2=secret";
    deliver_handoff(&agent, "ready", bearer, "old");
    shadow(&bar, ".agent-copy-link")
        .unchecked_into::<HtmlElement>()
        .click();
    // A delayed initial snapshot is still history, even after a copy click.
    deliver_receipts(&agent, &["old"], false);
    deliver_receipts(&agent, &["old"], true);
    assert!(!bar.has_attribute("data-agent-connected"));

    // A retained ready invitation works too: only the FAB where it is copied
    // arms the receipt, even when both instances receive the same handoff.
    deliver_handoff(&agent, "ready", bearer, mine);
    deliver_handoff(&other, "ready", bearer, mine);
    shadow(&bar, ".agent-copy-link")
        .unchecked_into::<HtmlElement>()
        .click();
    // This is the collapsed presentation; feedback must not change its state.
    shadow(&bar, ".w").class_list().add_1("collapsed").unwrap();
    settle::finish_animations(&bar);
    assert_width(&bar, 51.0);
    deliver_receipts(&agent, &["someone-elses"], true);
    assert!(!bar.has_attribute("data-agent-connected"));
    deliver_receipts(&other, &[mine], true);
    assert!(!viewer.has_attribute("data-agent-connected"));
    // The notice retires on its own timer; listen before it can start.
    let retired = settle::next_event(&bar, "fabb-notice-retired");
    deliver_receipts(&agent, &[mine], true);
    assert!(bar.has_attribute("data-agent-connected"));
    assert_eq!(
        shadow(&bar, ".agent-notice").text_content().as_deref(),
        Some("agent connected")
    );
    let wrapper = shadow(&bar, ".w");
    let _ = wrapper.get_bounding_client_rect();
    let animations = Reflect::get(&wrapper, &"getAnimations".into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call0(&wrapper)
        .unwrap();
    assert!(
        Array::from(&animations).length() > 0,
        "confirmation must animate the FAB width"
    );
    let expanded = 360.0_f64.min(window().unwrap().inner_width().unwrap().as_f64().unwrap() - 32.0);
    settle::finish_animations(&bar);
    assert_width(&bar, expanded);
    assert_feedback_height(&bar, true);
    assert!(
        shadow(&bar, ".w").get_bounding_client_rect().height() > 130.0,
        "the notice must pop out as a message surface, not replace the header label"
    );
    assert_eq!(
        shadow(&bar, ".space .n").text_content().as_deref(),
        Some("Project Atlas")
    );
    settle::arrived(retired, "the connection notice retires").await;
    settle::finish_animations(&bar);
    assert_width(&bar, 51.0);
    assert_feedback_height(&bar, false);
    assert!(!bar.has_attribute("data-agent-connected"));
    assert!(shadow(&bar, ".w").class_list().contains("collapsed"));
    deliver_receipts(&agent, &[mine], false);
    deliver_receipts(&agent, &[mine], true);
    assert!(
        !bar.has_attribute("data-agent-connected"),
        "reset and repeated updates do not replay"
    );

    bar.set_attribute("space", "did:key:other-space").unwrap();
    deliver_receipts(&agent, &[mine], true);
    assert!(!bar.has_attribute("data-agent-connected"));
    bar.remove();
    viewer.remove();
    let reopened = mount();
    let agent = agent_child(&reopened);
    deliver_handoff(&agent, "ready", bearer, mine);
    deliver_receipts(&agent, &[mine], false);
    assert!(!reopened.has_attribute("data-agent-connected"));
    reopened.remove();
}

#[dialog_common::test]
async fn connection_feedback_preserves_the_panel_and_clears_on_navigation() {
    let _profile = profile_fetch::install(ACCEPTED);
    let bar = mount();
    bar.remove_attribute("data-account-required").unwrap();
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".agent")
        .unchecked_into::<HtmlElement>()
        .click();
    let agent = agent_child(&bar);
    let receipt = "id:tonk:agent-connection:current";
    deliver_handoff(
        &agent,
        "ready",
        "https://example.test/#tonk-agent-v2=secret",
        receipt,
    );
    shadow(&bar, ".agent-copy-prompt")
        .unchecked_into::<HtmlElement>()
        .click();
    deliver_receipts(&agent, &[receipt], true);
    assert!(bar.has_attribute("data-agent-connected"));
    assert!(shadow(&bar, ".w").class_list().contains("has-panel"));
    assert!(!shadow(&bar, "#agent-panel").has_attribute("hidden"));
    bar.set_attribute("space", "did:key:next-space").unwrap();
    assert!(!bar.has_attribute("data-agent-connected"));
    assert_eq!(
        shadow(&bar, ".agent-notice").text_content().as_deref(),
        Some("")
    );
    deliver_receipts(&agent, &[receipt], true);
    assert!(!bar.has_attribute("data-agent-connected"));
    bar.remove();
}
