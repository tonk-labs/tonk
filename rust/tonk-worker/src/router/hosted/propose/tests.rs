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
        origin: "https://tonk.example".into(),
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
    let (id, expires) = proposals.hold(
        "note!: {}".into(),
        json!(null),
        json!({}),
        Effect::Document,
        100,
    );
    assert_eq!(expires, 100 + LIFETIME);
    assert_eq!(
        proposals.take(&id, 101),
        Ok(Taken {
            document: "note!: {}".to_owned(),
            revision: json!(null),
            seen: json!({}),
            effect: Effect::Document,
        })
    );
    assert_eq!(proposals.take(&id, 101), Err(Gone));
}

#[dialog_common::test]
fn it_forgets_a_change_once_it_lapses() {
    let proposals = Proposals::default();
    let (id, expires) = proposals.hold("a".into(), json!(null), json!({}), Effect::Document, 100);
    assert_eq!(proposals.take(&id, expires), Err(Gone));
}

#[dialog_common::test]
fn it_never_answers_an_id_it_did_not_mint() {
    let proposals = Proposals::default();
    let (id, _) = proposals.hold("a".into(), json!(null), json!({}), Effect::Document, 100);
    assert_eq!(proposals.take("guessed", 101), Err(Gone));
    // Another writer (another session) holds nothing under this id.
    assert_eq!(Proposals::default().take(&id, 101), Err(Gone));
    assert_eq!(proposals.take(&id, 101).unwrap().document, "a");
}

#[dialog_common::test]
fn it_holds_at_most_a_few_dropping_the_oldest() {
    let proposals = Proposals::default();
    let first = proposals
        .hold(
            "first".into(),
            json!(null),
            json!({}),
            Effect::Document,
            100,
        )
        .0;
    let mut last = String::new();
    for _ in 0..MOST {
        last = proposals
            .hold(
                "later".into(),
                json!(null),
                json!({}),
                Effect::Document,
                100,
            )
            .0;
    }
    assert_eq!(proposals.take(&first, 101), Err(Gone));
    assert_eq!(proposals.take(&last, 101).unwrap().document, "later");
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
    // A record's concept is not shown as its kind: it is an item.
    assert!(
        changes
            .iter()
            .all(|change| change.kind.as_deref() == Some("item"))
    );
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
        [("in", "Launch"), ("who sees it", "everyone in Launch")]
    );
    assert!(card.fine.is_none());
    let answers: Vec<(&str, Look)> = card
        .answers
        .iter()
        .map(|answer| (answer.label.as_str(), answer.kind))
        .collect();
    assert_eq!(
        answers,
        [("not now", Look::Ring), ("add 2 items", Look::Main)]
    );
    let arguments = json!({ "space": "did:key:zLaunch", "id": "id-1" });
    assert_eq!(
        card.answers[0].does,
        Does::Call {
            tool: DISMISS.into(),
            arguments: arguments.clone()
        }
    );
    assert_eq!(
        card.answers[1].does,
        Does::Call {
            tool: APPLY.into(),
            arguments
        }
    );

    let one = ask(&context(true), "id-1", &adding(&["Sun hat"]));
    assert_eq!(one.say, "add Sun hat to Launch");
    assert_eq!(one.ledger[1].value, "you and this agent");
    let changing = ask(&context(false), "id", &evaluation());
    assert_eq!(changing.say, "change 2 items in Launch");
    assert_eq!(changing.answers[0].label, "keep as is");
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
        [("drop it", Look::Link), ("try again", Look::Main)]
    );
    let refused = rejected(&context(false), false);
    assert_eq!(refused.disc, Disc::Filled);
    assert!(refused.fine.is_none());
    assert!(!refused.say.contains("did:"), "{}", refused.say);
}

#[dialog_common::test]
fn a_space_that_moved_shows_theirs_and_yours_then_offers_theirs_first() {
    let again = json!({
        "matches_before": [{
            "label": "stay",
            "results": [{ "this": "urn:stay:seeblick", "fields": { "name": "Seeblick Hotel", "nights": 4 } }],
        }],
        "matches_after": [{
            "label": "stay",
            "results": [{ "this": "urn:stay:seeblick", "fields": { "name": "Seeblick Hotel", "nights": 3 } }],
        }],
        "commits": { "claims": 1, "entities": {} },
    });
    let card = moved(&context(false), Some(("id-2", &again)));
    assert_eq!(
        card.space.as_ref().unwrap().item.as_deref(),
        Some("Seeblick Hotel")
    );
    assert_eq!(card.label.as_deref(), Some("Seeblick Hotel changed"));
    let ledger: Vec<(&str, &str)> = card
        .ledger
        .iter()
        .map(|entry| (entry.term.as_str(), entry.value.as_str()))
        .collect();
    assert_eq!(ledger, [("theirs", "4"), ("yours", "3")]);
    assert_eq!(card.answers[0].label, "use mine");
    assert_eq!(card.answers[1].label, "keep theirs");
    assert_eq!(card.answers[1].kind, Look::Main);
    let stale = moved(&context(false), None);
    assert!(stale.answers.is_empty());
}

