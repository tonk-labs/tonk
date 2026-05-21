//! Keyboard + pointer interaction.
//!
//! The pure-logic helpers — `next_focus_*`, `move_focused_*`,
//! `cycle_width` — derive the target of each user action from the
//! current [`Layout`] and the focused tile entity, without touching
//! the DOM or the network. Wasm-only handler glue (key dispatch,
//! click delegation, `/evaluate` POST) lands at the bottom of the
//! file, behind a `target_arch = "wasm32"` gate.

#![allow(dead_code)]

use crate::model::Layout;
use crate::order;

/// Which way to step when navigating or moving. `Backward` is left
/// (across columns) or up (within a column); `Forward` is right /
/// down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Backward,
    Forward,
}

/// Preset column widths the `R` key cycles through, matching niri's
/// default stops: ⅓, ½, ⅔, full.
const WIDTH_PRESETS: [f64; 4] = [1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0];

/// Given the focused tile, return the entity URI of the tile that
/// should become focus after stepping `dir` to the next/previous
/// column. Returns `None` at the strip boundary or when nothing is
/// focused. The new focus is the first tile of the target column —
/// niri snaps focus to the top of a column on horizontal moves.
pub fn next_focus_across_columns(layout: &Layout, dir: Direction) -> Option<String> {
    let (col_idx, _) = locate(layout, layout.focus.as_deref()?)?;
    let new_idx = match dir {
        Direction::Backward => col_idx.checked_sub(1)?,
        Direction::Forward => {
            let next = col_idx + 1;
            (next < layout.columns.len()).then_some(next)?
        }
    };
    layout.columns[new_idx]
        .tiles
        .first()
        .map(|t| t.entity.clone())
}

/// Within the focused tile's column, return the entity URI of the
/// tile that should become focus after stepping `dir`. Returns
/// `None` at the top/bottom of the column or when nothing is
/// focused.
pub fn next_focus_within_column(layout: &Layout, dir: Direction) -> Option<String> {
    let (col_idx, tile_idx) = locate(layout, layout.focus.as_deref()?)?;
    let col = &layout.columns[col_idx];
    let new_tile_idx = match dir {
        Direction::Backward => tile_idx.checked_sub(1)?,
        Direction::Forward => {
            let next = tile_idx + 1;
            (next < col.tiles.len()).then_some(next)?
        }
    };
    Some(col.tiles[new_tile_idx].entity.clone())
}

/// Compute the move payload for `Ctrl+arrow` on the focused column.
/// Returns `Some((column_entity, new_order))` — feed the order
/// straight into [`crate::writer::move_column_doc`]. Returns `None`
/// at the strip boundary or when no key fits in the target gap.
pub fn move_focused_column(layout: &Layout, dir: Direction) -> Option<(String, String)> {
    let (col_idx, _) = locate(layout, layout.focus.as_deref()?)?;
    let new_order = match dir {
        Direction::Backward => {
            if col_idx == 0 {
                return None;
            }
            // Target slot: before columns[col_idx - 1].
            let lo = (col_idx >= 2).then(|| layout.columns[col_idx - 2].order.as_str());
            let hi = Some(layout.columns[col_idx - 1].order.as_str());
            order::between(lo, hi)?
        }
        Direction::Forward => {
            if col_idx + 1 >= layout.columns.len() {
                return None;
            }
            // Target slot: after columns[col_idx + 1].
            let lo = Some(layout.columns[col_idx + 1].order.as_str());
            let hi = (col_idx + 2 < layout.columns.len())
                .then(|| layout.columns[col_idx + 2].order.as_str());
            order::between(lo, hi)?
        }
    };
    Some((layout.columns[col_idx].entity.clone(), new_order))
}

