//! The in-memory layout tree and the fold that builds it from
//! subscription frames.
//!
//! Three subscriptions feed this module — a `workspace` row, the
//! `column` rows belonging to it, and the `tile` rows belonging
//! to those columns. [`Layout::fold`] collapses the latest frame
//! of each into a single sorted tree: columns ordered by their
//! float `order`, tiles ordered by theirs.
//!
//! Everything here is target-independent so it runs under
//! `cargo test` without a DOM.

use tonk_schema::conclusion::Conclusion;

/// Field name carrying the focused-tile pointer on a `workspace`
/// row.
const WORKSPACE_FOCUS: &str = "focus";
/// Field names on a `column` row. The `workspace` parent
/// reference is a query-level filter (see `resolve`), so the fold
/// itself only needs the order and width.
const COLUMN_ORDER: &str = "order";
const COLUMN_WIDTH: &str = "width";
/// Field names on a `tile` row.
const TILE_COLUMN: &str = "column";
const TILE_ORDER: &str = "order";
const TILE_HEIGHT: &str = "height";
const TILE_ENTITY: &str = "entity";
const TILE_VIEW: &str = "view";
const TILE_MODEL: &str = "model";

/// Fallback column width, in major grid units, when a `column`
/// row omits or malforms its `width` field.
const DEFAULT_COLUMN_WIDTH: u32 = 8;
/// Fallback tile height, in major grid units, when a `tile` row
/// omits or malforms its `height` field.
const DEFAULT_TILE_HEIGHT: u32 = 8;

/// One cell in the strip. Mounts a `<tonk-display>` pointed at
/// [`Tile::entity`].
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Entity URI of the `tile` row itself — the layout-side
    /// identity, used as the DOM reconciliation key.
    pub id: String,
    /// Vertical position within the column. Sorted ascending.
    pub order: f64,
    /// Height in major grid units (1 unit = 64px). Tiles in a
    /// column divide its height by these counts.
    pub height: u32,
    /// Entity the tile's `<tonk-display>` renders, if any. A tile
    /// with no entity is a valid empty cell.
    pub entity: Option<String>,
    /// `view` attribute forwarded to the `<tonk-display>`.
    pub view: Option<String>,
    /// `model` attribute forwarded to the `<tonk-display>`.
    pub model: Option<String>,
}

/// A vertical stack of tiles, plus its width in the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Entity URI of the `column` row — the DOM reconciliation
    /// key.
    pub id: String,
    /// Horizontal position in the strip. Sorted ascending.
    pub order: f64,
    /// Width in major grid units (1 unit = 64px).
    pub width: u32,
    /// Tiles in this column, sorted by [`Tile::order`].
    pub tiles: Vec<Tile>,
}

/// The whole strip for one workspace.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Entity URI of the focused tile, if the workspace names one.
    pub focus: Option<String>,
    /// Columns in strip order.
    pub columns: Vec<Column>,
}

impl Layout {
    /// Fold the latest frame of each subscription into a sorted
    /// tree.
    ///
    /// - `workspace` is the single `workspace` row (or `None`
    ///   before its first frame); its `focus` field seeds
    ///   [`Layout::focus`].
    /// - `columns` is every `column` row for this workspace.
    /// - `tiles` is every `tile` row across those columns; each
    ///   is attached to its parent by the `column` field. A tile
    ///   whose parent column is absent is dropped.
    pub fn fold(
        workspace: Option<&Conclusion>,
        columns: &[Conclusion],
        tiles: &[Conclusion],
    ) -> Self {
        let focus = workspace.and_then(|row| string_field(row, WORKSPACE_FOCUS));

        let mut columns: Vec<Column> = columns
            .iter()
            .map(|row| Column {
                id: row.this.clone(),
                order: number_field(row, COLUMN_ORDER).unwrap_or(0.0),
                width: units_field(row, COLUMN_WIDTH).unwrap_or(DEFAULT_COLUMN_WIDTH),
                tiles: Vec::new(),
            })
            .collect();

        for row in tiles {
            let Some(parent) = string_field(row, TILE_COLUMN) else {
                continue;
            };
            let Some(column) = columns.iter_mut().find(|c| c.id == parent) else {
                // Orphan tile — its column hasn't arrived (or was
                // removed). Drop it; a later frame may include
                // the column and the tile both.
                continue;
            };
            column.tiles.push(Tile {
                id: row.this.clone(),
                order: number_field(row, TILE_ORDER).unwrap_or(0.0),
                height: units_field(row, TILE_HEIGHT).unwrap_or(DEFAULT_TILE_HEIGHT),
                entity: string_field(row, TILE_ENTITY),
                view: string_field(row, TILE_VIEW),
                model: string_field(row, TILE_MODEL),
            });
        }

        // Sort columns by strip position, tiles by vertical
        // position. `total_cmp` keeps the sort total even if a
        // malformed frame slips a NaN through.
        columns.sort_by(|a, b| a.order.total_cmp(&b.order));
        for column in &mut columns {
            column.tiles.sort_by(|a, b| a.order.total_cmp(&b.order));
        }

        Self { focus, columns }
    }

