use serde_json::json;

use super::*;
use crate::router::hosted::card::{Does, Kind};

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
                { "this": "urn:note:old", "fields": { "title": "Older" } },
                { "this": "urn:note:new", "fields": { "title": "New" } },
            ],
        }],
        "commits": { "claims": 3, "entities": {} },
    })
}

#[dialog_common::test]
fn it_answers_a_held_change_once() {
    let proposals = Proposals::default();
    let (id, expires) = proposals.hold("note!: {}".into(), json!(null), 100);
    assert_eq!(expires, 100 + LIFETIME);
    assert_eq!(
        proposals.take(&id, 101),
        Ok(("note!: {}".to_owned(), json!(null)))
    );
    assert_eq!(proposals.take(&id, 101), Err(Gone));
}

#[dialog_common::test]
fn it_forgets_a_change_once_it_lapses() {
    let proposals = Proposals::default();
    let (id, expires) = proposals.hold("a".into(), json!(null), 100);
    assert_eq!(proposals.take(&id, expires), Err(Gone));
}

#[dialog_common::test]
fn it_never_answers_an_id_it_did_not_mint() {
    let proposals = Proposals::default();
    let (id, _) = proposals.hold("a".into(), json!(null), 100);
    assert_eq!(proposals.take("guessed", 101), Err(Gone));
    // Another writer (another session) holds nothing under this id.
    assert_eq!(Proposals::default().take(&id, 101), Err(Gone));
    assert_eq!(proposals.take(&id, 101).unwrap().0, "a");
}

#[dialog_common::test]
fn it_holds_at_most_a_few_dropping_the_oldest() {
    let proposals = Proposals::default();
    let first = proposals.hold("first".into(), json!(null), 100).0;
    let mut last = String::new();
    for _ in 0..MOST {
        last = proposals.hold("later".into(), json!(null), 100).0;
    }
    assert_eq!(proposals.take(&first, 101), Err(Gone));
    assert_eq!(proposals.take(&last, 101).unwrap().0, "later");
}

#[dialog_common::test]
fn it_reads_what_a_dry_run_adds_changes_and_removes() {
    let mut removed = evaluation();
    removed["matches_after"][0]["results"] = json!([]);
    let changes = changed(&evaluation());
    let marks: Vec<(&str, &str)> = changes
        .iter()
        .map(|change| (change.mark, change.name.as_str()))
        .collect();
    assert_eq!(marks, [("~", "Older"), ("+", "New")]);
    // A record's concept is not shown as its kind.
    assert!(changes.iter().all(|change| change.kind.is_none()));
    assert_eq!(changed(&removed)[0].mark, "−");
}

#[dialog_common::test]
fn it_names_the_kind_of_a_schema_part_or_a_command() {
    let parts = json!({
        "matches_before": [{ "label": "view", "results": [] }, { "label": "library/install", "results": [] }],
        "matches_after": [
            { "label": "view", "results": [{ "this": "urn:v", "fields": { "name": "Decision log" } }] },
            { "label": "library/install", "results": [{ "this": "urn:c", "fields": {}, "transient": true }] },
        ],
        "commits": { "claims": 2, "entities": {} },
    });
    let changes = changed(&parts);
    assert_eq!(changes[0].kind.as_deref(), Some("view"));
    assert_eq!(changes[1].kind.as_deref(), Some("command"));
    assert!(changes[1].command);
}

#[dialog_common::test]
fn a_match_the_same_before_and_after_is_no_change() {
    let mut read = evaluation();
    // A second expression that only reads: the same answer twice.
    let query = json!({
        "label": "person",
        "results": [{ "this": "urn:person:sam", "fields": { "name": "Sam" } }],
    });
    read["matches_before"]
        .as_array_mut()
        .unwrap()
        .push(query.clone());
    read["matches_after"].as_array_mut().unwrap().push(query);
    let changes = changed(&read);
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes.iter().all(|change| change.of == "note"));
}

/// A preview that adds `names`, as notes.
fn adding(names: &[&str]) -> Value {
    json!({
        "matches_before": [{ "label": "note", "results": [] }],
        "matches_after": [{
            "label": "note",
            "results": names
                .iter()
                .map(|name| json!({ "this": format!("urn:note:{name}"), "fields": { "title": name } }))
                .collect::<Vec<_>>(),
        }],
        "commits": { "claims": names.len() * 2, "entities": {} },
    })
}

