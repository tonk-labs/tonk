// Consumed only by the wasm-gated `element` and this module's own
// native tests; a native non-test lib build reaches none of it.
#![allow(dead_code)]

//! Universal workspace fold.
//!
//! [`Layout`] is "the latest workspace + focus + tiles frames folded
//! together and sorted." Subscriptions deliver flat lists of
//! [`Conclusion`] rows; effect handlers want a tile linear order
//! with focus resolved so they can read "previous", "next", or
//! "tile at index N" without re-walking the frame.
//!
//! Focus rides its own frame because cardinality-one fields are
//! filter requirements: a fresh workspace has no focus claim, so
//! including focus in the workspace query's predicate would return
//! zero rows. The separate focus subscription pins `this =
//! workspace_entity` and returns zero or one row.

use tonk_schema::conclusion::Conclusion;

/// The folded workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Workspace entity URI.
    pub workspace: String,
    /// Focused tile entity URI, if any.
    pub focus: Option<String>,
    /// Tiles in lex `order` order.
    pub tiles: Vec<Tile>,
}

/// One tile in the workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Tile entity URI (`this:`).
    pub entity: String,
    /// Lex ordering key within the workspace.
    pub order: String,
    /// What the tile renders (`tile.entity`). `None` for concept-list
    /// tiles that key off `view` + `model` alone.
    pub target: Option<String>,
    /// View name (`tile.view`).
    pub view: String,
    /// Model / concept name (`tile.model`).
    pub model: String,
}

/// Fold the workspace + focus + tiles frames into a sorted [`Layout`].
///
/// Returns `None` when no workspace row matches — callers treat this
/// as "no workspace yet" and either render empty or lazy-bootstrap on
/// the first `open-tile`. Tiles whose `workspace` reference doesn't
/// match the folded workspace are dropped (the tiles subscription
/// streams every tile on the branch).
pub fn fold_universal(
    workspace_frame: &[Conclusion],
    focus_frame: &[Conclusion],
    tiles_frame: &[Conclusion],
) -> Option<Layout> {
    let ws_row = workspace_frame.first()?;
    let workspace = ws_row.this.clone();
    let focus = focus_frame
        .first()
        .and_then(|row| read_str(row, "focus"))
        .map(str::to_owned);

    let mut tiles: Vec<Tile> = tiles_frame
        .iter()
        .filter_map(|row| build_tile(row, &workspace))
        .collect();
    tiles.sort_by(|a, b| a.order.cmp(&b.order));

    Some(Layout {
        workspace,
        focus,
        tiles,
    })
}

/// Project a conclusion row into a [`Tile`], dropping it if it's not
/// part of `workspace` or is missing a required field.
fn build_tile(row: &Conclusion, workspace: &str) -> Option<Tile> {
    if read_str(row, "workspace")? != workspace {
        return None;
    }
    let order = read_str(row, "order")?.to_owned();
    let view = read_str(row, "view")?.to_owned();
    let model = read_str(row, "model")?.to_owned();
    let target = read_str(row, "entity").map(str::to_owned);
    Some(Tile {
        entity: row.this.clone(),
        order,
        target,
        view,
        model,
    })
}

fn read_str<'a>(c: &'a Conclusion, name: &str) -> Option<&'a str> {
    c.fields.get(name).and_then(|v| v.as_str())
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

    fn workspace(this: &str) -> Conclusion {
        conclusion(this, serde_json::Value::Object(serde_json::Map::new()))
    }

    fn focus_row(workspace_uri: &str, focus_tile: &str) -> Conclusion {
        conclusion(workspace_uri, json!({ "focus": focus_tile }))
    }

    fn tile(this: &str, workspace_uri: &str, order: &str) -> Conclusion {
        conclusion(
            this,
            json!({
                "workspace": workspace_uri,
                "order": order,
                "view": "card",
                "model": "person",
                "entity": "id:target",
            }),
        )
    }

    #[dialog_common::test]
    fn it_returns_none_when_the_workspace_frame_is_empty() {
        assert!(fold_universal(&[], &[], &[]).is_none());
    }

    #[dialog_common::test]
    fn it_carries_focus_when_the_focus_frame_has_a_row() {
        let layout = fold_universal(&[workspace("id:ws")], &[focus_row("id:ws", "id:t")], &[])
            .expect("layout folds");
        assert_eq!(layout.focus.as_deref(), Some("id:t"));
    }

    #[dialog_common::test]
    fn it_leaves_focus_none_when_the_focus_frame_is_empty() {
        let layout = fold_universal(&[workspace("id:ws")], &[], &[]).expect("layout folds");
        assert_eq!(layout.focus, None);
    }

    #[dialog_common::test]
    fn it_sorts_tiles_by_their_order_key() {
        let ws = "id:ws";
        let layout = fold_universal(
            &[workspace(ws)],
            &[],
            &[
                tile("id:t1", ws, "c"),
                tile("id:t2", ws, "a"),
                tile("id:t3", ws, "b"),
            ],
        )
        .expect("layout folds");
        let orders: Vec<&str> = layout.tiles.iter().map(|t| t.order.as_str()).collect();
        assert_eq!(orders, vec!["a", "b", "c"]);
    }

    #[dialog_common::test]
    fn it_drops_tiles_belonging_to_a_different_workspace() {
        let layout = fold_universal(
            &[workspace("id:ws-a")],
            &[],
            &[
                tile("id:t-mine", "id:ws-a", "n"),
                tile("id:t-theirs", "id:ws-b", "n"),
            ],
        )
        .expect("layout folds");
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!(layout.tiles[0].entity, "id:t-mine");
    }

    #[dialog_common::test]
    fn it_drops_tiles_missing_a_required_field() {
        let ws = "id:ws";
        let mut without_order = tile("id:no-order", ws, "n");
        without_order.fields.remove("order");
        let mut without_view = tile("id:no-view", ws, "n");
        without_view.fields.remove("view");
        let mut without_model = tile("id:no-model", ws, "n");
        without_model.fields.remove("model");
        let layout = fold_universal(
            &[workspace(ws)],
            &[],
            &[
                without_order,
                without_view,
                without_model,
                tile("id:ok", ws, "n"),
            ],
        )
        .expect("layout folds");
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!(layout.tiles[0].entity, "id:ok");
    }

    #[dialog_common::test]
    fn it_treats_tile_entity_as_optional() {
        let ws = "id:ws";
        let mut concept_tile = tile("id:concept", ws, "n");
        concept_tile.fields.remove("entity");
        let layout = fold_universal(&[workspace(ws)], &[], &[concept_tile]).expect("layout folds");
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!(layout.tiles[0].target, None);
    }

    #[dialog_common::test]
    fn it_inserts_a_tile_between_neighbours_at_the_right_slot() {
        let ws = "id:ws";
        let layout = fold_universal(
            &[workspace(ws)],
            &[],
            &[
                tile("id:tA", ws, "a"),
                tile("id:tC", ws, "c"),
                tile("id:tB", ws, "b"),
            ],
        )
        .expect("layout folds");
        let entities: Vec<&str> = layout.tiles.iter().map(|t| t.entity.as_str()).collect();
        assert_eq!(entities, vec!["id:tA", "id:tB", "id:tC"]);
    }
}