/// Compute the move payload for `Ctrl+arrow` on the focused tile
/// within its column. Returns `Some((tile_entity, new_order))` — feed
/// straight into [`crate::writer::move_tile_doc`] (passing the
/// tile's *current* column entity as `new_column`). Returns `None`
/// at the column boundary or when no order key fits.
pub fn move_focused_tile(layout: &Layout, dir: Direction) -> Option<(String, String)> {
    let focused = layout.focus.as_deref()?;
    let (col_idx, tile_idx) = locate(layout, focused)?;
    let col = &layout.columns[col_idx];
    let new_order = match dir {
        Direction::Backward => {
            if tile_idx == 0 {
                return None;
            }
            let lo = (tile_idx >= 2).then(|| col.tiles[tile_idx - 2].order.as_str());
            let hi = Some(col.tiles[tile_idx - 1].order.as_str());
            order::between(lo, hi)?
        }
        Direction::Forward => {
            if tile_idx + 1 >= col.tiles.len() {
                return None;
            }
            let lo = Some(col.tiles[tile_idx + 1].order.as_str());
            let hi =
                (tile_idx + 2 < col.tiles.len()).then(|| col.tiles[tile_idx + 2].order.as_str());
            order::between(lo, hi)?
        }
    };
    Some((focused.to_owned(), new_order))
}

/// Pick the next preset width strictly greater than `current`,
/// wrapping back to the smallest preset after passing the largest.
/// `R` cycles through these stops.
pub fn cycle_width(current: f64) -> f64 {
    // Small epsilon so a current value that floats around a preset
    // (say, 0.500000001) doesn't get stuck.
    const EPS: f64 = 1e-9;
    for preset in WIDTH_PRESETS {
        if preset > current + EPS {
            return preset;
        }
    }
    WIDTH_PRESETS[0]
}

/// Find a tile by entity URI in the layout's two-level tree.
/// Returns `(column_index, tile_index)` if found.
fn locate(layout: &Layout, tile_entity: &str) -> Option<(usize, usize)> {
    for (ci, col) in layout.columns.iter().enumerate() {
        for (ti, tile) in col.tiles.iter().enumerate() {
            if tile.entity == tile_entity {
                return Some((ci, ti));
            }
        }
    }
    None
}

/// Entity URI of the column that contains the focused tile, if any.
fn focused_column_entity(layout: &Layout) -> Option<String> {
    let focused = layout.focus.as_deref()?;
    let (col_idx, _) = locate(layout, focused)?;
    Some(layout.columns[col_idx].entity.clone())
}

/// Width of the column that contains the focused tile, if any.
fn focused_column_width(layout: &Layout) -> Option<f64> {
    let focused = layout.focus.as_deref()?;
    let (col_idx, _) = locate(layout, focused)?;
    Some(layout.columns[col_idx].width)
}

/// Dispatch a keyboard event to the matching mutation builder.
/// Returns the notation document to POST, or `None` if the key
/// doesn't bind to any action (or the action's target doesn't
/// exist, like arrow-down at the bottom of a column). Callers
/// `prevent_default()` only when this returns `Some`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn handle_keydown(
    layout: &Layout,
    ev: &web_sys::KeyboardEvent,
) -> Option<String> {
    use crate::writer;

    let key = ev.key();
    let ctrl = ev.ctrl_key() || ev.meta_key();

    match (key.as_str(), ctrl) {
        ("ArrowLeft", false) => {
            let tile = next_focus_across_columns(layout, Direction::Backward)?;
            Some(writer::set_focus_doc(&layout.workspace, &tile))
        }
        ("ArrowRight", false) => {
            let tile = next_focus_across_columns(layout, Direction::Forward)?;
            Some(writer::set_focus_doc(&layout.workspace, &tile))
        }
        ("ArrowUp", false) => {
            let tile = next_focus_within_column(layout, Direction::Backward)?;
            Some(writer::set_focus_doc(&layout.workspace, &tile))
        }
        ("ArrowDown", false) => {
            let tile = next_focus_within_column(layout, Direction::Forward)?;
            Some(writer::set_focus_doc(&layout.workspace, &tile))
        }
        ("ArrowLeft", true) => {
            let (entity, new_order) = move_focused_column(layout, Direction::Backward)?;
            Some(writer::move_column_doc(&entity, &new_order))
        }
        ("ArrowRight", true) => {
            let (entity, new_order) = move_focused_column(layout, Direction::Forward)?;
            Some(writer::move_column_doc(&entity, &new_order))
        }
        ("ArrowUp", true) => {
            let (tile_entity, new_order) = move_focused_tile(layout, Direction::Backward)?;
            let col_entity = focused_column_entity(layout)?;
            Some(writer::move_tile_doc(&tile_entity, &col_entity, &new_order))
        }
        ("ArrowDown", true) => {
            let (tile_entity, new_order) = move_focused_tile(layout, Direction::Forward)?;
            let col_entity = focused_column_entity(layout)?;
            Some(writer::move_tile_doc(&tile_entity, &col_entity, &new_order))
        }
        ("r" | "R", false) => {
            let col_entity = focused_column_entity(layout)?;
            let current = focused_column_width(layout)?;
            Some(writer::resize_column_doc(&col_entity, cycle_width(current)))
        }
        ("q" | "Q", false) => {
            let tile = layout.focus.clone()?;
            Some(writer::close_tile_doc(&tile))
        }
        _ => None,
    }
}

