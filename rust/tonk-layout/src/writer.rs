//! Notation-document builders for the six effects, plus the
//! wasm-only `/evaluate` POST.
//!
//! Every effect that touches branch state lands here. Builders are
//! pure string-format functions, native-testable. Resolvers are pure
//! readers of a folded [`Layout`] that derive concrete write args
//! (target tile, lex-midpoint order key, focus-advance choice) from
//! the effect's high-level params. The wasm-only `post_evaluate`
//! shipped at the bottom is the single transport point.
//!
//! All builders assume target entities already exist on the branch
//! unless they're freshly minted in the same document. Partial-field
//! updates are safe under dialog-yaml semantics because the analyzer
//! skips the "incomplete fresh-entity" check when `this:` resolves
//! to a known entity.

use crate::model::Layout;
use crate::order;

/// Step direction for `focus-prev` / `focus-next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Toward smaller `order` keys.
    Prev,
    /// Toward larger `order` keys.
    Next,
}

/// What `close-tile` should do to `workspace.focus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseFocus {
    /// Target wasn't focused — leave focus alone.
    Leave,
    /// Target was focused; advance to this neighbour.
    SetTo(String),
    /// Target was focused; no neighbours — retract focus.
    Clear,
}

/// `open-tile`'s parent workspace: either an existing entity URI, or
/// a fresh-minted workspace to lazy-bootstrap in the same document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTarget {
    /// Workspace already exists on the branch.
    Existing(String),
    /// Workspace does not exist yet; mint it under `id` with `name`.
    Bootstrap {
        /// Minted ULID URI (`id:<ulid>`) for the new workspace.
        id: String,
        /// Name claim (matches the host element's `workspace` attribute).
        name: String,
    },
}

/// Walk linear order from current focus to its neighbour. Returns
/// `None` at the boundary, or when nothing is focused (use
/// `focus-tile` to set an initial focus).
pub fn resolve_focus_step(layout: &Layout, direction: Direction) -> Option<String> {
    let focus = layout.focus.as_deref()?;
    let idx = layout.tiles.iter().position(|t| t.entity == focus)?;
    let new_idx = match direction {
        Direction::Prev => idx.checked_sub(1)?,
        Direction::Next => {
            let next = idx + 1;
            (next < layout.tiles.len()).then_some(next)?
        }
    };
    Some(layout.tiles[new_idx].entity.clone())
}

/// Compute the `order` key for an `open-tile` or `reorder-tile`
/// per the SPEC's order-key rules. `before` / `after` are tile
/// entity URIs; either, both, or neither may be set.
///
/// `Err` describes which input didn't resolve. `Ok` is the lex-key
/// to assert.
pub fn resolve_position_order(
    layout: &Layout,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<String, &'static str> {
    let find = |uri: &str| layout.tiles.iter().position(|t| t.entity == uri);
    let (lo, hi): (Option<&str>, Option<&str>) = match (before, after) {
        (Some(b), Some(a)) => {
            let b_idx = find(b).ok_or("before tile not found")?;
            let a_idx = find(a).ok_or("after tile not found")?;
            (
                Some(layout.tiles[a_idx].order.as_str()),
                Some(layout.tiles[b_idx].order.as_str()),
            )
        }
        (Some(b), None) => {
            let b_idx = find(b).ok_or("before tile not found")?;
            let prev_order = b_idx.checked_sub(1).map(|i| layout.tiles[i].order.as_str());
            (prev_order, Some(layout.tiles[b_idx].order.as_str()))
        }
        (None, Some(a)) => {
            let a_idx = find(a).ok_or("after tile not found")?;
            let next_order = layout.tiles.get(a_idx + 1).map(|t| t.order.as_str());
            (Some(layout.tiles[a_idx].order.as_str()), next_order)
        }
        (None, None) => {
            let last_order = layout.tiles.last().map(|t| t.order.as_str());
            (last_order, None)
        }
    };
    order::between(lo, hi).ok_or("no order key fits")
}

