use serde_json::json;

use super::*;
use dialog_palette::Selection;

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
