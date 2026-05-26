// Consumed only by the wasm-gated `element` and this module's own
// native tests; a native non-test lib build reaches none of it.
#![allow(dead_code)]

//! Wire-query builders for the three live subscriptions.
//!
//! - [`workspace_query`] — find the workspace whose `name` field
//!   equals the attribute value, projecting `this`.
//! - [`focus_query`] — for a known workspace entity, project its
//!   `focus` field. Returns zero or one row depending on whether the
//!   workspace has a focus claim yet.
//! - [`tiles_query`] — find every tile on the branch matching the
//!   universal tile predicate; the fold drops tiles whose `workspace`
//!   ref doesn't match. v1 trade-off: a branch with many workspaces
//!   ships tiles for all of them through this stream.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tonk_schema::query::Query;

/// Build the `/query` URL the SSE subscriptions POST against.
///
/// When neither `space` nor `branch` is set, the element runs inside
/// a host that has already scoped the request — a relative `/query`
/// path. Otherwise the full `/api/repository/<space>/branch/<branch>/query`
/// path routes the request to a specific repository / branch via the
/// worker's REST endpoint.
pub fn query_url(space: Option<&str>, branch: Option<&str>) -> String {
    endpoint_url(space, branch, "query")
}

/// Build the `/evaluate` URL the writer POSTs mutation documents to.
pub fn evaluate_url(space: Option<&str>, branch: Option<&str>) -> String {
    endpoint_url(space, branch, "evaluate")
}

fn endpoint_url(space: Option<&str>, branch: Option<&str>, route: &str) -> String {
    match (space, branch) {
        (None, None) => format!("/{route}"),
        _ => format!(
            "/api/repository/{}/branch/{}/{route}",
            space.unwrap_or("home"),
            branch.unwrap_or("main"),
        ),
    }
}

/// Build the live workspace subscription query. Pins `name` to the
/// caller's value; projects `this` (entity URI).
///
/// `focus` is intentionally absent from the predicate: a fresh
/// workspace has no focus claim yet, and a cardinality-one predicate
/// term is a filter requirement — including it would return zero
/// rows. Focus is fetched via [`focus_query`] once the workspace
/// entity is known.
pub fn workspace_query(name: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    terms.insert("name".into(), json!(name));
    serde_json::from_value(json!({
        "terms": terms,
        "predicate": {
            "with": {
                "name": { "the": "xyz.tonk.layout/workspace-name", "as": "Text", "cardinality": "one" }
            }
        }
    }))
}

/// Build the focus subscription query for a known workspace entity.
/// Returns zero rows when the workspace has no focus, one row with
/// the focused tile URI when it does.
pub fn focus_query(workspace_entity: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!(workspace_entity));
    terms.insert("focus".into(), json!({ "?": { "name": "focus" } }));
    serde_json::from_value(json!({
        "terms": terms,
        "predicate": {
            "with": {
                "focus": { "the": "xyz.tonk.layout/workspace-focus", "as": "Entity", "cardinality": "one" }
            }
        }
    }))
}

/// Build the live tiles subscription query. Leaves every field as a
/// variable so the worker delivers every tile on the branch; the
/// fold filters to those whose `workspace` matches the resolved
/// workspace entity. `entity` is the only optional field — predicate
/// declares it without cardinality so a tile lacking an `entity`
/// claim (a concept-list tile) still matches.
pub fn tiles_query() -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    for field in ["this", "workspace", "order", "entity", "view", "model"] {
        terms.insert(field.into(), json!({ "?": { "name": field } }));
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": tile_predicate() }))
}

/// The tile concept descriptor. `entity` is declared without
/// cardinality so tiles without an `entity` claim still match.
fn tile_predicate() -> Value {
    json!({
        "with": {
            "workspace": { "the": "xyz.tonk.layout/tile-workspace", "as": "Entity", "cardinality": "one" },
            "order":     { "the": "xyz.tonk.layout/tile-order",     "as": "Text",   "cardinality": "one" },
            "view":      { "the": "xyz.tonk.layout/tile-view",      "as": "Text",   "cardinality": "one" },
            "model":     { "the": "xyz.tonk.layout/tile-model",     "as": "Text",   "cardinality": "one" },
            "entity":    { "the": "xyz.tonk.layout/tile-entity",    "as": "Entity" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    fn term_value(q: &Query, name: &str) -> Value {
        serde_json::to_value(q.terms.get(name).expect("term present")).unwrap()
    }

    #[dialog_common::test]
    fn it_pins_the_name_constant_in_the_workspace_query() {
        let q = workspace_query("default").expect("workspace_query");
        assert_eq!(term_value(&q, "name"), json!("default"));
    }

    #[dialog_common::test]
    fn it_projects_this_as_a_variable_in_the_workspace_query() {
        let q = workspace_query("default").expect("workspace_query");
        assert_eq!(term_value(&q, "this"), json!({ "?": { "name": "this" } }));
    }

    #[dialog_common::test]
    fn it_omits_focus_from_the_workspace_query_predicate() {
        // Cardinality-one predicate terms are filter requirements:
        // a fresh workspace has no focus claim, so including focus
        // would return zero rows. Focus is fetched separately via
        // `focus_query`.
        let q = workspace_query("default").expect("workspace_query");
        assert!(q.terms.get("focus").is_none());
    }

    #[dialog_common::test]
    fn it_pins_workspace_entity_in_the_focus_query() {
        let q = focus_query("id:01HMW...").expect("focus_query");
        assert_eq!(term_value(&q, "this"), json!("id:01HMW..."));
        assert_eq!(term_value(&q, "focus"), json!({ "?": { "name": "focus" } }));
    }

    #[dialog_common::test]
    fn it_leaves_every_field_as_a_variable_in_the_tiles_query() {
        let q = tiles_query().expect("tiles_query");
        for field in ["this", "workspace", "order", "entity", "view", "model"] {
            assert_eq!(
                term_value(&q, field),
                json!({ "?": { "name": field } }),
                "tile term {field} must be a variable",
            );
        }
    }

    #[dialog_common::test]
    fn it_routes_to_relative_query_when_no_attributes_are_set() {
        assert_eq!(query_url(None, None), "/query");
    }

    #[dialog_common::test]
    fn it_routes_to_the_repository_endpoint_when_an_attribute_is_set() {
        assert_eq!(
            query_url(Some("home"), Some("main")),
            "/api/repository/home/branch/main/query",
        );
    }

    #[dialog_common::test]
    fn it_fills_in_default_space_when_only_branch_is_set() {
        assert_eq!(
            query_url(None, Some("feature-x")),
            "/api/repository/home/branch/feature-x/query",
        );
    }

    #[dialog_common::test]
    fn it_fills_in_default_branch_when_only_space_is_set() {
        assert_eq!(
            query_url(Some("staging"), None),
            "/api/repository/staging/branch/main/query",
        );
    }

    #[dialog_common::test]
    fn it_routes_evaluate_to_the_repository_endpoint() {
        assert_eq!(
            evaluate_url(Some("home"), Some("main")),
            "/api/repository/home/branch/main/evaluate",
        );
    }

    #[dialog_common::test]
    fn it_routes_evaluate_to_relative_when_no_attributes_are_set() {
        assert_eq!(evaluate_url(None, None), "/evaluate");
    }
}
