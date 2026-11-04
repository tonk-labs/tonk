use serde_json::json;

use super::*;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_service_worker);

const ORIGIN: &str = "https://tonk.example/";

fn agent() -> Agent {
    Agent {
        did: "did:key:zAgent".into(),
        name: Some("Juno".into()),
    }
}

fn launch() -> Place {
    Place {
        did: "did:key:zLaunch".into(),
        name: Some("Launch".into()),
        item: Some("Review".into()),
    }
}

#[dialog_common::test]
fn it_serves_the_card_as_an_mcp_app_with_fonts_from_the_origin_only() {
    let read = read_resource(CARD_URI, ORIGIN).unwrap();
    let content = &read["contents"][0];
    assert_eq!(content["uri"], CARD_URI);
    assert_eq!(content["mimeType"], "text/html;profile=mcp-app");
    let meta = &content["_meta"];
    assert_eq!(
        meta["ui"]["csp"],
        json!({ "connectDomains": [], "resourceDomains": ["https://tonk.example"] })
    );
    assert_eq!(meta["ui"]["prefersBorder"], true);
    assert_eq!(
        meta["openai/widgetCSP"],
        json!({ "connect_domains": [], "resource_domains": ["https://tonk.example"] })
    );
    assert!(read_resource("ui://tonk/other.html", ORIGIN).is_none());
    let listed = resources();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["uri"], CARD_URI);
}

#[dialog_common::test]
fn it_splices_the_in_agent_stylesheet_into_the_card() {
    let read = read_resource(CARD_URI, ORIGIN).unwrap();
    let html = read["contents"][0]["text"].as_str().unwrap();
    assert!(!html.contains(super::super::in_agent::MARK));
    assert!(html.contains(".tonk--ask"), "the stylesheet is not spliced");
    assert!(html.contains("https://tonk.example/in-agent/fonts/"));
    // The card loads nothing but what is inline and the fonts.
    assert!(!html.contains("<script src"));
    assert!(!html.contains("fetch("));
}

#[dialog_common::test]
fn it_draws_what_the_mocks_name_and_labels_each_kind() {
    let html = CARD;
    for part in [
        "tonk-changes",
        "tonk-lines",
        "tonk-ledger",
        "tonk-fine",
        "tonk-acts",
        "tonk-bot",
        "tonk-where",
        "'Tonk asks: '",
        "'Tonk: '",
        "'status'",
        "ui/update-model-context",
        "ui/notifications/host-context-changed",
    ] {
        assert!(html.contains(part), "the card does not draw {part}");
    }
}

#[dialog_common::test]
fn a_tell_where_the_person_is_not_in_the_space_has_no_role() {
    let card = Card::tell(
        Some(launch()),
        None,
        "no access",
        "You’re not in Launch.",
        agent(),
    )
    .to_value();
    assert_eq!(card["kind"], "tell");
    assert_eq!(card["label"], "no access");
    assert!(card.get("role").is_none(), "{card}");
    assert_eq!(
        card["agent"],
        json!({ "did": "did:key:zAgent", "name": "Juno" })
    );
    assert_eq!(card["person"], json!({ "name": null }));
}

#[dialog_common::test]
fn an_ask_lists_its_answers_safe_first() {
    let card = Card::ask(
        launch(),
        Some(Role::Write),
        "change 2 items in Launch",
        agent(),
    )
    .change("+", "Parked", Some("group".into()))
    .entry("in", "Launch")
    .entry_bot("made by", "Juno")
    .answer(
        "not now",
        false,
        "tonk_proposal_dismiss",
        json!({ "id": "a" }),
    )
    .answer(
        "change 2 items",
        true,
        "tonk_proposal_apply",
        json!({ "id": "a" }),
    )
    .to_value();
    assert_eq!(card["role"], "write");
    assert_eq!(card["disc"], "filled");
    assert_eq!(card["space"]["item"], "Review");
    assert_eq!(
        card["changes"],
        json!([{ "mark": "+", "name": "Parked", "kind": "group" }])
    );
    assert_eq!(
        card["ledger"][1],
        json!({ "term": "made by", "value": "Juno", "bot": true })
    );
    let answers = card["answers"].as_array().unwrap();
    assert_eq!(answers[0]["label"], "not now");
    assert!(answers[0].get("main").is_none());
    assert_eq!(answers[1]["main"], true);
    assert_eq!(answers[1]["tool"], "tonk_proposal_apply");
}

#[dialog_common::test]
fn a_strip_by_the_agent_says_so() {
    let card = Card::strip(
        Some(launch()),
        Some(Role::Read),
        "saved 1 item in Launch",
        agent(),
    )
    .by_agent()
    .to_value();
    assert_eq!(card["kind"], "strip");
    assert_eq!(card["byAgent"], true);
    assert_eq!(card["role"], "read");
}

#[dialog_common::test]
fn a_strip_ends_with_undo_whose_change_the_card_reads_from_meta() {
    let receipt = || {
        Card::strip(
            Some(launch()),
            Some(Role::Write),
            "saved 1 item in Launch",
            agent(),
        )
    };
    let strip = receipt().by_agent().undo("tonk_undo").to_value();
    // The label and the tool only: the change is in `_meta`, which the
    // model does not read.
    assert_eq!(
        strip["undo"],
        json!({ "label": "undo", "tool": "tonk_undo" })
    );
    assert!(receipt().to_value().get("undo").is_none());
    let read = read_resource(CARD_URI, ORIGIN).unwrap();
    let html = read["contents"][0]["text"].as_str().unwrap();
    assert!(
        html.contains(&format!("'{}'", super::super::mcp::CHANGE_META)),
        "the card reads the change from another key"
    );
}
