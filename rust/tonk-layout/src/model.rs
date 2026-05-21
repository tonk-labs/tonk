//! The in-memory layout tree the reconciler patches into the DOM.
//!
//! [`Layout`] / [`Column`] / [`Tile`] are the structured form of
//! "the latest frame of each subscription, folded together and
//! sorted." Subscriptions deliver flat lists of [`Conclusion`]
//! rows; the reconciler wants a tree shape with parent/child
//! relationships resolved and siblings in lex order so it can
//! diff against the prior frame's tree.

// Wasm-side consumer arrives with `reconcile.rs`; until then the
// types are exercised by native tests only.
#![allow(dead_code)]

use tonk_schema::conclusion::Conclusion;

/// The folded workspace tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Workspace entity URI.
    pub workspace: String,
    /// Focused tile entity URI, if any.
    pub focus: Option<String>,
    /// Columns in lex `order` order.
    pub columns: Vec<Column>,
}

/// A column in the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Column entity URI.
    pub entity: String,
    /// Lex ordering key within the workspace.
    pub order: String,
    /// Width as a fraction of the viewport.
    pub width: f64,
    /// Tiles in lex `order` order.
    pub tiles: Vec<Tile>,
}

/// A tile within a column.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Tile entity URI.
    pub entity: String,
    /// Lex ordering key within the column.
    pub order: String,
    /// Height as a fraction of the column.
    pub height: f64,
    /// Content kind; v1 recognises `"display"`.
    pub kind: String,
    /// For `kind: "display"` — the entity the tile renders.
    pub display_entity: Option<String>,
    /// For `kind: "display"` — the view name.
    pub display_view: Option<String>,
    /// For `kind: "display"` — the model name.
    pub display_model: Option<String>,
}

/// Fold the three subscription frames into a sorted [`Layout`].
///
/// Returns `None` when no workspace row matches — callers render
/// state `empty` (or trigger lazy-create on first interaction).
/// Tiles whose `column` doesn't appear in the columns frame are
/// dropped: a column-resize commit can briefly arrive before the
/// re-asserted tile rows, and a stray tile-without-parent has no
/// sensible slot.
pub fn fold_layout(
    workspace_frame: &[Conclusion],
    columns_frame: &[Conclusion],
    tiles_frame: &[Conclusion],
) -> Option<Layout> {
    let ws_row = workspace_frame.first()?;
    let workspace = ws_row.this.clone();
    let focus = read_str(ws_row, "focus").map(str::to_owned);

    // Collect columns into a map keyed by entity URI; we'll attach
    // tiles to them in the second pass and only keep columns whose
    // `order` field is present (it's required for placement).
    let mut columns: std::collections::HashMap<String, Column> =
        std::collections::HashMap::with_capacity(columns_frame.len());
    for row in columns_frame {
        let Some(order) = read_str(row, "order") else {
            continue;
        };
        let width = read_f64(row, "width").unwrap_or(1.0);
        columns.insert(
            row.this.clone(),
            Column {
                entity: row.this.clone(),
                order: order.to_owned(),
                width,
                tiles: Vec::new(),
            },
        );
    }

    // Bucket tiles into their parent column; drop orphans.
    for row in tiles_frame {
        let Some(parent) = read_str(row, "column") else {
            continue;
        };
        let Some(order) = read_str(row, "order") else {
            continue;
        };
        let Some(column) = columns.get_mut(parent) else {
            continue;
        };
        column.tiles.push(Tile {
            entity: row.this.clone(),
            order: order.to_owned(),
            height: read_f64(row, "height").unwrap_or(1.0),
            kind: read_str(row, "kind").unwrap_or("display").to_owned(),
            display_entity: read_str(row, "entity").map(str::to_owned),
            display_view: read_str(row, "view").map(str::to_owned),
            display_model: read_str(row, "model").map(str::to_owned),
        });
    }

    // Sort tiles within each column, then collect columns and sort.
    let mut columns: Vec<Column> = columns.into_values().collect();
    for column in columns.iter_mut() {
        column.tiles.sort_by(|a, b| a.order.cmp(&b.order));
    }
    columns.sort_by(|a, b| a.order.cmp(&b.order));

    Some(Layout {
        workspace,
        focus,
        columns,
    })
}

fn read_str<'a>(c: &'a Conclusion, name: &str) -> Option<&'a str> {
    c.fields.get(name).and_then(|v| v.as_str())
}