#[dialog_common::test]
fn the_ask_says_what_it_does_and_leads_with_the_main_answer() {
    let card = ask(&context(false), "id-1", &adding(&["Sun hat", "Map"]));
    assert_eq!(card.kind, Kind::Ask);
    assert_eq!(card.say, "add 2 items to Launch");
    let ledger: Vec<(&str, &str)> = card
        .ledger
        .iter()
        .map(|entry| (entry.term.as_str(), entry.value.as_str()))
        .collect();
    assert_eq!(
        ledger,
        [
            ("in", "Launch"),
            ("who sees it", "everyone in Launch"),
            ("undo", "from the receipt")
        ]
    );
    assert!(card.fine.is_none());
    let answers: Vec<(&str, Look)> = card
        .answers
        .iter()
        .map(|answer| (answer.label.as_str(), answer.kind))
        .collect();
    assert_eq!(
        answers,
        [("add 2 items", Look::Main), ("not now", Look::Ring)]
    );
    let arguments = json!({ "space": "did:key:zLaunch", "id": "id-1" });
    assert_eq!(
        card.answers[0].does,
        Does::Call {
            tool: APPLY.into(),
            arguments: arguments.clone()
        }
    );
    assert_eq!(
        card.answers[1].does,
        Does::Call {
            tool: DISMISS.into(),
            arguments
        }
    );

    let one = ask(&context(true), "id-1", &adding(&["Sun hat"]));
    assert_eq!(one.say, "add Sun hat to Launch");
    assert_eq!(one.ledger[1].value, "you and this agent");
    let changing = ask(&context(false), "id", &evaluation());
    assert_eq!(changing.say, "change 2 items in Launch");
    assert_eq!(changing.answers[1].label, "keep as is");
}

#[dialog_common::test]
fn the_ask_lists_a_few_changes_then_how_many_more() {
    let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let card = ask(&context(false), "id", &adding(&names));
    assert_eq!(card.changes.len(), LISTED + 1);
    assert_eq!(card.changes[LISTED].name, "3 more");
}

#[dialog_common::test]
fn a_receipt_says_what_was_done_by_you_and_the_bot() {
    let card = receipt(&context(false), &adding(&["Sun hat", "Map"]), true);
    assert_eq!(card.kind, Kind::Strip);
    assert_eq!(card.say, "added 2 items to Launch");
    assert!(card.by_agent);
    assert_eq!(
        receipt(&context(false), &adding(&["Sun hat"]), true).say,
        "added Sun hat to Launch"
    );
    // Asserting what is there already changes nothing, whatever the claims.
    let mut same = evaluation();
    same["matches_after"] = same["matches_before"].clone();
    let card = receipt(&context(false), &same, false);
    assert_eq!(card.say, "nothing changed in Launch");
    assert!(!card.by_agent);
    assert!(!dismissed(&context(false)).by_agent);
}

#[dialog_common::test]
fn a_refused_write_is_not_the_same_as_one_not_saved() {
    let unsaved = not_saved(&context(false));
    assert_eq!(unsaved.label.as_deref(), Some("not saved"));
    assert_eq!(unsaved.disc, Disc::Hollow);
    assert!(unsaved.fine.is_none());
    let answers: Vec<(&str, Look)> = unsaved
        .answers
        .iter()
        .map(|answer| (answer.label.as_str(), answer.kind))
        .collect();
    assert_eq!(
        answers,
        [("try again", Look::Main), ("drop it", Look::Link)]
    );
    let refused = rejected(&context(false), false);
    assert_eq!(refused.disc, Disc::Filled);
    assert!(refused.fine.is_none());
    assert!(!refused.say.contains("did:"), "{}", refused.say);
}

#[dialog_common::test]
fn a_space_that_moved_offers_theirs_first_then_mine() {
    let card = moved(&context(false), Some(("id-2", &adding(&["Sun hat"]))));
    assert_eq!(
        card.space.as_ref().unwrap().item.as_deref(),
        Some("Sun hat")
    );
    assert_eq!(card.label.as_deref(), Some("Sun hat changed"));
    assert_eq!(card.ledger[0].term, "yours");
    assert_eq!(card.ledger[0].value, "add Sun hat");
    assert_eq!(card.answers[0].label, "keep theirs");
    assert_eq!(card.answers[1].label, "use mine");
    let stale = moved(&context(false), None);
    assert!(stale.answers.is_empty());
}

#[dialog_common::test]
fn a_space_out_of_reach_shows_neither_disc_nor_role() {
    let card = no_access(&context(false).place, &context(false).agent, false);
    assert!(card.role.is_none());
    assert_eq!(card.label.as_deref(), Some("no access to Launch"));
    assert!(!card.say.contains("tonk_"), "{}", card.say);
    let read = read_only(&Context {
        role: Some(Role::Read),
        ..context(false)
    });
    assert_eq!(read.role, Some(Role::Read));
    assert_eq!(read.label.as_deref(), Some("read only in Launch"));
}
