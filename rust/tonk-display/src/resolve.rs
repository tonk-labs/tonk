//! Wire-query construction for `<tonk-display>`.
//!
//! - [`view_by_model_query`] — resolve a view by querying a view
//!   *concept* (the predicate) constrained to a `model`, projecting
//!   `display` (and `type` when the concept declares it).
//! - [`entity_query`] — subscribe to a single entity by URI,
//!   projecting every field in the model concept's descriptor.
//! - [`view_predicate`] — the descriptor JSON of the built-in
//!   `view` concept, used as the default view predicate when the
//!   `<tonk-display>` has no `view` attribute.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_schema::query::Query;

/// Build the live view-resolution query for the new design: given a
/// view *concept* descriptor (the concept named by the `view`
/// attribute, or the built-in `view`) and a `model_entity`, find the
/// instance of that view concept whose `model` field equals
/// `model_entity`, projecting `display` (the template) and, if the
/// descriptor declares it, `type` (the render mode).
///
/// The view concept is the query predicate; `model` is the
/// constraint. The view attribute thus names a concept whose
/// `display` we query for, rather than an anchor that resolves to
/// one fixed entity.
pub fn view_by_model_query(
    view_descriptor: &Value,
    model_entity: &str,
) -> Result<Query, serde_json::Error> {
    let has_type = view_descriptor
        .get("with")
        .and_then(|w| w.get("type"))
        .is_some();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "view" } }));
    terms.insert("model".into(), json!(model_entity));
    terms.insert("display".into(), json!({ "?": { "name": "display" } }));
    if has_type {
        terms.insert("type".into(), json!({ "?": { "name": "type" } }));
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": view_descriptor }))
}

