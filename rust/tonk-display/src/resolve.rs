//! Wire-query construction for `<tonk-display>`.
//!
//! Three builders:
//!
//! - [`name_target_query`] — resolve a view's `id:` name to the
//!   entity it currently points at (so re-asserting the anchor
//!   always resolves to the latest view).
//! - [`view_fields_query`] — subscribe to that view entity,
//!   projecting `model` and `display`.
//! - [`entity_query`] — subscribe to a single entity by URI,
//!   projecting every field in the model concept's descriptor.
//! - [`view_predicate`] — the predicate JSON of the `view` concept
//!   that the worker dispatches against. Hardcoded; this is the
//!   concept of *concepts named "view"*, not the concept of any
//!   particular view.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_schema::query::Query;

/// Resolve a view's `id:` name to the entity it currently points
/// at. A view is published under an anchor name (`view!:
/// &book-dashboard`); this pins `this` to that name URI and reads
/// back the `dialog.name/referent` claim (cardinality one) that the
/// `Name` concept stores its target in.
/// Re-asserting the anchor re-points the name, so the resolved
/// entity is always the latest — older view entities linger
/// unreferenced, never resolved.
pub fn name_target_query(name_uri: &str) -> Result<Query, serde_json::Error> {
    let predicate = json!({
        "with": {
            "entity": { "the": "dialog.name/referent", "as": "Entity", "cardinality": "one" }
        }
    });
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(name_uri));
    terms.insert("entity".into(), json!({ "?": { "name": "entity" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

/// Build the live `view` subscription pinned to a specific view
/// entity (the target of [`name_target_query`]), projecting `model`
/// and `display`. Exactly one row, so there is no `(model, name)`
/// ambiguity — the view to render is the one its name points at.
pub fn view_fields_query(view_entity: &str) -> Result<Query, serde_json::Error> {
    let predicate = view_predicate();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(view_entity));
    terms.insert("model".into(), json!({ "?": { "name": "model" } }));
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
/// Two fields: `model` (entity — the concept being displayed) and
/// `display` (text — the HTML template). A view is identified by
/// its anchor name (`view!: &book-dashboard`), not by a `name`
/// field, so re-asserting the anchor replaces it in place. Attribute
/// URIs follow the `xyz.tonk.view/*` namespace.
pub fn view_predicate() -> Value {
    json!({
        "with": {
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
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_resolves_a_view_name_to_its_target_entity() {
        // A view is published under an `id:` name; the lookup pins
        // `this` to that name URI and projects the `entity` it
        // currently points at, so re-asserting (which re-points the
        // name) always resolves to the latest view entity.
        let q = name_target_query("id:book-dashboard").expect("name_target_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("id:book-dashboard")
        );
        let entity = q.terms.get("entity").expect("entity term");
        assert_eq!(
            serde_json::to_value(entity).unwrap(),
            json!({ "?": { "name": "entity" } })
        );
        // The target is carried by `dialog.name/referent` — the
        // attribute `tonk_core::meta::Name` (and `lookup_named_entity`)
        // store a name's current target in.
        let predicate = serde_json::to_value(&q.predicate).unwrap();
        assert_eq!(
            predicate["with"]["entity"]["the"],
            json!("dialog.name/referent")
        );
    }

    #[dialog_common::test]
    fn it_builds_a_view_fields_query_pinned_to_the_view_entity() {
        let q = view_fields_query("did:key:zView").expect("view_fields_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(serde_json::to_value(this).unwrap(), json!("did:key:zView"));
        // `model` and `display` flow back as variables.
        for field in ["model", "display"] {
            let term = q.terms.get(field).unwrap_or_else(|| panic!("{field} term"));
            assert_eq!(
                serde_json::to_value(term).unwrap(),
                json!({ "?": { "name": field } })
            );
        }
    }

    #[dialog_common::test]
    fn the_view_predicate_has_no_name_field() {
        let p = view_predicate();
        let with = p.get("with").and_then(|v| v.as_object()).expect("with");
        assert!(with.contains_key("model"));
        assert!(with.contains_key("display"));
        assert!(
            !with.contains_key("name"),
            "view identity moved to the anchor name; the `name` field is gone"
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
    fn it_builds_views_for_model_pinning_model_only() {
        let q = views_for_model_query("concept:zGreeting").expect("views_for_model_query");
        let model = q.terms.get("model").expect("model term");
        assert_eq!(
            serde_json::to_value(model).unwrap(),
            json!("concept:zGreeting"),
        );
        // `display` (and `this`) stay variables so the frame delivers
        // one row per view of the model; slides key off the view
        // entity since views no longer carry a `name` field.
        let display = q.terms.get("display").expect("display term");
        assert_eq!(
            serde_json::to_value(display).unwrap(),
            json!({ "?": { "name": "display" } }),
        );
        assert!(!q.terms.contains("name"));
    }
}