/// Decide what to do with `workspace.focus` on `close-tile(target)`.
/// If the target wasn't focused, leave focus untouched. Otherwise
/// advance to the previous tile (or next if previous is gone, or
/// clear if no tiles remain).
pub fn resolve_close_focus(layout: &Layout, target: &str) -> CloseFocus {
    if layout.focus.as_deref() != Some(target) {
        return CloseFocus::Leave;
    }
    let Some(idx) = layout.tiles.iter().position(|t| t.entity == target) else {
        // Target was focused but isn't in the tile list — clear
        // focus so the workspace doesn't keep pointing at a ghost.
        return CloseFocus::Clear;
    };
    if let Some(prev) = idx.checked_sub(1).and_then(|i| layout.tiles.get(i)) {
        return CloseFocus::SetTo(prev.entity.clone());
    }
    if let Some(next) = layout.tiles.get(idx + 1) {
        return CloseFocus::SetTo(next.entity.clone());
    }
    CloseFocus::Clear
}

/// Build the notation document for `focus-tile` / `focus-prev` /
/// `focus-next`: assert `workspace.focus = target`. Re-asserting a
/// cardinality-one field retracts the previous value automatically.
pub fn focus_tile_doc(workspace_entity: &str, target_tile: &str) -> String {
    format!("workspace!:\n  this: {workspace_entity}\n  focus: {target_tile}\n")
}

/// Build the notation document for `close-tile`: retract every claim
/// on the target tile and, if it was focused, advance focus in the
/// same atomic document.
pub fn close_tile_doc(
    target_tile: &str,
    workspace_entity: &str,
    focus_action: CloseFocus,
) -> String {
    let mut doc = format!("tile!:\n  this: {target_tile}\n  ..: _\n");
    match focus_action {
        CloseFocus::Leave => {}
        CloseFocus::SetTo(new_focus) => {
            doc.push('\n');
            doc.push_str(&focus_tile_doc(workspace_entity, &new_focus));
        }
        CloseFocus::Clear => {
            doc.push('\n');
            doc.push_str(&format!(
                "workspace!:\n  this: {workspace_entity}\n  focus: _\n"
            ));
        }
    }
    doc
}

/// Build the notation document for `reorder-tile`: assert `order` on
/// `target_tile`. The caller resolves the new order via
/// [`resolve_position_order`].
pub fn reorder_tile_doc(target_tile: &str, new_order: &str) -> String {
    format!(
        "tile!:\n  this: {target_tile}\n  order: {}\n",
        quoted(new_order),
    )
}

/// Build the notation document for `update-tile-content`: assert
/// whichever of `entity` / `view` / `model` are provided. Emits an
/// empty doc when all three are `None`.
pub fn update_tile_content_doc(
    target_tile: &str,
    entity: Option<&str>,
    view: Option<&str>,
    model: Option<&str>,
) -> String {
    if entity.is_none() && view.is_none() && model.is_none() {
        return String::new();
    }
    let mut doc = format!("tile!:\n  this: {target_tile}\n");
    if let Some(e) = entity {
        doc.push_str(&format!("  entity: {e}\n"));
    }
    if let Some(v) = view {
        doc.push_str(&format!("  view: {}\n", quoted(v)));
    }
    if let Some(m) = model {
        doc.push_str(&format!("  model: {}\n", quoted(m)));
    }
    doc
}

/// Build the notation document for `open-tile`: assert a fresh `tile!`
/// row and set `workspace.focus` to it, atomically. When
/// `workspace = Bootstrap`, also mints the workspace in the same
/// document — the full lazy-bootstrap case.
pub fn open_tile_doc(
    workspace: WorkspaceTarget,
    new_tile_id: &str,
    order: &str,
    view: &str,
    model: &str,
    entity: Option<&str>,
) -> String {
    let mut doc = String::new();
    let workspace_ref = match &workspace {
        WorkspaceTarget::Existing(uri) => uri.as_str(),
        WorkspaceTarget::Bootstrap { id, name } => {
            doc.push_str(&format!(
                "workspace!:\n  this: {id}\n  name: {}\n\n",
                quoted(name)
            ));
            id.as_str()
        }
    };
    doc.push_str(&format!(
        "tile!:\n  this: {new_tile_id}\n  workspace: {workspace_ref}\n  order: {}\n  view: {}\n  model: {}\n",
        quoted(order),
        quoted(view),
        quoted(model),
    ));
    if let Some(e) = entity {
        doc.push_str(&format!("  entity: {e}\n"));
    }
    doc.push('\n');
    doc.push_str(&focus_tile_doc(workspace_ref, new_tile_id));
    doc
}

/// Double-quote a string for YAML output. Embedded backslashes /
/// quotes are escaped — Rust's debug format does the right thing.
fn quoted(value: &str) -> String {
    format!("{value:?}")
}