fn read_f64(c: &Conclusion, name: &str) -> Option<f64> {
    c.fields.get(name).and_then(|v| v.as_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a [`Conclusion`] from an entity URI and a JSON-object
    /// of fields. Keeps the tests' setup terse — the alternative is
    /// constructing `BTreeMap` literals inline at every call site.
    fn conclusion(this: &str, fields: serde_json::Value) -> Conclusion {
        let obj = fields
            .as_object()
            .expect("fields must be a JSON object")
            .clone();
        Conclusion {
            this: this.to_owned(),
            fields: obj.into_iter().collect::<BTreeMap<_, _>>(),
        }
    }

    fn workspace(this: &str, focus: Option<&str>) -> Conclusion {
        let mut f = serde_json::Map::new();
        if let Some(t) = focus {
            f.insert("focus".into(), json!(t));
        }
        conclusion(this, serde_json::Value::Object(f))
    }

    fn column(this: &str, workspace_uri: &str, order: &str, width: f64) -> Conclusion {
        conclusion(
            this,
            json!({ "workspace": workspace_uri, "order": order, "width": width }),
        )
    }

    fn tile(this: &str, column_uri: &str, order: &str, height: f64) -> Conclusion {
        conclusion(
            this,
            json!({
                "column": column_uri,
                "order": order,
                "height": height,
                "kind": "display",
                "entity": "id:ent",
                "view": "card",
                "model": "person",
            }),
        )
    }

    #[dialog_common::test]
    fn it_returns_none_when_the_workspace_frame_is_empty() {
        // No matching workspace → caller renders the "empty" state
        // (or kicks off lazy-create on first interaction).
        let layout = fold_layout(&[], &[], &[]);
        assert!(layout.is_none());
    }

    #[dialog_common::test]
    fn it_folds_a_simple_two_column_strip() {
        let ws = "id:ws";
        let c_a = "id:col-a";
        let c_b = "id:col-b";
        let layout = fold_layout(
            &[workspace(ws, None)],
            &[column(c_a, ws, "a", 0.5), column(c_b, ws, "b", 0.5)],
            &[tile("id:t1", c_a, "n", 1.0), tile("id:t2", c_b, "n", 1.0)],
        )
        .expect("layout folds");
        assert_eq!(layout.workspace, ws);
        assert_eq!(layout.columns.len(), 2);
        assert_eq!(layout.columns[0].entity, c_a);
        assert_eq!(layout.columns[1].entity, c_b);
        assert_eq!(layout.columns[0].tiles.len(), 1);
        assert_eq!(layout.columns[0].tiles[0].entity, "id:t1");
    }

    #[dialog_common::test]
    fn it_sorts_columns_by_order_key() {
        let ws = "id:ws";
        let layout = fold_layout(
            &[workspace(ws, None)],
            &[
                column("id:c1", ws, "c", 0.33),
                column("id:c2", ws, "a", 0.33),
                column("id:c3", ws, "b", 0.33),
            ],
            &[],
        )
        .expect("layout folds");
        let orders: Vec<&str> = layout.columns.iter().map(|c| c.order.as_str()).collect();
        assert_eq!(orders, vec!["a", "b", "c"]);
    }

    #[dialog_common::test]
    fn it_sorts_tiles_within_a_column_by_order_key() {
        let ws = "id:ws";
        let c = "id:col";
        let layout = fold_layout(
            &[workspace(ws, None)],
            &[column(c, ws, "n", 1.0)],
            &[
                tile("id:t1", c, "c", 0.33),
                tile("id:t2", c, "a", 0.33),
                tile("id:t3", c, "b", 0.33),
            ],
        )
        .expect("layout folds");
        let orders: Vec<&str> = layout.columns[0]
            .tiles
            .iter()
            .map(|t| t.order.as_str())
            .collect();
        assert_eq!(orders, vec!["a", "b", "c"]);
    }

    #[dialog_common::test]
    fn it_drops_tiles_whose_column_is_missing() {
        // A tile whose `column` reference doesn't exist in the
        // columns frame has no parent to live under. We drop it
        // rather than silently re-parent — keeps the tree clean
        // and the case is short-lived in practice (frames catch up).
        let ws = "id:ws";
        let c = "id:col";
        let layout = fold_layout(
            &[workspace(ws, None)],
            &[column(c, ws, "n", 1.0)],
            &[
                tile("id:t1", c, "a", 1.0),
                tile("id:orphan", "id:missing-col", "a", 1.0),
            ],
        )
        .expect("layout folds");
        assert_eq!(layout.columns[0].tiles.len(), 1);
        assert_eq!(layout.columns[0].tiles[0].entity, "id:t1");
    }

    #[dialog_common::test]
    fn it_carries_focus_from_the_workspace_frame() {
        let ws = "id:ws";
        let layout =
            fold_layout(&[workspace(ws, Some("id:focused-tile"))], &[], &[]).expect("layout folds");
        assert_eq!(layout.focus.as_deref(), Some("id:focused-tile"));
    }

    #[dialog_common::test]
    fn it_inserts_a_column_between_neighbours_at_the_right_slot() {
        // The reconciler relies on lex sort: an order key strictly
        // between two existing keys must land between them.
        let ws = "id:ws";
        let layout = fold_layout(
            &[workspace(ws, None)],
            &[
                column("id:cA", ws, "a", 0.33),
                column("id:cC", ws, "c", 0.33),
                column("id:cB", ws, "b", 0.33),
            ],
            &[],
        )
        .expect("layout folds");
        let entities: Vec<&str> = layout.columns.iter().map(|c| c.entity.as_str()).collect();
        assert_eq!(entities, vec!["id:cA", "id:cB", "id:cC"]);
    }

    #[dialog_common::test]
    fn it_preserves_unknown_kinds_so_the_reconciler_can_flag_them() {
        // A `kind` value the WM doesn't recognise still flows
        // through the fold; the reconciler is what surfaces an
        // error placeholder. Preserving the kind here means new
        // kinds can be added without touching model code.
        let ws = "id:ws";
        let c = "id:col";
        let mut t = tile("id:t1", c, "n", 1.0);
        t.fields.insert("kind".into(), json!("future-thing"));
        let layout = fold_layout(&[workspace(ws, None)], &[column(c, ws, "n", 1.0)], &[t])
            .expect("layout folds");
        assert_eq!(layout.columns[0].tiles[0].kind, "future-thing");
    }
}
