//! Notation-document builders for layout mutations + the
//! wasm-only `/evaluate` POST that submits them.
//!
//! Every mutation that changes branch state — moving a column,
//! resizing a tile, opening or closing a tile, setting focus —
//! goes through one of the builders here. The builders are pure
//! string-format functions, native-testable; the POST and the
//! debounce machinery sit behind `cfg(target_arch = "wasm32")`.
//!
//! All builders assume the target entity already exists on the
//! branch (the writer is for mutations, not creation outside the
//! `create_*` family). Partial-field updates are safe under
//! dialog-yaml semantics because the analyzer skips the "incomplete
//! fresh-entity" check when `this:` resolves to a known entity.

// Wasm-side consumers (interact.rs, attribute_changed_callback)
// arrive later; until then the builders are exercised by native
// tests only.
#![allow(dead_code)]

/// Build the notation document for moving a column to a new lex
/// order key. Width and tile membership are untouched.
pub fn move_column_doc(column_entity: &str, new_order: &str) -> String {
    format!(
        "column!:\n  this: {column_entity}\n  order: {}\n",
        quoted(new_order),
    )
}

/// Build the notation document for resizing a column.
pub fn resize_column_doc(column_entity: &str, new_width: f64) -> String {
    format!(
        "column!:\n  this: {column_entity}\n  width: {}\n",
        float(new_width),
    )
}

/// Build the notation document for moving a tile — possibly to a
/// different column, possibly within the same column. One document,
/// one dialog transaction: the `column` and `order` claims are
/// re-asserted atomically.
pub fn move_tile_doc(tile_entity: &str, new_column: &str, new_order: &str) -> String {
    format!(
        "tile!:\n  this: {tile_entity}\n  column: {new_column}\n  order: {}\n",
        quoted(new_order),
    )
}

/// Build the notation document for resizing a tile.
pub fn resize_tile_doc(tile_entity: &str, new_height: f64) -> String {
    format!(
        "tile!:\n  this: {tile_entity}\n  height: {}\n",
        float(new_height),
    )
}

/// Build the notation document for closing a tile — retracts every
/// claim on the tile entity via the rest-retraction marker.
pub fn close_tile_doc(tile_entity: &str) -> String {
    format!("tile!:\n  this: {tile_entity}\n  ..: _\n")
}

/// Build the notation document for setting the workspace's focused
/// tile. Re-asserting a cardinality:one field retracts the previous
/// value automatically.
pub fn set_focus_doc(workspace_entity: &str, focused_tile: &str) -> String {
    format!("workspace!:\n  this: {workspace_entity}\n  focus: {focused_tile}\n")
}

/// Build the notation document for clearing focus.
pub fn clear_focus_doc(workspace_entity: &str) -> String {
    format!("workspace!:\n  this: {workspace_entity}\n  focus: _\n")
}

/// Build the notation document for opening a `kind: "display"` tile
/// in an existing column. `new_tile_id` is the caller-minted ULID
/// (typically from `ulid::new_ulid()`); the resulting tile's `this:`
/// becomes `id:<new_tile_id>` so subsequent edits target the same
/// entity instead of content-addressing each body.
pub fn create_display_tile_doc(
    new_tile_id: &str,
    column_entity: &str,
    order: &str,
    height: f64,
    display_entity: &str,
    view_name: &str,
    model_name: &str,
) -> String {
    format!(
        "tile!:\n  \
         this: {new_tile_id}\n  \
         column: {column_entity}\n  \
         order: {}\n  \
         height: {}\n  \
         kind: \"display\"\n  \
         entity: {display_entity}\n  \
         view: {}\n  \
         model: {}\n",
        quoted(order),
        float(height),
        quoted(view_name),
        quoted(model_name),
    )
}

/// Double-quote a string for YAML output. Embedded backslashes /
/// quotes are escaped — Rust's debug format does the right thing.
fn quoted(value: &str) -> String {
    format!("{value:?}")
}

