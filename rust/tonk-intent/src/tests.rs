use serde_json::json;

use super::*;
use dialog_lingo::Selection;

fn row(this: &str, fields: serde_json::Value) -> Row {
    Row {
        this: this.into(),
        fields: serde_json::from_value(fields).expect("fields are a map"),
    }
}

/// The rows a space branch seeded with the library would deliver.
fn space() -> Source {
    Source {
        branch: "main@did:key:space".into(),
        verbs: vec![
            row("concept:expel", json!({ "name": "expel" })),
            row("concept:expel", json!({ "name": "remove member" })),
            row("concept:rename", json!({ "name": "rename" })),
        ],
        arguments: vec![
            row(
                "argument:expel-member",
                json!({
                    "command": "concept:expel",
                    "field": "the:expel-member",
                    "role": "role:object",
                    "noun": "concept:account",
                }),
            ),
            row(
                "argument:rename-subject",
                json!({
                    "command": "concept:rename",
                    "field": "the:rename-subject",
                    "role": "role:object",
                    "noun": "concept:repository",
                }),
            ),
            row(
                "argument:rename-name",
                json!({
                    "command": "concept:rename",
                    "field": "the:rename-name",
                    "role": "role:goal",
                }),
            ),
        ],
        roles: vec![
            row("role:object", json!({ "name": "object" })),
            row("role:goal", json!({ "name": "goal" })),
        ],
        nouns: vec![row("concept:account", json!({ "name": "member" }))],
        attributes: vec![
            row(
                "the:expel-member",
                json!({ "id": "xyz.tonk.command.expel-member/member", "type": "Entity" }),
            ),
            row(
                "the:rename-subject",
                json!({ "id": "xyz.tonk.rename-repository/subject", "type": "Entity" }),
            ),
            row(
                "the:rename-name",
                json!({ "id": "xyz.tonk.rename-repository/name", "type": "Text" }),
            ),
        ],
        concepts: BTreeMap::from([
            (
                "concept:account".into(),
                ConceptRows {
                    label: Some("<span class=\"who\">{name}</span>\n".into()),
                    rows: vec![
                        row("did:key:alice", json!({ "name": "Alice" })),
                        row("did:key:bob", json!({ "name": "Bob" })),
                    ],
                },
            ),
            (
                "concept:repository".into(),
                ConceptRows {
                    label: Some("{name}".into()),
                    rows: vec![row("did:key:space", json!({ "name": "Budget" }))],
                },
            ),
        ]),
    }
}

fn request(input: &str) -> Request {
    Request {
        input: input.into(),
        max: 5,
        context: Context {
            selection: None,
            // The page knows the entity, not its label.
            this: Some(Selection {
                text: String::new(),
                entity: Some("did:key:space".into()),
            }),
        },
        sources: vec![space()],
        memory: Vec::new(),
        now: Some(1_000.5),
    }
}

#[test]
fn it_matches_candidates_by_their_rendered_label() {
    let proposals = propose(&request("expel bo"));
    let top = &proposals[0];
    assert_eq!(top.command, "concept:expel");
    assert_eq!(top.branch, "main@did:key:space");
    assert_eq!(top.parse.display_text(), "expel [Bob]");
}

#[test]
fn it_builds_the_claim_from_the_fields_selectors() {
    let proposals = propose(&request("expel bob"));
    assert_eq!(
        proposals[0].claim,
        Some(json!({
            "claims": [{
                "op": "assert",
                "application": {
                    "predicate": { "kind": "transient", "concept": { "with": {
                        "member": { "the": "xyz.tonk.command.expel-member/member", "as": "Entity" }
                    } } },
                    "parameters": { "member": "did:key:bob" }
                }
            }]
        }))
    );
}

#[test]
fn it_reads_this_as_the_entity_the_page_shows() {
    let proposals = propose(&request("rename this to Q3"));
    let top = &proposals[0];
    assert_eq!(top.command, "concept:rename");
    assert_eq!(top.parse.display_text(), "rename [Budget] to [Q3]");
    let claim = top.claim.as_ref().expect("a complete parse has a claim");
    assert_eq!(
        claim["claims"][0]["application"]["parameters"],
        json!({ "subject": "did:key:space", "name": "Q3" })
    );
}

#[test]
fn it_labels_an_empty_argument_by_its_noun_and_withholds_the_claim() {
    let proposals = propose(&request("expel"));
    let top = &proposals[0];
    assert_eq!(top.parse.display_text(), "expel (member)");
    assert_eq!(top.claim, None);
}

#[test]
fn it_keeps_one_command_on_two_branches_apart() {
    let mut profile = space();
    profile.branch = "main@profile:tonk".into();
    let mut request = request("rename this to Q3");
    request.sources.push(profile);
    let branches: BTreeSet<String> = propose(&request)
        .into_iter()
        .filter(|proposal| proposal.command == "concept:rename")
        .map(|proposal| proposal.branch)
        .collect();
    assert_eq!(branches.len(), 2);
}

