//! The live roster remains attached to the FABB and handles frame deltas.

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::wasm_bindgen_test_configure;
use web_sys::{Element, HtmlElement, window};

wasm_bindgen_test_configure!(run_in_browser);

fn mount() -> (HtmlElement, HtmlElement) {
    tonk_fab::register();
    let document = window().unwrap().document().unwrap();
    let bar: HtmlElement = document
        .create_element("tonk-fab")
        .unwrap()
        .unchecked_into();
    bar.set_attribute("space", "did:key:zMembers").unwrap();
    document.body().unwrap().append_child(&bar).unwrap();
    let roster = bar
        .query_selector("ui-member-roster")
        .unwrap()
        .unwrap()
        .unchecked_into();
    (bar, roster)
}

fn shadow(bar: &HtmlElement, selector: &str) -> Element {
    bar.shadow_root()
        .unwrap()
        .query_selector(selector)
        .unwrap()
        .unwrap()
}

fn deliver(roster: &HtmlElement, method: &str, payload: serde_json::Value) {
    deliver_tagged(roster, "ui-member-roster", method, payload);
}

fn deliver_tagged(roster: &HtmlElement, tag: &str, method: &str, payload: serde_json::Value) {
    let payload = js_sys::JSON::parse(&payload.to_string()).unwrap();
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &tag.into()).unwrap();
    Reflect::get(roster, &method.into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(roster, &payload, &opts)
        .unwrap();
}

fn member(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "this": id,
        "fields": { "name": name, "member": format!("did:key:{id}"), "role": "tonk:member" }
    })
}

async fn settle() {
    let promise = Promise::new(&mut |resolve, _| {
        window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

fn viewer(roster: &HtmlElement, method: &str, payload: serde_json::Value) {
    deliver_tagged(roster, "ui-member-roster-viewer", method, payload);
}

fn self_member(did: &str) -> serde_json::Value {
    serde_json::json!({ "this": "state:self-member", "fields": { "member": did } })
}

#[dialog_common::test]
async fn the_self_member_row_gets_a_separate_you_marker_and_follows_an_account_change() {
    let (bar, roster) = mount();
    deliver(
        &roster,
        "reset",
        serde_json::json!([
            { "this": "owner", "fields": { "name": "Owner", "member": "did:key:owner", "role": "tonk:founder" } },
            { "this": "member", "fields": { "name": "Member", "member": "did:key:member", "role": "tonk:member" } }
        ]),
    );
    viewer(
        &roster,
        "reset",
        serde_json::json!([self_member("did:key:owner")]),
    );
    settle().await;
    let panel = shadow(&bar, ".members-list");
    assert_eq!(
        panel
            .query_selector(".mem-self")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Owner")
    );
    assert_eq!(
        panel
            .query_selector(".mem-you")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("you")
    );
    assert_eq!(
        panel
            .query_selector(".mem-role")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("owner")
    );

    // The worker re-stamps `state:self-member` when the account changes.
    viewer(
        &roster,
        "update",
        serde_json::json!({ "asserted": [self_member("did:key:member")], "retracted": [] }),
    );
    settle().await;
    assert_eq!(
        panel
            .query_selector(".mem-self")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Member")
    );
    assert_eq!(
        panel
            .query_selector(".mem-you")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("you")
    );
    assert_eq!(
        panel
            .query_selector(".mem-role")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("owner")
    );

    bar.remove();
}

#[dialog_common::test]
async fn reset_update_and_retract_render_inside_the_attached_panel() {
    let (bar, roster) = mount();
    assert_eq!(
        shadow(&bar, ".members-list")
            .text_content()
            .unwrap_or_default(),
        "no members are available"
    );

    let ada = member("ada", "Ada");
    let lin = member("lin", "Lin");
    deliver(&roster, "reset", serde_json::json!([ada, lin]));
    shadow(&bar, ".space")
        .unchecked_into::<HtmlElement>()
        .click();
    shadow(&bar, ".members")
        .unchecked_into::<HtmlElement>()
        .click();
    let panel = shadow(&bar, "#members-panel");
    assert!(!panel.has_attribute("hidden"));
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 2);
    assert_eq!(
        shadow(&bar, ".members span").text_content().as_deref(),
        Some("view members")
    );

    deliver(
        &roster,
        "update",
        serde_json::json!({ "asserted": [], "retracted": [member("ada", "Ada")] }),
    );
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 1);
    let text = panel.text_content().unwrap_or_default();
    assert!(text.contains("Lin"));
    assert!(!text.contains("Ada"));

    bar.remove();
}

#[dialog_common::test]
async fn twelve_people_remain_readable_without_invented_presence_or_agent_edges() {
    let (bar, roster) = mount();
    let rows = Array::new();
    for index in 0..12 {
        rows.push(
            &js_sys::JSON::parse(
                &member(&format!("m{index}"), &format!("Member {index}")).to_string(),
            )
            .unwrap(),
        );
    }
    let opts = Object::new();
    Reflect::set(&opts, &"tag".into(), &"ui-member-roster".into()).unwrap();
    Reflect::get(&roster, &"reset".into())
        .unwrap()
        .dyn_into::<Function>()
        .unwrap()
        .call2(&roster, &rows, &opts)
        .unwrap();

    let panel = shadow(&bar, ".members-list");
    assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 12);
    assert!(
        panel
            .text_content()
            .unwrap_or_default()
            .contains("Member 11")
    );
    assert!(
        panel
            .query_selector("[data-last-active]")
            .unwrap()
            .is_none(),
        "unknown activity is left unknown"
    );
    bar.remove();
}
