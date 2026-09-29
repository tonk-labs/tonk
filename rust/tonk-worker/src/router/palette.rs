//! The `<tonk-palette>` element's reads, against a space seeded with the
//! core library, through to running what it proposes.
//!
//! The element (`profile.yaml`) subscribes with inline descriptors. The
//! bodies below are copies of the ones it sends, kept in step by hand the
//! way `tonk_template::resolve` mirrors the view predicate: if the library's
//! palette schema moves, this test is what notices. Native, on the same
//! fixture as the other native router tests.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::repository::profile_library_tests::test_state;
use super::{AppState, api_router_with_state};
use tonk_palette::{ConceptRows, Row, Source};

const CORE: &str = include_str!("../../../tonk-core/assets/library/core.yaml");

fn var(name: &str) -> Value {
    json!({ "?": { "name": name } })
}

fn text(the: &str, cardinality: &str) -> Value {
    json!({ "the": the, "as": "Text", "cardinality": cardinality })
}

fn entity(the: &str, optional: bool) -> Value {
    let mut field = json!({ "the": the, "as": "Entity", "cardinality": "one" });
    if optional {
        field["optional"] = json!(true);
    }
    field
}

async fn send(app: &Router, method: &str, uri: &str, kind: &str, body: String) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .method(method)
                .header("content-type", kind)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{method} {uri}: {status} {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// One query as the element sends it, answered by the reactor.
async fn rows(state: &AppState, key: &str, body: Value) -> Vec<Row> {
    let wire: crate::reactor::Query = serde_json::from_value(body).expect("a wire query");
    let query = wire.into_concept_query().expect("a concept query");
    let guard = state.read().await;
    let conclusions = guard
        .reactor
        .repository(key)
        .branch("main")
        .query(query)
        .perform(&guard.operator)
        .await
        .expect("the query runs");
    serde_json::from_value(serde_json::to_value(&conclusions).unwrap()).unwrap()
}

/// Everything the element reads from the space branch.
async fn source(state: &AppState, key: &str) -> Source {
    let mut source = Source {
        branch: format!("main@{key}"),
        ..Source::default()
    };
    let named = |the: &str, cardinality: &str| {
        json!({
            "predicate": { "with": { "name": text(the, cardinality) } },
            "terms": { "this": var("this"), "name": var("name") }
        })
    };
    source.verbs = rows(state, key, named("xyz.tonk.palette.verb/name", "many")).await;
    source.nouns = rows(state, key, named("xyz.tonk.palette.noun/name", "many")).await;
    source.roles = rows(state, key, named("xyz.tonk.palette.role/name", "one")).await;
    source.attributes = rows(
        state,
        key,
        json!({
            "predicate": { "with": {
                "id": text("db.attribute/id", "one"),
                "type": text("db.attribute/type", "one")
            } },
            "terms": { "this": var("this"), "id": var("id"), "type": var("type") }
        }),
    )
    .await;
    source.arguments = rows(
        state,
        key,
        json!({
            "predicate": { "with": {
                "command": entity("xyz.tonk.palette.argument/command", false),
                "field": entity("xyz.tonk.palette.argument/field", false),
                "role": entity("xyz.tonk.palette.argument/role", false),
                "noun": entity("xyz.tonk.palette.argument/noun", true)
            } },
            "terms": {
                "this": var("this"), "command": var("command"), "field": var("field"),
                "role": var("role"), "noun": var("noun")
            }
        }),
    )
    .await;

    let nouns: Vec<String> = source
        .arguments
        .iter()
        .filter_map(|row| row.fields.get("noun")?.as_str().map(str::to_owned))
        .collect();
    for concept in nouns {
        // The descriptor, the way `<tonk-display>` resolves a model.
        let described = rows(
            state,
            key,
            json!({
                "predicate": { "with": {
                    "concept": { "the": "db.meta/concept", "as": "Entity", "cardinality": "one" },
                    "name": text("db.meta/name", "one"),
                    "description": text("db.meta/description", "one"),
                    "source": text("db.meta/source", "one"),
                    "transient": { "the": "dialog.concept/transient", "as": "Boolean", "cardinality": "one" }
                } },
                "terms": { "this": concept, "name": var("name"), "source": var("source") }
            }),
        )
        .await;
        let descriptor: Value = described
            .iter()
            .find_map(|row| row.fields.get("source")?.as_str())
            .map(|source| serde_json::from_str(source).unwrap())
            .unwrap_or_else(|| panic!("{concept} has a descriptor"));
        let mut terms = json!({ "this": var("this") });
        for field in descriptor["with"].as_object().unwrap().keys() {
            terms[field] = var(field);
        }
        let instances = rows(
            state,
            key,
            json!({ "predicate": descriptor, "terms": terms }),
        )
        .await;
        let facets = rows(
            state,
            key,
            json!({
                "predicate": { "with": { "show": {
                    "the": { "domain": "xyz.tonk.view", "keyed": "dictionary" },
                    "as": "Text", "cardinality": "one"
                } } },
                "terms": { "this": concept, "show": var("show"), "show/key": var("show/key") }
            }),
        )
        .await;
        let label = facets
            .iter()
            .find(|row| row.fields.get("show/key").and_then(Value::as_str) == Some("label"))
            .and_then(|row| row.fields.get("show")?.as_str().map(str::to_owned));
        source.concepts.insert(
            concept,
            ConceptRows {
                label,
                rows: instances,
            },
        );
    }
    source
}

fn request(input: &str, subject: &str, source: &Source) -> tonk_palette::Request {
    tonk_palette::Request {
        input: input.into(),
        max: 8,
        context: dialog_palette::Context {
            selection: None,
            this: Some(dialog_palette::Selection {
                text: String::new(),
                entity: Some(subject.into()),
            }),
        },
        sources: vec![source.clone()],
        memory: Vec::new(),
    }
}

#[dialog_common::test]
async fn it_reads_the_palette_from_a_seeded_space_and_runs_what_it_proposes() {
    let (app, state, _lsp) = api_router_with_state(test_state().await);
    let created = send(
        &app,
        "PUT",
        "/api/repository/palette",
        "application/json",
        "{}".into(),
    )
    .await;
    let key = created["name"].as_str().unwrap().to_owned();
    let subject = created["subject"].as_str().unwrap().to_owned();
    let evaluate = format!("/api/repository/{key}/branch/main/evaluate");
    send(&app, "POST", &evaluate, "application/yaml", CORE.into()).await;
    send(
        &app,
        "POST",
        &evaluate,
        "application/yaml",
        format!("tonk/repository!:\n  this: {subject}\n  name: \"Budget\"\n"),
    )
    .await;

    let source = source(&state, &key).await;
    let words: Vec<&str> = source
        .verbs
        .iter()
        .filter_map(|row| row.fields.get("name")?.as_str())
        .collect();
    for word in ["expel", "remove member", "rename"] {
        assert!(words.contains(&word), "{word} is a verb: {words:?}");
    }

    // An empty argument is labelled by its noun, and nothing runs yet.
    let proposals = tonk_palette::propose(&request("expel", &subject, &source));
    let expel = proposals
        .iter()
        .find(|proposal| proposal.parse.name == "expel")
        .expect("expel is proposed");
    assert_eq!(expel.parse.display_text(), "expel (member)");
    assert_eq!(expel.claim, None);

    // "this" is the space; the goal is typed text.
    let proposals = tonk_palette::propose(&request("rename this to Q3", &subject, &source));
    let top = &proposals[0];
    assert_eq!(top.parse.display_text(), "rename [Budget] to [Q3]");
    assert_eq!(top.branch, format!("main@{key}"));
    let claim = top.claim.clone().expect("a complete parse runs");

    send(
        &app,
        "POST",
        &format!("/api/repository/{key}/branch/main/transact"),
        "application/json",
        claim.to_string(),
    )
    .await;

    let renamed = rows(
        &state,
        &key,
        json!({
            "predicate": { "with": { "name": text("xyz.tonk.repo/name", "one") } },
            "terms": { "this": subject, "name": var("name") }
        }),
    )
    .await;
    assert_eq!(
        renamed
            .iter()
            .filter_map(|row| row.fields.get("name")?.as_str())
            .collect::<Vec<_>>(),
        vec!["Q3"],
        "the rename rule applied the palette's claim"
    );
}
