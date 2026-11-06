use serde_json::json;

use super::*;
use tonk_lingo::Selection;

fn row(this: &str, fields: serde_json::Value) -> Row {
    Row {
        this: this.into(),
        fields: serde_json::from_value(fields).expect("fields are a map"),
    }
}

/// What rules derive for a command's entity field: one value each.
fn derived(command: &str, field: &str, values: &[&str]) -> Vec<Row> {
    values
        .iter()
        .map(|value| {
            row(
                "",
                json!({ "command": command, "field": field, "value": value }),
            )
        })
        .collect()
}

/// The rows a space branch seeded with the library would deliver, with
/// what its rules derive: every member for expel's member, and the space
/// itself for rename's subject.
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
                    "role": "object",
                }),
            ),
            row(
                "argument:rename-subject",
                json!({
                    "command": "concept:rename",
                    "field": "the:rename-subject",
                    "role": "object",
                }),
            ),
            row(
                "argument:rename-name",
                json!({
                    "command": "concept:rename",
                    "field": "the:rename-name",
                    "role": "goal",
                }),
            ),
        ],
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
        fragments: [
            derived(
                "concept:expel",
                "the:expel-member",
                &["did:key:alice", "did:key:bob"],
            ),
            derived("concept:rename", "the:rename-subject", &["did:key:space"]),
        ]
        .concat(),
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
        // The page is the space's own.
        shown: vec!["did:key:space".into()],
        sources: vec![space()],
        memory: Vec::new(),
        picks: Vec::new(),
        now: Some(1_000.5),
    }
}

#[test]
fn it_fills_no_field_unasked_with_a_value_the_page_does_not_show() {
    // Rules offer only Alice for expel's member, and the page shows the
    // space, not her: "expel" lists her, to be taken, rather than
    // expelling the only one there is.
    let mut source = space();
    source.fragments = derived("concept:expel", "the:expel-member", &["did:key:alice"]);
    let mut asked = request("expel");
    asked.sources = vec![source];
    let top = &propose(&asked)[0];
    assert_eq!(top.parse.display_text(), "expel [Alice]");
    assert!(top.listed);
    assert_eq!(top.claim, None);

    // Named, she fills it, by her concept row's label.
    asked.input = "expel alice".into();
    let top = &propose(&asked)[0];
    assert_eq!(top.parse.display_text(), "expel [Alice]");
    assert!(top.claim.is_some());

    // What rules do not derive cannot be named: Bob is no candidate here.
    asked.input = "expel bob".into();
    assert!(
        propose(&asked)
            .iter()
            .all(|proposal| proposal.parse.display_text() != "expel [Bob]")
    );
}

#[test]
fn it_shows_a_derived_value_by_the_label_derived_with_it() {
    // A value no labelled concept has as a row is shown by the label it
    // was derived with.
    let mut source = space();
    source
        .verbs
        .push(row("concept:retitle", json!({ "name": "retitle" })));
    source.arguments.push(row(
        "",
        json!({ "command": "concept:retitle", "field": "the:retitle-subject", "role": "object" }),
    ));
    source.arguments.push(row(
        "",
        json!({ "command": "concept:retitle", "field": "the:retitle-title", "role": "goal" }),
    ));
    source.attributes.push(row(
        "the:retitle-subject",
        json!({ "id": "xyz.tonk.notebook.retitle/subject", "type": "Entity" }),
    ));
    source.attributes.push(row(
        "the:retitle-title",
        json!({ "id": "xyz.tonk.notebook.retitle/title", "type": "Text" }),
    ));
    source.fragments = vec![row(
        "",
        json!({
            "command": "concept:retitle", "field": "the:retitle-subject",
            "value": "notebook:plans", "label": "Plans"
        }),
    )];
    // Nothing derived: the subject stays empty rather than taking text.
    let mut bare = source.clone();
    bare.fragments.clear();
    let mut asked = request("retitle to Notes");
    asked.sources = vec![bare];
    let proposals = propose(&asked);
    assert!(
        proposals.iter().all(|proposal| proposal.claim.is_none()),
        "an entity field takes no typed text: {:?}",
        proposals
            .iter()
            .map(|p| p.parse.display_text())
            .collect::<Vec<_>>()
    );

    // The page is the notebook's: what it shows comes before its space.
    let mut asked = request("retitle to Notes");
    asked.sources = vec![source];
    asked.shown = vec!["notebook:plans".into(), "did:key:space".into()];
    let top = &propose(&asked)[0];
    assert_eq!(top.parse.display_text(), "retitle [Plans] to [Notes]");
    let claim = top
        .claim
        .as_ref()
        .expect("both fields are filled")
        .to_string();
    assert!(
        claim.contains("notebook:plans") && claim.contains("Notes"),
        "{claim}"
    );
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
fn it_labels_an_empty_argument_by_its_field_and_withholds_the_claim() {
    // Nothing derived for the member: nothing to list either.
    let mut source = space();
    source
        .fragments
        .retain(|row| row.text("command") != Some("concept:expel"));
    let mut request = request("expel");
    request.sources = vec![source];
    let proposals = propose(&request);
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
            json!({ "command": "concept:pause", "field": "the:pause-space", "role": "object" }),
        ),
        row(
            "argument:pause-time",
            json!({ "command": "concept:pause", "field": "the:pause-time", "role": "now" }),
        ),
    ];
    // Rules offer every space on the account; the page shows Budget.
    profile.fragments = derived(
        "concept:pause",
        "the:pause-space",
        &["did:key:space", "did:key:plans"],
    );
    profile
        .concepts
        .get_mut("concept:repository")
        .expect("spaces")
        .rows
        .push(row("did:key:plans", json!({ "name": "Plans" })));
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
    // Of the spaces derived, the field takes the one the page shows; time
    // is not shown.
    assert_eq!(top.parse.display_text(), "pause sync [Budget]");
    assert_eq!(
        top.claim.as_ref().unwrap()["claims"][0]["application"]["parameters"],
        json!({ "space": "did:key:space", "time": 1_000.5 })
    );

    // Off a space's page, which of several is for the user to say.
    request.context.this = None;
    request.shown.clear();
    let top = &propose(&request)[0];
    assert_eq!(top.parse.display_text(), "pause sync [Budget]");
    assert!(top.listed);
    assert_eq!(top.claim, None);
    request.input = "pause plans".into();
    assert_eq!(
        propose(&request)[0].parse.display_text(),
        "pause sync [Plans]"
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

#[test]
fn it_lists_the_values_of_the_argument_the_one_command_matched_waits_for() {
    let proposals = propose(&request("exp"));
    let shown: Vec<(String, Option<String>)> = proposals
        .iter()
        .map(|proposal| (proposal.parse.display_text(), proposal.completion.clone()))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("expel [Alice]".to_owned(), Some("expel Alice".to_owned())),
            ("expel [Bob]".to_owned(), Some("expel Bob".to_owned())),
        ]
    );
    // Listed, not chosen: none runs as it stands.
    assert!(
        proposals
            .iter()
            .all(|proposal| proposal.listed && proposal.claim.is_none())
    );

    // Taking one types it, and what it says then runs.
    let top = &propose(&request("expel Bob"))[0];
    assert!(!top.listed);
    assert_eq!(
        top.claim.as_ref().unwrap()["claims"][0]["application"]["parameters"],
        json!({ "member": "did:key:bob" })
    );
}

