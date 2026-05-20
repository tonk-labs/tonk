//! Wire-query builders for the three layout subscriptions.
//!
//! Unlike `<tonk-concept>`, the layout concepts are fixed and
//! known ahead of time — `workspace`, `column`, and `tile` — so
//! there is no concept-of-concept lookup. Each builder embeds the
//! concept's `with` descriptor directly and binds one term per
//! field: a constant on the filtered field, a projection variable
//! on the rest.
//!
//! The attribute URIs (`xyz.tonk.layout/*`) must match the
//! concept definitions declared on the branch (see
//! `/plan/tonk-layout.md`).
//!
//! Everything here is target-independent and unit-tested
//! natively.

use serde_json::json;
use tonk_schema::query::Query;

/// Build a query for the `workspace` row whose `name` equals
/// `name`.
///
/// Projects `focus` so the fold can read the focused-tile
/// pointer. Reads back as `(this, name, focus)`.
pub fn workspace_query(name: &str) -> Query {
    let body = json!({
        "terms": {
            "this":  { "?": { "name": "this" } },
            "name":  name,
            "focus": { "?": { "name": "focus" } }
        },
        "predicate": {
            "with": {
                "name":  { "the": "xyz.tonk.layout/workspace-name",  "as": "Text",   "cardinality": "one" },
                "focus": { "the": "xyz.tonk.layout/workspace-focus", "as": "Entity", "cardinality": "one" }
            }
        }
    });
    serde_json::from_value(body).expect("workspace query body is well-formed")
}

/// Build a query for every `column` row whose `workspace` equals
/// the workspace entity URI.
///
/// Reads back as `(this, order, width)`; `workspace` is pinned to
/// the constant so only this workspace's columns surface.
pub fn columns_query(workspace_entity: &str) -> Query {
    let body = json!({
        "terms": {
            "this":      { "?": { "name": "this" } },
            "workspace": workspace_entity,
            "order":     { "?": { "name": "order" } },
            "width":     { "?": { "name": "width" } }
        },
        "predicate": {
            "with": {
                "workspace": { "the": "xyz.tonk.layout/column-workspace", "as": "Entity", "cardinality": "one" },
                "order":     { "the": "xyz.tonk.layout/column-order",     "as": "Float",  "cardinality": "one" },
                "width":     { "the": "xyz.tonk.layout/column-width",     "as": "Float",  "cardinality": "one" }
            }
        }
    });
    serde_json::from_value(body).expect("columns query body is well-formed")
}

/// Build a query for every `tile` row whose `workspace` equals
/// the workspace entity URI.
///
/// Tiles carry a denormalized `workspace` reference (a copy of
/// their column's `workspace`), so a single subscription returns
/// exactly this workspace's tiles — no one-per-column fan-out and
/// no cross-workspace rows for the fold to discard.
/// [`crate::model::Layout::fold`] attaches each tile to its
/// parent column by the `column` field.
///
/// Reads back the full content descriptor
/// `(this, column, order, height, entity, view, model)`;
/// `workspace` is pinned to the constant.
pub fn tiles_query(workspace_entity: &str) -> Query {
    let body = json!({
        "terms": {
            "this":      { "?": { "name": "this" } },
            "workspace": workspace_entity,
            "column":    { "?": { "name": "column" } },
            "order":     { "?": { "name": "order" } },
            "height":    { "?": { "name": "height" } },
            "entity":    { "?": { "name": "entity" } },
            "view":      { "?": { "name": "view" } },
            "model":     { "?": { "name": "model" } }
        },
        "predicate": {
            "with": {
                "workspace": { "the": "xyz.tonk.layout/tile-workspace", "as": "Entity", "cardinality": "one" },
                "column":    { "the": "xyz.tonk.layout/tile-column",    "as": "Entity", "cardinality": "one" },
                "order":     { "the": "xyz.tonk.layout/tile-order",     "as": "Float",  "cardinality": "one" },
                "height":    { "the": "xyz.tonk.layout/tile-height",    "as": "Float",  "cardinality": "one" },
                "entity":    { "the": "xyz.tonk.layout/tile-entity",    "as": "Entity", "cardinality": "one" },
                "view":      { "the": "xyz.tonk.layout/tile-view",      "as": "Text",   "cardinality": "one" },
                "model":     { "the": "xyz.tonk.layout/tile-model",     "as": "Text",   "cardinality": "one" }
            }
        }
    });
    serde_json::from_value(body).expect("tiles query body is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize a term back to JSON so a test can assert whether
    /// it is a pinned constant or a projection variable.
    fn term(query: &Query, name: &str) -> serde_json::Value {
        let term = query.terms.get(name).expect("term present");
        serde_json::to_value(term).expect("term serializes")
    }

    #[test]
    fn it_pins_the_workspace_query_by_name() {
        let query = workspace_query("default");
        assert_eq!(term(&query, "name"), json!("default"));
    }

    #[test]
    fn it_projects_focus_on_the_workspace_query() {
        let query = workspace_query("default");
        // A projection variable is an object, not a bare string.
        assert!(term(&query, "focus").is_object());
        assert!(term(&query, "this").is_object());
    }

    #[test]
    fn it_pins_the_columns_query_by_workspace_entity() {
        let query = columns_query("did:key:zWorkspace");
        assert_eq!(term(&query, "workspace"), json!("did:key:zWorkspace"));
    }

    #[test]
    fn it_projects_order_and_width_on_the_columns_query() {
        let query = columns_query("did:key:zWorkspace");
        assert!(term(&query, "order").is_object());
        assert!(term(&query, "width").is_object());
    }

    #[test]
    fn it_pins_the_tiles_query_by_workspace_entity() {
        let query = tiles_query("did:key:zWorkspace");
        assert_eq!(term(&query, "workspace"), json!("did:key:zWorkspace"));
    }

    #[test]
    fn it_projects_the_full_content_descriptor_on_the_tiles_query() {
        let query = tiles_query("did:key:zWorkspace");
        for field in [
            "column", "order", "height", "entity", "view", "model", "this",
        ] {
            assert!(
                term(&query, field).is_object(),
                "{field} should be a projection variable"
            );
        }
    }
}