/// Build the live entity subscription query: given the model
/// concept's `descriptor_json` (raw JSON from a Phase-1 resolve)
/// and the target `entity` URI, return a query that pins `this` to
/// `entity` and projects every field in the descriptor's `with:`
/// map as a variable.
///
/// Frame size from this subscription is 0 (entity not yet on the
/// branch / removed) or 1.
pub fn entity_query(descriptor_json: &str, entity: &str) -> Result<Query, serde_json::Error> {
    let predicate: Value = serde_json::from_str(descriptor_json)?;
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(entity));
    for field in with.keys() {
        terms.insert(field.clone(), json!({ "?": { "name": field } }));
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// The descriptor of the built-in `view` concept.
///
/// Two fields: `model` (entity — the concept being displayed) and
/// `display` (text — the HTML template). Used as the default view
/// predicate when `<tonk-display>` has no `view` attribute, so the
/// built-in presentation resolves without reading the concept from
/// the branch. Attribute URIs follow the `xyz.tonk.view/*`
/// namespace; this is kept in step with the bootstrap declaration
/// pinned to `tonk:view`.
pub fn view_predicate() -> Value {
    json!({
        "with": {
            "model":   { "the": "xyz.tonk.view/model",   "as": "Entity", "cardinality": "one" },
            "display": { "the": "xyz.tonk.view/display", "as": "Text",   "cardinality": "one" }
        }
    })
}

/// The descriptor of the built-in **directory** view concept
/// (`tonk:view/directory`). Same `model` field as [`view_predicate`],
/// but its template lives under `xyz.tonk.view/directory` so a model
/// can declare a detail view and a directory view independently (both
/// keyed by `model`). Used as the default view predicate when
/// `<tonk-display>` has no `entity` (directory mode). The `display`
/// term name is kept so `view_by_model_query` and the renderer read
/// the template uniformly regardless of view kind.
pub fn directory_view_predicate() -> Value {
    json!({
        "with": {
            "model":   { "the": "xyz.tonk.view/model",     "as": "Entity", "cardinality": "one" },
            "display": { "the": "xyz.tonk.view/directory", "as": "Text",   "cardinality": "one" }
        }
    })
}

/// Build the live **directory** subscription query: like
/// [`entity_query`] but with `this` left as a variable instead of
/// pinned, so the query matches *every* instance of the model. The
/// worker emits one flat row per (instance, many-value) tuple; the
/// caller groups them by `this`. Used when `<tonk-display>` has no
/// `entity` (directory mode).
pub fn instances_query(descriptor_json: &str) -> Result<Query, serde_json::Error> {
    let predicate: Value = serde_json::from_str(descriptor_json)?;
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    for field in with.keys() {
        terms.insert(field.clone(), json!({ "?": { "name": field } }));
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// True if `s` looks like an entity URI (contains `:`) rather than
/// a bookmark name.
pub fn looks_like_uri(s: &str) -> bool {
    s.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn the_view_predicate_has_no_name_field() {
        let p = view_predicate();
        let with = p.get("with").and_then(|v| v.as_object()).expect("with");
        assert!(with.contains_key("model"));
        assert!(with.contains_key("display"));
        assert!(
            !with.contains_key("name"),
            "the built-in view concept is keyed by (concept, model), not a `name` field"
        );
    }

    #[dialog_common::test]
    fn it_builds_an_entity_query_pinning_this() {
        let descriptor = r#"{"with":{
            "message": { "the": "greeting/message", "as": "Text", "cardinality": "one" }
        }}"#;
        let q = entity_query(descriptor, "did:key:zGreeting").expect("entity_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("did:key:zGreeting"),
        );
    }

    #[dialog_common::test]
    fn it_projects_every_descriptor_field_in_the_entity_query() {
        let descriptor = r#"{"with":{
            "message":   { "the": "greeting/message",   "as": "Text", "cardinality": "one" },
            "recipient": { "the": "greeting/recipient", "as": "Text", "cardinality": "one" }
        }}"#;
        let q = entity_query(descriptor, "did:key:zGreeting").expect("entity_query");
        assert!(q.terms.contains("message"));
        assert!(q.terms.contains("recipient"));
    }

    #[dialog_common::test]
    fn it_distinguishes_uri_from_bookmark() {
        assert!(looks_like_uri("did:key:zAlice"));
        assert!(looks_like_uri("concept:abc"));
        assert!(!looks_like_uri("greeting"));
    }

    #[dialog_common::test]
    fn it_builds_a_view_by_model_query_constraining_model() {
        // The view concept is the predicate; `model` is pinned to
        // the subject's model entity and `display` flows back.
        let predicate = view_predicate();
        let q = view_by_model_query(&predicate, "concept:zCounter").expect("view_by_model_query");
        let model = q.terms.get("model").expect("model term");
        assert_eq!(
            serde_json::to_value(model).unwrap(),
            json!("concept:zCounter"),
        );
        let display = q.terms.get("display").expect("display term");
        assert_eq!(
            serde_json::to_value(display).unwrap(),
            json!({ "?": { "name": "display" } }),
        );
        // The built-in view predicate has no `type` field, so the
        // query doesn't project one.
        assert!(!q.terms.contains("type"));
    }

    #[dialog_common::test]
    fn it_projects_type_when_the_view_concept_declares_it() {
        // A custom view concept that adds a `type` field gets `type`
        // projected so the render-mode fork can read it.
        let predicate = json!({
            "with": {
                "model":   { "the": "xyz.tonk.view/model",   "as": "Entity", "cardinality": "one" },
                "type":    { "the": "xyz.tonk.view/type",    "as": "Text",   "cardinality": "one" },
                "display": { "the": "xyz.tonk.view/display", "as": "Text",   "cardinality": "one" }
            }
        });
        let q = view_by_model_query(&predicate, "concept:zCounter").expect("view_by_model_query");
        let typ = q.terms.get("type").expect("type term");
        assert_eq!(
            serde_json::to_value(typ).unwrap(),
            json!({ "?": { "name": "type" } }),
        );
    }
}