#[test]
fn it_fills_the_now_role_from_the_callers_clock_without_asking_for_it() {
    let mut profile = space();
    profile.branch = "main@profile:tonk".into();
    profile.verbs = vec![row("concept:pause", json!({ "name": "pause sync" }))];
    profile.arguments = vec![
        row(
            "argument:pause-space",
            json!({
                "command": "concept:pause",
                "field": "the:pause-space",
                "role": "role:object",
                "noun": "concept:repository",
            }),
        ),
        row(
            "argument:pause-time",
            json!({ "command": "concept:pause", "field": "the:pause-time", "role": "role:now" }),
        ),
    ];
    profile
        .roles
        .push(row("role:now", json!({ "name": "now" })));
    profile.attributes = vec![
        row(
            "the:pause-space",
            json!({ "id": "xyz.tonk.pause-sync/space", "type": "Entity" }),
        ),
        row(
            "the:pause-time",
            json!({ "id": "xyz.tonk.command.pause-sync/time", "type": "Float" }),
        ),
    ];
    let mut request = request("pause");
    request.sources = vec![profile];
    let top = &propose(&request)[0];
    // The space defaults to the one the page shows; time is not shown.
    assert_eq!(top.parse.display_text(), "pause sync [Budget]");
    assert_eq!(
        top.claim.as_ref().unwrap()["claims"][0]["application"]["parameters"],
        json!({ "space": "did:key:space", "time": 1_000.5 })
    );
}

#[test]
fn it_scores_a_command_higher_for_the_words_it_was_chosen_for() {
    let score = |request: &Request, command: &str| {
        propose(request)
            .into_iter()
            .find(|proposal| proposal.command == command)
            .map(|proposal| proposal.parse.score)
            .expect("proposed")
    };
    let choices = |input: &str| {
        (0..3)
            .map(|n| {
                row(
                    &format!("choice:{n}"),
                    json!({ "command": "concept:expel", "input": input }),
                )
            })
            .collect()
    };
    let mut request = request("re");
    let before = score(&request, "concept:expel");
    request.memory = choices("re");
    let after = score(&request, "concept:expel");
    assert!(after > before, "{before} -> {after}");
    // Only for those words: chosen after "rem", it does not move for "re".
    request.memory = choices("rem");
    assert_eq!(score(&request, "concept:expel"), before);
}

#[test]
fn it_completes_the_input_up_to_the_first_empty_argument() {
    let top = &propose(&request("ren"))[0];
    assert_eq!(top.completion.as_deref(), Some("rename Budget to "));
    // The typed part stays as typed.
    let top = &propose(&request("REN"))[0];
    assert_eq!(top.completion.as_deref(), Some("REName Budget to "));
    // Nothing to add once the input says it all.
    let top = &propose(&request("rename this to Q3"))[0];
    assert_eq!(top.completion, None);
}

#[test]
fn it_drops_readings_that_only_fit_by_taking_the_input_as_text() {
    let shown: Vec<String> = propose(&request("ren"))
        .iter()
        .map(|proposal| proposal.parse.display_text())
        .collect();
    assert!(
        !shown.iter().any(|text| text.ends_with("[ren]")),
        "{shown:?}"
    );
}

#[test]
fn it_says_which_roles_take_which_kind_of_thing() {
    let top = &propose(&request("expel bob"))[0];
    assert_eq!(
        top.nouns,
        BTreeMap::from([("object".to_owned(), "member".to_owned())])
    );
}

#[test]
fn it_reads_an_accepted_completion_back_as_the_same_command() {
    let completion = propose(&request("ren"))[0].completion.clone().unwrap();
    let top = &propose(&request(&format!("{completion}Q3")))[0];
    assert_eq!(
        top.claim.as_ref().unwrap()["claims"][0]["application"]["parameters"],
        json!({ "subject": "did:key:space", "name": "Q3" })
    );
}

#[test]
fn it_lists_what_can_be_done_without_saying_more() {
    let mut request = request("");
    request.sources[0]
        .verbs
        .push(row("concept:members", json!({ "name": "view members" })));
    let shown: Vec<String> = menu(&request)
        .iter()
        .map(|proposal| proposal.parse.display_text())
        .collect();
    // Expel wants a member and rename a name; nothing typed says either.
    assert_eq!(shown, vec!["view members"]);

    // The most chosen come first.
    request.sources[0]
        .verbs
        .push(row("concept:agent", json!({ "name": "connect agent" })));
    request.memory = vec![row(
        "choice:0",
        json!({ "command": "concept:members", "input": "" }),
    )];
    let order: Vec<String> = menu(&request)
        .iter()
        .map(|proposal| proposal.command.clone())
        .collect();
    assert_eq!(order, vec!["concept:members", "concept:agent"]);
}

/// How the parse scales with a noun's rows. Run with
/// `cargo test --release -p tonk-intent -- --ignored --nocapture scales`.
#[test]
#[ignore = "timing, not a check"]
fn it_scales_with_candidates() {
    for count in [10, 100, 1_000, 10_000] {
        let mut request = request("");
        let members = &mut request.sources[0]
            .concepts
            .get_mut("concept:account")
            .expect("members")
            .rows;
        for n in 0..count {
            members.push(row(
                &format!("did:key:member{n}"),
                json!({ "name": format!("Member {n} of the budget team") }),
            ));
        }
        for input in [
            "ex",
            "expel budget",
            "expel member 42 of",
            "rename this to Q3",
        ] {
            request.input = input.into();
            let runs = 5;
            let start = std::time::Instant::now();
            for _ in 0..runs {
                std::hint::black_box(propose(&request));
            }
            println!(
                "{count:>6} rows  {input:<20} {:>8.2} ms",
                start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs)
            );
        }
    }
}