#[dialog_common::test]
fn a_space_out_of_reach_offers_to_grant_it_in_tonk() {
    let card = no_access(&context(false), false);
    assert!(card.role.is_none());
    assert_eq!(card.label.as_deref(), Some("no access to Launch"));
    assert!(!card.say.contains("tonk_"), "{}", card.say);
    assert_eq!(
        card.answers[0].does,
        Does::Open {
            url: "https://tonk.example/settings".into()
        }
    );
    let read = read_only(&Context {
        role: Some(Role::Read),
        ..context(false)
    });
    assert_eq!(read.role, Some(Role::Read));
    assert_eq!(read.label.as_deref(), Some("read only in Launch"));
    assert_eq!(
        read.answers[1].does,
        Does::Open {
            url: "https://tonk.example/space/did:key:zLaunch".into()
        }
    );
    assert_eq!(read.answers[0].label, "skip");
}

/// An evaluation that runs `name` once, with `fields`.
fn running(name: &str, fields: Value) -> Value {
    json!({
        "matches_before": [{ "label": name, "results": [] }],
        "matches_after": [{
            "label": name,
            "results": [{ "this": "did:key:zCommand", "fields": fields, "transient": true }],
        }],
        "commits": { "claims": 2, "entities": {} },
    })
}

#[dialog_common::test]
fn a_document_naming_a_key_command_is_asked_first() {
    assert!(names_key_command(
        "tonk/rename-repository!:\n  subject: did:key:zLaunch\n  name: \"Launch 2\"\n"
    ));
    assert!(names_key_command(
        "member/expel!:\n  member: did:key:zSam\n"
    ));
    assert!(names_key_command("tonk/invite!:\n  time: 1\n"));
    assert!(!names_key_command("note!:\n  title: \"tonk/invite\"\n"));
    assert!(!names_key_command("tonk/repository:\n  name: ?name\n"));
}

#[dialog_common::test]
fn the_named_tools_write_one_key_command_each() {
    let rename = rename_document("did:key:zLaunch", "Launch \"2\"\n  name: other");
    assert_eq!(
        rename,
        "tonk/rename-repository!:\n  subject: did:key:zLaunch\n  name: \"Launch \\\"2\\\"\\n  name: other\"\n"
    );
    assert!(names_key_command(&rename));
    assert_eq!(rename.lines().count(), 3, "the name adds no line: {rename}");

    let remove = remove_member_document("did:key:zSam");
    assert_eq!(remove, "member/expel!:\n  member: did:key:zSam\n");
    assert!(names_key_command(&remove));
}

#[dialog_common::test]
fn a_key_command_is_asked_and_receipted_in_words() {
    let rename = running(
        "tonk/rename-repository",
        json!({ "subject": "did:key:zLaunch", "name": "Launch 2" }),
    );
    let card = ask(&context(false), "id-1", &rename);
    assert_eq!(card.say, "rename Launch to Launch 2");
    assert!(card.changes.is_empty(), "a key command lists no changes");
    let answers: Vec<(&str, Look)> = card
        .answers
        .iter()
        .map(|answer| (answer.label.as_str(), answer.kind))
        .collect();
    assert_eq!(
        answers,
        [("keep the name", Look::Ring), ("rename it", Look::Main)]
    );
    assert_eq!(
        receipt(&context(false), &rename, true).say,
        "renamed Launch to Launch 2"
    );

    let expel = running(
        "member/expel",
        json!({ "member": "did:key:z6MkSamSamSamSamSam" }),
    );
    let card = ask(&context(false), "id-2", &expel);
    assert_eq!(card.say, "remove z6MkSamSamSa… from Launch");
    assert_eq!(card.ledger[0].term, "they lose");

    let invite = running("tonk/invite", json!({ "time": 1.0 }));
    assert_eq!(
        ask(&context(false), "id-3", &invite).say,
        "make an invite to Launch"
    );
    assert_eq!(key_command(&invite).unwrap().name, "tonk/invite");
    assert!(
        key_command(&evaluation()).is_none(),
        "a plain write runs no key command"
    );
}
