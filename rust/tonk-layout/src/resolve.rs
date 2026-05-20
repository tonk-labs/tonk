//! Wire-query builders for the three live subscriptions.
//!
//! - [`workspace_query`] — find the workspace whose `name` field
//!   equals the attribute value, projecting `focus`.
//! - [`columns_query`] — find every column whose `workspace` field
//!   equals the resolved workspace entity URI, projecting `order`
//!   and `width`.
//! - [`tiles_query`] — find every tile on the branch, projecting all
//!   its fields. The fold drops tiles whose `column` ref is missing
//!   from the columns frame, which is how we restrict to "tiles
//!   belonging to this workspace" without a join. v1 trade-off: a
//!   branch with many workspaces ships tiles for all of them through
//!   this stream.

// Wasm-side consumer arrives with the read path; until then the
// builders are exercised by native tests only.
#![allow(dead_code)]

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
    match (space, branch) {
        (None, None) => "/query".to_owned(),
        _ => format!(
            "/api/repository/{}/branch/{}/query",
            space.unwrap_or("home"),
            branch.unwrap_or("main"),
        ),
    }
}

/// Build the live workspace subscription query. Pins `name` to the
/// caller's value; projects `this` (entity URI) and `focus` (the
/// tile entity, when set).
pub fn workspace_query(name: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    terms.insert("name".into(), json!(name));
    terms.insert("focus".into(), json!({ "?": { "name": "focus" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": workspace_predicate() }))
}

/// Build the live columns subscription query. Pins `workspace` to
/// the resolved workspace entity URI; projects `this`, `order`, and
/// `width`.
pub fn columns_query(workspace_entity: &str) -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    terms.insert("this".into(), json!({ "?": { "name": "this" } }));
    terms.insert("workspace".into(), json!(workspace_entity));
    terms.insert("order".into(), json!({ "?": { "name": "order" } }));
    terms.insert("width".into(), json!({ "?": { "name": "width" } }));
    serde_json::from_value(json!({ "terms": terms, "predicate": column_predicate() }))
}

/// Build the live tiles subscription query. Leaves every field as a
/// variable so the worker delivers every tile on the branch; the
/// fold drops orphans whose `column` isn't in the columns frame.
pub fn tiles_query() -> Result<Query, serde_json::Error> {
    let mut terms: IndexMap<String, Value> = IndexMap::new();
    for field in [
        "this", "column", "order", "height", "kind", "entity", "view", "model",
    ] {
        terms.insert(field.into(), json!({ "?": { "name": field } }));
    }
    serde_json::from_value(json!({ "terms": terms, "predicate": tile_predicate() }))
}

/// The workspace concept descriptor — the predicate the worker
/// dispatches against to find workspace rows.
fn workspace_predicate() -> Value {
    json!({
        "with": {
            "name":  { "the": "xyz.tonk.layout/workspace-name",  "as": "Text",   "cardinality": "one" },
            "focus": { "the": "xyz.tonk.layout/workspace-focus", "as": "Entity", "cardinality": "one" }
        }
    })
}

/// The column concept descriptor.
fn column_predicate() -> Value {
    json!({
        "with": {
            "workspace": { "the": "xyz.tonk.layout/column-workspace", "as": "Entity", "cardinality": "one" },
            "order":     { "the": "xyz.tonk.layout/column-order",     "as": "Text",   "cardinality": "one" },
            "width":     { "the": "xyz.tonk.layout/column-width",     "as": "Float",  "cardinality": "one" }
        }
    })
}

/// The tile concept descriptor.
fn tile_predicate() -> Value {
    json!({
        "with": {
            "column": { "the": "xyz.tonk.layout/tile-column", "as": "Entity", "cardinality": "one" },
            "order":  { "the": "xyz.tonk.layout/tile-order",  "as": "Text",   "cardinality": "one" },
            "height": { "the": "xyz.tonk.layout/tile-height", "as": "Float",  "cardinality": "one" },
            "kind":   { "the": "xyz.tonk.layout/tile-kind",   "as": "Text",   "cardinality": "one" },
            "entity": { "the": "xyz.tonk.layout/tile-entity", "as": "Entity", "cardinality": "one" },
            "view":   { "the": "xyz.tonk.layout/tile-view",   "as": "Text",   "cardinality": "one" },
            "model":  { "the": "xyz.tonk.layout/tile-model",  "as": "Text",   "cardinality": "one" }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term_value(q: &Query, name: &str) -> Value {
        serde_json::to_value(q.terms.get(name).expect("term present")).unwrap()
    }

    #[dialog_common::test]
    fn it_pins_the_name_constant_in_the_workspace_query() {
        let q = workspace_query("default").expect("workspace_query");
        assert_eq!(term_value(&q, "name"), json!("default"));
    }

    #[dialog_common::test]
    fn it_projects_this_and_focus_as_variables_in_the_workspace_query() {
        let q = workspace_query("default").expect("workspace_query");
        assert_eq!(term_value(&q, "this"), json!({ "?": { "name": "this" } }));
        assert_eq!(term_value(&q, "focus"), json!({ "?": { "name": "focus" } }));
    }

    #[dialog_common::test]
    fn it_pins_the_workspace_constant_in_the_columns_query() {
        let q = columns_query("id:01HMW...").expect("columns_query");
        assert_eq!(term_value(&q, "workspace"), json!("id:01HMW..."));
    }

    #[dialog_common::test]
    fn it_projects_order_and_width_as_variables_in_the_columns_query() {
        let q = columns_query("id:01HMW...").expect("columns_query");
        assert_eq!(term_value(&q, "order"), json!({ "?": { "name": "order" } }));
        assert_eq!(term_value(&q, "width"), json!({ "?": { "name": "width" } }));
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
    fn it_leaves_every_field_as_a_variable_in_the_tiles_query() {
        let q = tiles_query().expect("tiles_query");
        for field in [
            "this", "column", "order", "height", "kind", "entity", "view", "model",
        ] {
            assert_eq!(
                term_value(&q, field),
                json!({ "?": { "name": field } }),
                "tile term {field} must be a variable",
            );
        }
    }
}
