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
fn an_ask_lists_its_answers_main_first() {
    let card = Card::ask(
        launch(),
        Some(Role::Write),
        "install Decision log in Launch",
        agent(),
    )
    .change("+", "Decision log", Some("view".into()))
    .entry("in", "Launch")
    .entry_bot("changed by", "you")
    .answer(
        "install",
        Look::Main,
        "tonk_proposal_apply",
        json!({ "id": "a" }),
    )
    .answer(
        "not now",
        Look::Ring,
        "tonk_proposal_dismiss",
        json!({ "id": "a" }),
    )
    .answer_saying("drop it", Look::Link, "dropped", "The person dropped it.")
    .to_value();
    assert_eq!(card["role"], "write");
    assert_eq!(card["disc"], "filled");
    assert_eq!(card["space"]["item"], "Review");
    assert_eq!(
        card["changes"],
        json!([{ "mark": "+", "name": "Decision log", "kind": "view" }])
    );
    assert_eq!(
        card["ledger"][1],
        json!({ "term": "changed by", "value": "you", "bot": true })
    );
    assert_eq!(
        card["answers"],
        json!([
            { "label": "install", "kind": "main", "tool": "tonk_proposal_apply", "arguments": { "id": "a" } },
            { "label": "not now", "kind": "ring", "tool": "tonk_proposal_dismiss", "arguments": { "id": "a" } },
            { "label": "drop it", "kind": "link", "receipt": "dropped", "context": "The person dropped it." },
        ])
    );
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