/// Format a float so an integer-valued one still has a `.0` tail —
/// `as: float` attribute fields require float YAML syntax (`1.0`),
/// not integer syntax (`1`).
fn float(value: f64) -> String {
    let s = format!("{value}");
    if s.contains('.') { s } else { format!("{s}.0") }
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
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    const COLUMN: &str = "id:01HMC000000000000000000000";
    const TILE: &str = "id:01HMT000000000000000000000";
    const WORKSPACE: &str = "id:01HMW000000000000000000000";

    #[dialog_common::test]
    fn it_builds_a_move_column_doc_with_only_order() {
        // Width is left untouched so the move doesn't accidentally
        // reset a column the user resized.
        let doc = move_column_doc(COLUMN, "nm");
        assert!(doc.contains("column!"));
        assert!(doc.contains(&format!("this: {COLUMN}")));
        assert!(doc.contains(r#"order: "nm""#));
        assert!(!doc.contains("width"));
    }

    #[dialog_common::test]
    fn it_builds_a_resize_column_doc_with_only_width() {
        let doc = resize_column_doc(COLUMN, 0.5);
        assert!(doc.contains("column!"));
        assert!(doc.contains(&format!("this: {COLUMN}")));
        assert!(doc.contains("width: 0.5"));
        assert!(!doc.contains("order"));
    }

    #[dialog_common::test]
    fn it_builds_a_move_tile_doc_with_column_and_order_in_one_document() {
        // Moving a tile between columns rewrites two claims — they
        // must be in one document so dialog commits them as one
        // transaction (no intermediate "tile under wrong parent"
        // frame).
        let doc = move_tile_doc(TILE, COLUMN, "n");
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TILE}")));
        assert!(doc.contains(&format!("column: {COLUMN}")));
        assert!(doc.contains(r#"order: "n""#));
        // One head — not two separate `tile!:` blocks.
        assert_eq!(doc.matches("tile!").count(), 1);
    }

    #[dialog_common::test]
    fn it_builds_a_resize_tile_doc_with_only_height() {
        let doc = resize_tile_doc(TILE, 0.4);
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TILE}")));
        assert!(doc.contains("height: 0.4"));
        assert!(!doc.contains("width"));
    }

    #[dialog_common::test]
    fn it_builds_a_close_tile_doc_with_rest_retraction_marker() {
        // `..: _` tells the analyzer to retract every claim on the
        // entity that isn't explicitly set in the body — the
        // canonical close form.
        let doc = close_tile_doc(TILE);
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {TILE}")));
        assert!(doc.contains("..: _"));
    }

    #[dialog_common::test]
    fn it_builds_a_set_focus_doc_pointing_at_the_focused_tile() {
        let doc = set_focus_doc(WORKSPACE, TILE);
        assert!(doc.contains("workspace!"));
        assert!(doc.contains(&format!("this: {WORKSPACE}")));
        assert!(doc.contains(&format!("focus: {TILE}")));
    }

    #[dialog_common::test]
    fn it_builds_a_clear_focus_doc_retracting_the_focus_field() {
        // Per-field retraction is `<field>: _`. The analyzer
        // emits a retraction transaction for just `focus` without
        // touching the workspace's `name` claim.
        let doc = clear_focus_doc(WORKSPACE);
        assert!(doc.contains("workspace!"));
        assert!(doc.contains(&format!("this: {WORKSPACE}")));
        assert!(doc.contains("focus: _"));
    }

    #[dialog_common::test]
    fn it_builds_a_create_display_tile_doc_with_every_required_field() {
        let new_id = "id:01HMT999999999999999999999";
        let target = "id:01HENT000000000000000000000";
        let doc = create_display_tile_doc(new_id, COLUMN, "n", 1.0, target, "card", "person");
        assert!(doc.contains("tile!"));
        assert!(doc.contains(&format!("this: {new_id}")));
        assert!(doc.contains(&format!("column: {COLUMN}")));
        assert!(doc.contains(r#"order: "n""#));
        assert!(doc.contains("height: 1"));
        assert!(doc.contains(r#"kind: "display""#));
        assert!(doc.contains(&format!("entity: {target}")));
        assert!(doc.contains(r#"view: "card""#));
        assert!(doc.contains(r#"model: "person""#));
    }
}