/// Dispatch a click to a focus-set mutation when the click landed
/// on (or inside) a `.niri-tile`. Returns the notation document to
/// POST, or `None` for clicks outside tile chrome.
#[cfg(target_arch = "wasm32")]
pub(crate) fn handle_click(
    layout: &Layout,
    ev: &web_sys::MouseEvent,
) -> Option<String> {
    use wasm_bindgen::JsCast;

    let target = ev.target()?;
    let el = target.dyn_into::<web_sys::Element>().ok()?;
    let tile_el = el.closest(".niri-tile").ok().flatten()?;
    let tile_entity = tile_el.get_attribute("data-entity")?;
    // No-op if the click was on the already-focused tile.
    if layout.focus.as_deref() == Some(tile_entity.as_str()) {
        return None;
    }
    Some(crate::writer::set_focus_doc(&layout.workspace, &tile_entity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column, Tile};

    /// Build a tile with the given entity / order, all other fields
    /// at sensible defaults. Tests don't need the kind / display
    /// fields to be realistic.
    fn tile(entity: &str, order: &str) -> Tile {
        Tile {
            entity: entity.to_owned(),
            order: order.to_owned(),
            height: 1.0,
            kind: "display".to_owned(),
            display_entity: None,
            display_view: None,
            display_model: None,
        }
    }

    fn column(entity: &str, order: &str, tiles: Vec<Tile>) -> Column {
        Column {
            entity: entity.to_owned(),
            order: order.to_owned(),
            width: 0.5,
            tiles,
        }
    }

    fn layout(focus: Option<&str>, columns: Vec<Column>) -> Layout {
        Layout {
            workspace: "id:ws".into(),
            focus: focus.map(str::to_owned),
            columns,
        }
    }

    #[dialog_common::test]
    fn it_steps_focus_to_the_first_tile_of_the_next_column() {
        let l = layout(
            Some("t-a1"),
            vec![
                column("c-a", "a", vec![tile("t-a1", "n")]),
                column("c-b", "b", vec![tile("t-b1", "n"), tile("t-b2", "p")]),
            ],
        );
        assert_eq!(
            next_focus_across_columns(&l, Direction::Forward).as_deref(),
            Some("t-b1"),
        );
    }

    #[dialog_common::test]
    fn it_returns_none_at_the_strip_boundary_when_stepping_across() {
        let l = layout(
            Some("t-a1"),
            vec![column("c-a", "a", vec![tile("t-a1", "n")])],
        );
        assert!(next_focus_across_columns(&l, Direction::Backward).is_none());
        assert!(next_focus_across_columns(&l, Direction::Forward).is_none());
    }

    #[dialog_common::test]
    fn it_steps_focus_within_the_column() {
        let l = layout(
            Some("t-b1"),
            vec![column(
                "c-b",
                "n",
                vec![tile("t-b1", "n"), tile("t-b2", "p"), tile("t-b3", "r")],
            )],
        );
        assert_eq!(
            next_focus_within_column(&l, Direction::Forward).as_deref(),
            Some("t-b2"),
        );
        let mid = layout(Some("t-b2"), l.columns.clone());
        assert_eq!(
            next_focus_within_column(&mid, Direction::Backward).as_deref(),
            Some("t-b1"),
        );
    }

    #[dialog_common::test]
    fn it_returns_none_at_the_column_boundary_when_stepping_within() {
        let l = layout(
            Some("t-b1"),
            vec![column(
                "c-b",
                "n",
                vec![tile("t-b1", "n"), tile("t-b2", "p")],
            )],
        );
        assert!(next_focus_within_column(&l, Direction::Backward).is_none());
        let bot = layout(Some("t-b2"), l.columns.clone());
        assert!(next_focus_within_column(&bot, Direction::Forward).is_none());
    }

    #[dialog_common::test]
    fn it_computes_a_new_order_for_moving_a_column_left() {
        // Three columns at orders n / p / r. Focus is on the
        // middle column; moving it left should produce an order
        // strictly less than "n" and strictly greater than nothing
        // (no column to its left of left-target).
        let l = layout(
            Some("t-mid"),
            vec![
                column("c-l", "n", vec![tile("t-l", "n")]),
                column("c-m", "p", vec![tile("t-mid", "n")]),
                column("c-r", "r", vec![tile("t-r", "n")]),
            ],
        );
        let (entity, new_order) =
            move_focused_column(&l, Direction::Backward).expect("move expected");
        assert_eq!(entity, "c-m");
        assert!(
            new_order.as_str() < "n",
            "expected new order < 'n', got {new_order:?}",
        );
    }

    #[dialog_common::test]
    fn it_computes_a_new_order_for_moving_a_column_right() {
        let l = layout(
            Some("t-mid"),
            vec![
                column("c-l", "n", vec![tile("t-l", "n")]),
                column("c-m", "p", vec![tile("t-mid", "n")]),
                column("c-r", "r", vec![tile("t-r", "n")]),
            ],
        );
        let (entity, new_order) =
            move_focused_column(&l, Direction::Forward).expect("move expected");
        assert_eq!(entity, "c-m");
        assert!(
            new_order.as_str() > "r",
            "expected new order > 'r', got {new_order:?}",
        );
    }

    #[dialog_common::test]
    fn it_refuses_to_move_a_column_off_the_strip() {
        let l = layout(
            Some("t-a1"),
            vec![column("c-a", "n", vec![tile("t-a1", "n")])],
        );
        assert!(move_focused_column(&l, Direction::Backward).is_none());
        assert!(move_focused_column(&l, Direction::Forward).is_none());
    }

    #[dialog_common::test]
    fn it_computes_a_new_order_for_moving_a_tile_down() {
        let l = layout(
            Some("t-mid"),
            vec![column(
                "c",
                "n",
                vec![tile("t-top", "n"), tile("t-mid", "p"), tile("t-bot", "r")],
            )],
        );
        let (entity, new_order) =
            move_focused_tile(&l, Direction::Forward).expect("move expected");
        assert_eq!(entity, "t-mid");
        assert!(
            new_order.as_str() > "r",
            "expected new order > 'r', got {new_order:?}",
        );
    }

    #[dialog_common::test]
    fn it_refuses_to_move_a_tile_off_the_column() {
        let l = layout(
            Some("t-only"),
            vec![column("c", "n", vec![tile("t-only", "n")])],
        );
        assert!(move_focused_tile(&l, Direction::Backward).is_none());
        assert!(move_focused_tile(&l, Direction::Forward).is_none());
    }

    #[dialog_common::test]
    fn it_cycles_through_preset_column_widths() {
        // From 0.0 (or any value < 1/3): jump to 1/3.
        assert!((cycle_width(0.0) - 1.0 / 3.0).abs() < 1e-9);
        // From exactly 1/3: jump to 1/2.
        assert!((cycle_width(1.0 / 3.0) - 0.5).abs() < 1e-9);
        // From 1/2: jump to 2/3.
        assert!((cycle_width(0.5) - 2.0 / 3.0).abs() < 1e-9);
        // From 2/3: jump to 1.
        assert!((cycle_width(2.0 / 3.0) - 1.0).abs() < 1e-9);
        // From 1: wrap to 1/3.
        assert!((cycle_width(1.0) - 1.0 / 3.0).abs() < 1e-9);
    }

    #[dialog_common::test]
    fn it_returns_none_for_focus_navigation_when_nothing_is_focused() {
        let l = layout(None, vec![column("c", "n", vec![tile("t", "n")])]);
        assert!(next_focus_across_columns(&l, Direction::Forward).is_none());
        assert!(next_focus_within_column(&l, Direction::Forward).is_none());
    }
}
