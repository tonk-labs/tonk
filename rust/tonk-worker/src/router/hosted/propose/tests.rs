use serde_json::json;

use super::*;
use crate::router::hosted::card::Kind;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen_test::wasm_bindgen_test_configure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test_configure!(run_in_service_worker);

fn context(home: bool) -> Context {
    Context {
        place: Place {
            did: "did:key:zLaunch".into(),
            name: Some("Launch".into()),
            item: None,
        },
        role: Some(Role::Write),
        agent: Agent {
            did: "did:key:zAgent".into(),
            name: Some("Juno".into()),
        },
        home,
    }
}

/// A worker answer that adds one note and changes another.
fn evaluation() -> Value {
    json!({
        "matches_before": [{
            "label": "note",
            "results": [{ "this": "urn:note:old", "fields": { "title": "Old" } }],
        }],
        "matches_after": [{
            "label": "note",
            "results": [
                { "this": "urn:note:old", "fields": { "title": "Old" } },
                { "this": "urn:note:new", "fields": { "title": "New" } },
            ],
        }],
        "commits": { "claims": 3, "entities": {} },
    })
}

#[dialog_common::test]
fn it_answers_a_held_change_once() {
    let proposals = Proposals::default();
    let (id, expires) = proposals.hold("note!: {}".into(), 100);
    assert_eq!(expires, 100 + LIFETIME);
    assert_eq!(proposals.take(&id, 101), Ok("note!: {}".to_owned()));
    assert_eq!(proposals.take(&id, 101), Err(Gone));
}

#[dialog_common::test]
fn it_forgets_a_change_once_it_lapses() {
    let proposals = Proposals::default();
    let (id, expires) = proposals.hold("a".into(), 100);
    assert_eq!(proposals.take(&id, expires), Err(Gone));
}

#[dialog_common::test]
fn it_never_answers_an_id_it_did_not_mint() {
    let proposals = Proposals::default();
    let (id, _) = proposals.hold("a".into(), 100);
    assert_eq!(proposals.take("guessed", 101), Err(Gone));
    // Another writer (another session) holds nothing under this id.
    assert_eq!(Proposals::default().take(&id, 101), Err(Gone));
    assert_eq!(proposals.take(&id, 101), Ok("a".to_owned()));
}

#[dialog_common::test]
fn it_holds_at_most_a_few_dropping_the_oldest() {
    let proposals = Proposals::default();
    let first = proposals.hold("first".into(), 100).0;
    let mut last = String::new();
    for _ in 0..MOST {
        last = proposals.hold("later".into(), 100).0;
    }
    assert_eq!(proposals.take(&first, 101), Err(Gone));
    assert_eq!(proposals.take(&last, 101), Ok("later".to_owned()));
}

#[dialog_common::test]
fn it_reads_what_a_dry_run_adds_changes_and_removes() {
    let mut removed = evaluation();
    removed["matches_after"][0]["results"] = json!([]);
    assert_eq!(
        changed(&evaluation()),
        vec![
            Changed {
                mark: "~",
                name: "Old".into(),
                kind: "note".into()
            },
            Changed {
                mark: "+",
                name: "New".into(),
                kind: "note".into()
            },
        ]
    );
    assert_eq!(changed(&removed)[0].mark, "−");
}

#[dialog_common::test]
fn the_ask_says_what_will_change_and_offers_not_now_first() {
    let card = ask(&context(false), "id-1", &evaluation());
    assert_eq!(card.kind, Kind::Ask);
    assert_eq!(card.say, "change 2 items in Launch");
    assert_eq!(card.changes.len(), 2);
    assert_eq!(card.ledger[1].value, "everyone in Launch");
    assert_eq!(card.ledger[2].value, "Juno");
    assert!(card.ledger[2].bot);
    assert_eq!(card.answers[0].label, "not now");
    assert_eq!(card.answers[0].tool, DISMISS);
    assert!(!card.answers[0].main);
    assert_eq!(card.answers[1].label, "change 2 items");
    assert_eq!(card.answers[1].tool, APPLY);
    assert!(card.answers[1].main);
    assert_eq!(
        card.answers[1].arguments,
        json!({ "space": "did:key:zLaunch", "id": "id-1" })
    );
    let home = ask(&context(true), "id-1", &evaluation());
    assert_eq!(home.ledger[1].value, "you and this agent");
}

#[dialog_common::test]
fn the_ask_lists_a_few_changes_then_how_many_more() {
    let mut many = evaluation();
    many["matches_before"] = json!([]);
    many["matches_after"][0]["results"] = (0..8)
        .map(|n| json!({ "this": format!("urn:note:{n}"), "fields": {} }))
        .collect();
    let card = ask(&context(false), "id", &many);
    assert_eq!(card.changes.len(), LISTED + 1);
    assert_eq!(card.changes[LISTED].name, "3 more");
}

#[dialog_common::test]
fn a_receipt_is_by_you_and_the_bot() {
    let card = receipt(&context(false), &evaluation());
    assert_eq!(card.kind, Kind::Strip);
    assert_eq!(card.say, "saved 2 items in Launch");
    assert!(card.by_agent);
    let mut nothing = evaluation();
    nothing["commits"]["claims"] = json!(0);
    let card = receipt(&context(false), &nothing);
    assert_eq!(card.say, "nothing changed in Launch");
    assert!(!card.by_agent);
    assert!(!dismissed(&context(false)).by_agent);
}
