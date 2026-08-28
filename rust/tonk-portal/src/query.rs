//! Wire-query construction for the no-argument bridge calls.
//!
//! `tonk.subscribe()` / `tonk.query()` with no argument stream the
//! portal's scoped entity. This builds that query from the model
//! concept's descriptor and the scoped entity URI — the same shape
//! `<tonk-display>` builds for its entity subscription, so the
//! imperative escape hatch reads exactly what the declarative path
//! would.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_worker_api::Query;

/// Build the scoped-entity query: pin `this` to `entity` and project
/// every field in the descriptor's `with:` map as a variable. The
/// descriptor is the raw JSON `<tonk-display>` resolves for the
/// model concept (`{ "with": { <field>: { the, as, cardinality } } }`)
/// and doubles as the query predicate.
pub fn entity_query(descriptor_json: &str, entity: &str) -> Result<Query, serde_json::Error> {
    let predicate: Value = serde_json::from_str(descriptor_json)?;
    let with = predicate
        .get("with")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(entity));
    for (field, spec) in &with {
        terms.insert(field.clone(), json!({ "?": { "name": field } }));
        // A keyed collection binds TWO terms — the field and its key
        // operand (`block`, `block/key`) — because an entry is a
        // `(key, value)` pair. Requesting only the field leaves the key
        // unbound and the wire fold has no pair to turn into
        // `{key: value}`, so every entry reads as an unkeyed value.
        if spec.get("the").is_some_and(Value::is_object) {
            let key = format!("{field}/key");
            terms.insert(key.clone(), json!({ "?": { "name": key } }));
        }
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": predicate }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const DESCRIPTOR: &str = r#"{"with":{
        "count": { "the": "counter/count", "as": "UnsignedInteger", "cardinality": "one" }
    }}"#;

    #[dialog_common::test]
    fn it_pins_this_to_the_scoped_entity() {
        let q = entity_query(DESCRIPTOR, "id:demo-counter").expect("entity_query");
        let this = q.terms.get("this").expect("this term");
        assert_eq!(
            serde_json::to_value(this).unwrap(),
            json!("id:demo-counter")
        );
    }

    #[dialog_common::test]
    fn it_projects_every_descriptor_field_as_a_variable() {
        let q = entity_query(DESCRIPTOR, "id:demo-counter").expect("entity_query");
        let count = q.terms.get("count").expect("count term");
        assert_eq!(
            serde_json::to_value(count).unwrap(),
            json!({ "?": { "name": "count" } }),
        );
    }
}