/// POST a notation document to the `/evaluate` endpoint. Returns
/// `Ok(())` on a 2xx response; surfaces network or HTTP errors as
/// [`ErrorDetail`] so callers can route them through the same fail
/// path subscriptions use.
#[cfg(target_arch = "wasm32")]
pub async fn post_evaluate(url: &str, doc: &str) -> Result<(), tonk_concept::error::ErrorDetail> {
    use tonk_concept::error::{ErrorDetail, ErrorKind};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Headers, Request, RequestInit, Response, window};

    let init = RequestInit::new();
    init.set_method("POST");
    let headers = Headers::new()
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Headers: {e:?}")))?;
    headers
        .append("content-type", "application/yaml")
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("content-type: {e:?}")))?;
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(doc));

    let request = Request::new_with_str_and_init(url, &init)
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("Request: {e:?}")))?;
    let win = window().ok_or_else(|| ErrorDetail::new(ErrorKind::Network, "no window"))?;
    let resp_value = JsFuture::from(win.fetch_with_request(&request))
        .await
        .map_err(|e| ErrorDetail::new(ErrorKind::Network, format!("fetch: {e:?}")))?;
    let resp: Response = resp_value
        .dyn_into()
        .map_err(|_| ErrorDetail::new(ErrorKind::Network, "fetch did not return Response"))?;
    if !resp.ok() {
        return Err(ErrorDetail::new(
            ErrorKind::Network,
            format!("evaluate HTTP {}", resp.status()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tile;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const WORKSPACE: &str = "id:01HMW000000000000000000000";
    const TILE_NEW: &str = "id:01HMT999999999999999999999";
    const TARGET: &str = "id:01HENT000000000000000000000";

    fn tile(entity: &str, order: &str) -> Tile {
        Tile {
            entity: entity.to_owned(),
            order: order.to_owned(),
            target: None,
            view: "card".to_owned(),
            model: "person".to_owned(),
        }
    }

    fn layout(focus: Option<&str>, tiles: Vec<Tile>) -> Layout {
        Layout {
            workspace: WORKSPACE.to_owned(),
            focus: focus.map(str::to_owned),
            tiles,
        }
    }

    // -- resolve_focus_step ---------------------------------------

    #[dialog_common::test]
    fn it_steps_focus_to_the_next_tile_in_linear_order() {
        let l = layout(
            Some("id:a"),
            vec![tile("id:a", "a"), tile("id:b", "b"), tile("id:c", "c")],
        );
        assert_eq!(
            resolve_focus_step(&l, Direction::Next).as_deref(),
            Some("id:b"),
        );
    }

    #[dialog_common::test]
    fn it_steps_focus_to_the_previous_tile_in_linear_order() {
        let l = layout(
            Some("id:c"),
            vec![tile("id:a", "a"), tile("id:b", "b"), tile("id:c", "c")],
        );
        assert_eq!(
            resolve_focus_step(&l, Direction::Prev).as_deref(),
            Some("id:b"),
        );
    }

    #[dialog_common::test]
    fn it_returns_none_at_the_linear_order_boundary() {
        let l = layout(Some("id:a"), vec![tile("id:a", "a"), tile("id:b", "b")]);
        assert!(resolve_focus_step(&l, Direction::Prev).is_none());
        let r = layout(Some("id:b"), vec![tile("id:a", "a"), tile("id:b", "b")]);
        assert!(resolve_focus_step(&r, Direction::Next).is_none());
    }

    #[dialog_common::test]
    fn it_returns_none_when_no_tile_is_focused() {
        let l = layout(None, vec![tile("id:a", "a"), tile("id:b", "b")]);
        assert!(resolve_focus_step(&l, Direction::Next).is_none());
    }

    // -- resolve_position_order -----------------------------------

    #[dialog_common::test]
    fn it_picks_a_midpoint_between_two_supplied_neighbours() {
        // before set + after set: midpoint(after.order, before.order).
        let l = layout(None, vec![tile("id:a", "a"), tile("id:c", "c")]);
        let mid = resolve_position_order(&l, Some("id:c"), Some("id:a")).expect("key fits");
        assert!(mid.as_str() > "a" && mid.as_str() < "c");
    }

    #[dialog_common::test]
    fn it_places_a_new_tile_before_an_existing_tile() {
        // before set + after unset: midpoint(prev(before).order, before.order).
        let l = layout(
            None,
            vec![tile("id:a", "a"), tile("id:c", "c"), tile("id:e", "e")],
        );
        let mid = resolve_position_order(&l, Some("id:c"), None).expect("key fits");
        assert!(mid.as_str() > "a" && mid.as_str() < "c");
    }

    #[dialog_common::test]
    fn it_places_a_new_tile_before_the_first_tile() {
        // before is the only tile / first tile: prev(before) is sentinel-min.
        let l = layout(None, vec![tile("id:a", "n")]);
        let mid = resolve_position_order(&l, Some("id:a"), None).expect("key fits");
        assert!(mid.as_str() < "n");
    }

    #[dialog_common::test]
    fn it_places_a_new_tile_after_an_existing_tile() {
        // before unset + after set: midpoint(after.order, next(after).order).
        let l = layout(
            None,
            vec![tile("id:a", "a"), tile("id:c", "c"), tile("id:e", "e")],
        );
        let mid = resolve_position_order(&l, None, Some("id:c")).expect("key fits");
        assert!(mid.as_str() > "c" && mid.as_str() < "e");
    }

    #[dialog_common::test]
    fn it_places_a_new_tile_after_the_last_tile() {
        // After unset + after set with after being last: next(after) is sentinel-max.
        let l = layout(None, vec![tile("id:a", "a")]);
        let mid = resolve_position_order(&l, None, Some("id:a")).expect("key fits");
        assert!(mid.as_str() > "a");
    }

    #[dialog_common::test]
    fn it_appends_to_the_end_when_neither_bound_is_supplied() {
        let l = layout(None, vec![tile("id:a", "a"), tile("id:b", "b")]);
        let mid = resolve_position_order(&l, None, None).expect("key fits");
        assert!(mid.as_str() > "b");
    }

    #[dialog_common::test]
    fn it_picks_a_first_key_when_the_workspace_is_empty() {
        let l = layout(None, vec![]);
        let mid = resolve_position_order(&l, None, None).expect("key fits");
        assert!(!mid.is_empty());
    }

    #[dialog_common::test]
    fn it_returns_err_when_before_does_not_resolve() {
        let l = layout(None, vec![tile("id:a", "a")]);
        assert!(resolve_position_order(&l, Some("id:missing"), None).is_err());
    }

    #[dialog_common::test]
    fn it_returns_err_when_after_does_not_resolve() {
        let l = layout(None, vec![tile("id:a", "a")]);
        assert!(resolve_position_order(&l, None, Some("id:missing")).is_err());
    }

    // -- resolve_close_focus --------------------------------------

    #[dialog_common::test]
    fn it_leaves_focus_alone_when_closing_an_unfocused_tile() {
        let l = layout(Some("id:a"), vec![tile("id:a", "a"), tile("id:b", "b")]);
        assert_eq!(resolve_close_focus(&l, "id:b"), CloseFocus::Leave);
    }

    #[dialog_common::test]
    fn it_advances_focus_to_the_previous_tile_when_closing_focused() {
        let l = layout(
            Some("id:b"),
            vec![tile("id:a", "a"), tile("id:b", "b"), tile("id:c", "c")],
        );
        assert_eq!(
            resolve_close_focus(&l, "id:b"),
            CloseFocus::SetTo("id:a".to_owned()),
        );
    }

    #[dialog_common::test]
    fn it_advances_focus_to_the_next_tile_when_closing_the_first() {
        let l = layout(Some("id:a"), vec![tile("id:a", "a"), tile("id:b", "b")]);
        assert_eq!(
            resolve_close_focus(&l, "id:a"),
            CloseFocus::SetTo("id:b".to_owned()),
        );
    }

    #[dialog_common::test]
    fn it_clears_focus_when_closing_the_only_tile() {
        let l = layout(Some("id:a"), vec![tile("id:a", "a")]);
        assert_eq!(resolve_close_focus(&l, "id:a"), CloseFocus::Clear);
    }

    // -- focus_tile_doc -------------------------------------------

    #[dialog_common::test]
    fn it_builds_a_focus_tile_doc_pointing_at_the_target() {
        let doc = focus_tile_doc(WORKSPACE, TARGET);
        assert!(doc.contains("workspace!"));
        assert!(doc.contains(&format!("this: {WORKSPACE}")));
        assert!(doc.contains(&format!("focus: {TARGET}")));
    }

    // -- close_tile_doc -------------------------------------------

    #[dialog_common::test]
    fn it_builds_a_close_tile_doc_with_rest_retraction_marker() {
        let doc = close_tile_doc(TARGET, WORKSPACE, CloseFocus::Leave);
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TARGET}")));
        assert!(doc.contains("..: _"));
        // No workspace block when focus stays put.
        assert!(!doc.contains("workspace!"));
    }

    #[dialog_common::test]
    fn it_includes_focus_advance_when_closing_the_focused_tile() {
        let doc = close_tile_doc(
            TARGET,
            WORKSPACE,
            CloseFocus::SetTo("id:neighbour".to_owned()),
        );
        assert!(doc.contains("tile!"));
        assert!(doc.contains("..: _"));
        assert!(doc.contains("workspace!"));
        assert!(doc.contains("focus: id:neighbour"));
    }

    #[dialog_common::test]
    fn it_clears_focus_when_closing_the_last_tile() {
        let doc = close_tile_doc(TARGET, WORKSPACE, CloseFocus::Clear);
        assert!(doc.contains("..: _"));
        assert!(doc.contains("workspace!"));
        assert!(doc.contains("focus: _"));
    }

    // -- reorder_tile_doc -----------------------------------------

    #[dialog_common::test]
    fn it_builds_a_reorder_tile_doc_with_only_order() {
        let doc = reorder_tile_doc(TARGET, "nm");
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TARGET}")));
        assert!(doc.contains(r#"order: "nm""#));
        assert!(!doc.contains("workspace"));
    }

    // -- update_tile_content_doc ----------------------------------

    #[dialog_common::test]
    fn it_returns_an_empty_doc_when_no_field_is_supplied() {
        assert!(update_tile_content_doc(TARGET, None, None, None).is_empty());
    }

    #[dialog_common::test]
    fn it_writes_only_the_supplied_fields() {
        let doc = update_tile_content_doc(TARGET, Some("id:e"), None, Some("person"));
        assert!(doc.contains(&format!("this: {TARGET}")));
        assert!(doc.contains("entity: id:e"));
        assert!(doc.contains(r#"model: "person""#));
        assert!(!doc.contains("view"));
    }

    // -- open_tile_doc --------------------------------------------

    #[dialog_common::test]
    fn it_builds_an_open_tile_doc_under_an_existing_workspace() {
        let doc = open_tile_doc(
            WorkspaceTarget::Existing(WORKSPACE.to_owned()),
            TILE_NEW,
            "n",
            "card",
            "person",
            Some(TARGET),
        );
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TILE_NEW}")));
        assert!(doc.contains(&format!("workspace: {WORKSPACE}")));
        assert!(doc.contains(r#"order: "n""#));
        assert!(doc.contains(r#"view: "card""#));
        assert!(doc.contains(r#"model: "person""#));
        assert!(doc.contains(&format!("entity: {TARGET}")));
        // Atomic focus update in the same doc.
        assert!(doc.contains("workspace!"));
        assert!(doc.contains(&format!("focus: {TILE_NEW}")));
    }

    #[dialog_common::test]
    fn it_omits_entity_from_open_tile_doc_for_concept_list_tiles() {
        let doc = open_tile_doc(
            WorkspaceTarget::Existing(WORKSPACE.to_owned()),
            TILE_NEW,
            "n",
            "concept-list",
            "person",
            None,
        );
        assert!(doc.contains("tile!"));
        assert!(!doc.contains("entity:"));
    }

    #[dialog_common::test]
    fn it_bootstraps_the_workspace_in_one_open_tile_doc() {
        let new_ws = "id:01HMW111111111111111111111";
        let doc = open_tile_doc(
            WorkspaceTarget::Bootstrap {
                id: new_ws.to_owned(),
                name: "default".to_owned(),
            },
            TILE_NEW,
            "n",
            "card",
            "person",
            Some(TARGET),
        );
        // Three blocks: workspace creation, tile, focus.
        assert_eq!(doc.matches("workspace!").count(), 2);
        assert!(doc.contains(&format!("this: {new_ws}")));
        assert!(doc.contains(r#"name: "default""#));
        assert!(doc.contains(&format!("workspace: {new_ws}")));
        assert!(doc.contains(&format!("focus: {TILE_NEW}")));
    }
}
