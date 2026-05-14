//! Wire-query construction for `<tonk-display>`.
//!
//! Three builders:
//!
//! - [`view_query`] — subscribe to the `view` row matching a given
//!   `(model, name)` pair, projecting the `display` field.
//! - [`entity_query`] — subscribe to a single entity by URI,
//!   projecting every field in the model concept's descriptor.
//! - [`view_predicate`] — the predicate JSON of the `view` concept
//!   that the worker dispatches against. Hardcoded; this is the
//!   concept of *concepts named "view"*, not the concept of any
//!   particular view.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_schema::query::Query;

/// Build the live `view` subscription query: find the `view` row
/// whose `model` field equals `model_entity` and whose `name` field
/// equals `view_name`, projecting `display` (and `this`) as
/// variables.
///
/// The predicate is the built-in `view` concept descriptor —
/// `view_predicate`. The terms map nails `model` + `name` to
/// constants and leaves `display` and `this` as variables so they
/// flow back in every frame.
pub fn view_query(model_entity: &str, view_name: &str) -> Result<Query, serde_json::Error> {
    let predicate = view_predicate();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "view" } }));
    terms.insert("model".into(), json!(model_entity));
    terms.insert("name".into(), json!(view_name));
    terms.insert("display".into(), json!({ "?": { "name": "display" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// Build the live "all views for this model" subscription query:
/// find every `view` row whose `model` field equals `model_entity`,
/// projecting `name` and `display` as variables.
///
/// Used by `<tonk-display>`'s carousel fallback (when no `view`
/// attribute is set) to enumerate the available presentations for
/// a given concept.
pub fn views_for_model_query(model_entity: &str) -> Result<Query, serde_json::Error> {
    let predicate = view_predicate();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "view" } }));
    terms.insert("model".into(), json!(model_entity));
    terms.insert("name".into(), json!({ "?": { "name": "name" } }));
    terms.insert("display".into(), json!({ "?": { "name": "display" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
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

/// The descriptor of the `view` concept itself.
///
/// Three fields: `name` (text — distinguishes views over the same
/// concept), `model` (entity — the concept being displayed), and
/// `display` (text — the HTML template). Attribute URIs follow the
/// `xyz.tonk.view/*` namespace the user's design pins.
pub fn view_predicate() -> Value {
    json!({
        "with": {
            "name":    { "the": "xyz.tonk.view/name",    "as": "Text",   "cardinality": "one" },
            "model":   { "the": "xyz.tonk.view/model",   "as": "Entity", "cardinality": "one" },
            "display": { "the": "xyz.tonk.view/display", "as": "Text",   "cardinality": "one" }
        }
    })
}

/// True if `s` looks like an entity URI (contains `:`) rather than
/// a bookmark name.
pub fn looks_like_uri(s: &str) -> bool {
    s.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_builds_a_view_query_pinning_model_and_name() {
        let q = view_query("concept:zGreeting", "basic").expect("view_query");
        let model = q.terms.get("model").expect("model term");
        let name = q.terms.get("name").expect("name term");
        assert_eq!(
            serde_json::to_value(model).unwrap(),
            json!("concept:zGreeting")
        );
        assert_eq!(serde_json::to_value(name).unwrap(), json!("basic"));
    }

    #[test]
    fn it_projects_display_as_a_variable_in_the_view_query() {
        let q = view_query("concept:zGreeting", "basic").expect("view_query");
        let display = q.terms.get("display").expect("display term");
        let v = serde_json::to_value(display).unwrap();
        // Variable terms shape as `{ "?": { "name": "display" } }`.
        assert_eq!(v, json!({ "?": { "name": "display" } }));
    }

    #[test]
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

    #[test]
    fn it_projects_every_descriptor_field_in_the_entity_query() {
        let descriptor = r#"{"with":{
            "message":   { "the": "greeting/message",   "as": "Text", "cardinality": "one" },
            "recipient": { "the": "greeting/recipient", "as": "Text", "cardinality": "one" }
        }}"#;
        let q = entity_query(descriptor, "did:key:zGreeting").expect("entity_query");
        assert!(q.terms.contains("message"));
        assert!(q.terms.contains("recipient"));
    }

    #[test]
    fn it_distinguishes_uri_from_bookmark() {
        assert!(looks_like_uri("did:key:zAlice"));
        assert!(looks_like_uri("concept:abc"));
        assert!(!looks_like_uri("greeting"));
    }

    #[test]
    fn it_builds_views_for_model_pinning_model_only() {
        let q = views_for_model_query("concept:zGreeting").expect("views_for_model_query");
        let model = q.terms.get("model").expect("model term");
        assert_eq!(
            serde_json::to_value(model).unwrap(),
            json!("concept:zGreeting"),
        );
        // `name` and `display` must remain variables so the frame
        // delivers one row per available view.
        let name = q.terms.get("name").expect("name term");
        assert_eq!(
            serde_json::to_value(name).unwrap(),
            json!({ "?": { "name": "name" } }),
        );
        let display = q.terms.get("display").expect("display term");
        assert_eq!(
            serde_json::to_value(display).unwrap(),
            json!({ "?": { "name": "display" } }),
        );
    }
}