    /// True when the workspace resolved but holds no columns —
    /// the element reflects `data-state="empty"` for this.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

/// Read a string-valued field, treating absent and non-string
/// values alike as `None`.
fn string_field(row: &Conclusion, name: &str) -> Option<String> {
    row.fields
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// Read a `float`-valued field as `f64` — used for the sortable
/// `order` keys.
fn number_field(row: &Conclusion, name: &str) -> Option<f64> {
    row.fields.get(name).and_then(serde_json::Value::as_f64)
}

/// Read an `unsigned-integer`-valued field as a `u32` grid-unit
/// count. A negative or absurdly large value is treated as absent
/// so the caller's default applies.
fn units_field(row: &Conclusion, name: &str) -> Option<u32> {
    row.fields
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// Build a `Conclusion` for `entity` with the given fields.
    fn row(entity: &str, fields: &[(&str, serde_json::Value)]) -> Conclusion {
        let mut map = BTreeMap::new();
        for (name, value) in fields {
            map.insert((*name).to_string(), value.clone());
        }
        Conclusion {
            this: entity.to_string(),
            fields: map,
        }
    }

    #[test]
    fn it_folds_an_empty_workspace_to_an_empty_layout() {
        let layout = Layout::fold(None, &[], &[]);
        assert!(layout.is_empty());
        assert_eq!(layout.focus, None);
    }

    #[test]
    fn it_reads_the_focused_tile_from_the_workspace_row() {
        let workspace = row("ws:1", &[("focus", json!("tile:7"))]);
        let layout = Layout::fold(Some(&workspace), &[], &[]);
        assert_eq!(layout.focus.as_deref(), Some("tile:7"));
    }

    #[test]
    fn it_sorts_columns_by_their_float_order() {
        let columns = [
            row("col:b", &[("order", json!(2.0)), ("width", json!(8))]),
            row("col:a", &[("order", json!(1.0)), ("width", json!(8))]),
            row("col:c", &[("order", json!(3.0)), ("width", json!(8))]),
        ];
        let layout = Layout::fold(None, &columns, &[]);
        let ids: Vec<&str> = layout.columns.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["col:a", "col:b", "col:c"]);
    }

    #[test]
    fn it_places_a_fractional_order_column_between_its_neighbours() {
        // The fractional-indexing property: inserting at order
        // 1.5 lands the column between 1.0 and 2.0 with no
        // renumbering.
        let columns = [
            row("col:a", &[("order", json!(1.0))]),
            row("col:c", &[("order", json!(2.0))]),
            row("col:b", &[("order", json!(1.5))]),
        ];
        let layout = Layout::fold(None, &columns, &[]);
        let ids: Vec<&str> = layout.columns.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["col:a", "col:b", "col:c"]);
    }

    #[test]
    fn it_attaches_tiles_to_their_parent_column_sorted_by_order() {
        let columns = [row("col:1", &[("order", json!(1.0))])];
        let tiles = [
            row(
                "tile:y",
                &[("column", json!("col:1")), ("order", json!(2.0))],
            ),
            row(
                "tile:x",
                &[("column", json!("col:1")), ("order", json!(1.0))],
            ),
        ];
        let layout = Layout::fold(None, &columns, &tiles);
        let tile_ids: Vec<&str> = layout.columns[0]
            .tiles
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(tile_ids, ["tile:x", "tile:y"]);
    }

    #[test]
    fn it_drops_a_tile_whose_parent_column_is_absent() {
        let tiles = [row("tile:orphan", &[("column", json!("col:missing"))])];
        let layout = Layout::fold(None, &[], &tiles);
        assert!(layout.is_empty());
    }

    #[test]
    fn it_forwards_the_tile_content_descriptor() {
        let columns = [row("col:1", &[("order", json!(1.0))])];
        let tiles = [row(
            "tile:1",
            &[
                ("column", json!("col:1")),
                ("entity", json!("did:key:zEntity")),
                ("view", json!("card")),
                ("model", json!("note")),
            ],
        )];
        let layout = Layout::fold(None, &columns, &tiles);
        let tile = &layout.columns[0].tiles[0];
        assert_eq!(tile.entity.as_deref(), Some("did:key:zEntity"));
        assert_eq!(tile.view.as_deref(), Some("card"));
        assert_eq!(tile.model.as_deref(), Some("note"));
    }

    #[test]
    fn it_falls_back_to_default_sizes_when_fields_are_missing() {
        let columns = [row("col:1", &[("order", json!(1.0))])];
        let tiles = [row("tile:1", &[("column", json!("col:1"))])];
        let layout = Layout::fold(None, &columns, &tiles);
        assert_eq!(layout.columns[0].width, DEFAULT_COLUMN_WIDTH);
        assert_eq!(layout.columns[0].tiles[0].height, DEFAULT_TILE_HEIGHT);
    }

    #[test]
    fn it_keeps_an_empty_tile_with_no_entity() {
        let columns = [row("col:1", &[("order", json!(1.0))])];
        let tiles = [row("tile:1", &[("column", json!("col:1"))])];
        let layout = Layout::fold(None, &columns, &tiles);
        assert_eq!(layout.columns[0].tiles.len(), 1);
        assert_eq!(layout.columns[0].tiles[0].entity, None);
    }
}