#[test]
fn it_lists_nothing_while_the_input_could_mean_several_commands() {
    // "re" reads as rename and as remove member.
    let proposals = propose(&request("re"));
    let commands: BTreeSet<&str> = proposals
        .iter()
        .map(|proposal| proposal.command.as_str())
        .collect();
    assert!(commands.len() > 1, "{commands:?}");
    assert!(proposals.iter().all(|proposal| !proposal.listed));
}

#[test]
fn it_lists_nothing_for_an_argument_that_takes_text() {
    // Rename's subject defaults to the space; what waits is its name.
    let proposals = propose(&request("ren"));
    assert!(proposals.iter().all(|proposal| !proposal.listed));
}

#[test]
fn it_lists_the_values_picked_most_and_most_recently_first() {
    const DAY: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
    let pick = |n: usize, member: &str, time: f64| {
        row(
            &format!("pick:{n}"),
            json!({ "command": "concept:expel", "value": member, "time": time }),
        )
    };
    let order = |request: &Request| -> Vec<String> {
        propose(request)
            .iter()
            .map(|proposal| proposal.parse.display_text())
            .collect()
    };
    let mut request = request("expel");
    request.now = Some(60.0 * DAY);
    assert_eq!(order(&request), vec!["expel [Alice]", "expel [Bob]"]);

    // Picked once, Bob comes first.
    request.picks = vec![pick(0, "did:key:bob", 59.0 * DAY)];
    assert_eq!(order(&request), vec!["expel [Bob]", "expel [Alice]"]);

    // Twice as often counts for more than once, at the same time.
    request.picks.extend([
        pick(1, "did:key:alice", 59.0 * DAY),
        pick(2, "did:key:alice", 59.0 * DAY),
    ]);
    assert_eq!(order(&request), vec!["expel [Alice]", "expel [Bob]"]);

    // But picks a month old count for less than one from today.
    request.picks = vec![
        pick(0, "did:key:alice", 30.0 * DAY),
        pick(1, "did:key:alice", 30.0 * DAY),
        pick(2, "did:key:bob", 60.0 * DAY),
    ];
    assert_eq!(order(&request), vec!["expel [Bob]", "expel [Alice]"]);

    // A pick for another command says nothing about this one.
    request.picks = vec![row(
        "pick:0",
        json!({ "command": "concept:rename", "value": "did:key:bob", "time": 60.0 * DAY }),
    )];
    assert_eq!(order(&request), vec!["expel [Alice]", "expel [Bob]"]);
}

/// How the parse scales with a noun's rows. Run with
/// `cargo test --release -p tonk-intent -- --ignored --nocapture scales`.
#[test]
#[ignore = "timing, not a check"]
fn it_scales_with_candidates() {
    for count in [10, 100, 1_000, 10_000] {
        let mut request = request("");
        let source = &mut request.sources[0];
        for n in 0..count {
            let member = format!("did:key:member{n}");
            source.fragments.extend(derived(
                "concept:expel",
                "the:expel-member",
                &[member.as_str()],
            ));
            source
                .concepts
                .get_mut("concept:account")
                .expect("members")
                .rows
                .push(row(
                    &member,
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
