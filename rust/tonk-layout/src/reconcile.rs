//! In-place DOM patcher.
//!
//! Given the host element and the latest folded [`Layout`], walk
//! the existing strip / column / tile tree and bring it in line —
//! inserting new columns and tiles, removing vanished ones,
//! reordering survivors, and updating `flex` sizing /
//! `data-focused` / `<tonk-display>` attributes on nodes that
//! stay put.
//!
//! Node identity is preserved for healthy tiles so that as
//! columns / tiles get reshuffled in response to an `order` edit,
//! `<tonk-display>` instances aren't torn down and recreated. (A
//! move within the document still fires custom-element
//! disconnect/connect callbacks per the DOM spec, restarting
//! subscriptions; this is unavoidable without a CSS-grid
//! reposition scheme and is accepted v1 behaviour.)

use web_sys::{Document, Element, Node, window};

use crate::model::{Column, Layout, Tile};

/// Patch `host`'s DOM to match `layout`.
pub fn reconcile_layout(host: &Element, layout: &Layout) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let strip = ensure_strip(host, &document);

    // Collect existing column nodes by their entity URI; nodes
    // claimed during the walk are removed from the map, so anything
    // left over at the end is stale and gets removed from the DOM.
    let mut existing_columns = collect_keyed_children(&strip, "niri-column");
    // Resize handles between columns are keyed by the entity of the
    // column they sit *after* — the one whose right edge they drag.
    let mut existing_handles = collect_keyed_handles(&strip);

    let mut anchor: Option<Node> = strip.first_child();
    for (i, column) in layout.columns.iter().enumerate() {
        let column_el = existing_columns
            .remove(&column.entity)
            .unwrap_or_else(|| create_column_element(&document, column));
        update_column_attrs(&column_el, column);
        reconcile_tiles(&document, host, &column_el, column, layout.focus.as_deref());
        place_node(&strip, &column_el, &mut anchor);

        // Resize handle between this column and the next (skip
        // after the last column — no right neighbour to resize
        // against).
        if i + 1 < layout.columns.len() {
            let handle_el = existing_handles
                .remove(&column.entity)
                .unwrap_or_else(|| create_resize_handle(&document, &column.entity));
            place_node(&strip, &handle_el, &mut anchor);
        }
    }
    remove_remaining(&existing_columns);
    remove_remaining(&existing_handles);
}

/// Patch a column's tiles in place. Mirror of the column-level walk
/// but with a tile body that mounts / updates `<tonk-display>`.
fn reconcile_tiles(
    document: &Document,
    host: &Element,
    column_el: &Element,
    column: &Column,
    focus: Option<&str>,
) {
    let mut existing = collect_keyed_children(column_el, "niri-tile");
    let mut anchor: Option<Node> = column_el.first_child();
    for tile in &column.tiles {
        let tile_el = existing
            .remove(&tile.entity)
            .unwrap_or_else(|| create_tile_element(document, tile, host));
        update_tile_attrs(&tile_el, tile, focus);
        update_tile_body_attrs(&tile_el, tile, host);
        place_node(column_el, &tile_el, &mut anchor);
    }
    remove_remaining(&existing);
}

/// Find the existing `<div class="niri-strip">` child, or create
/// one and append it. Same shape as the skeleton `element::start`
/// mounts.
fn ensure_strip(host: &Element, document: &Document) -> Element {
    if let Some(existing) = first_child_with_class(host, "niri-strip") {
        return existing;
    }
    let Ok(strip) = document.create_element("div") else {
        // The only realistic failure mode for create_element is an
        // invalid name — `div` is always valid — so this branch is
        // effectively unreachable. We return the host itself as a
        // best-effort fallback to keep the type happy.
        return host.clone();
    };
    let _ = strip.set_attribute("class", "niri-strip");
    let _ = host.append_child(&strip);
    strip
}

fn create_column_element(document: &Document, column: &Column) -> Element {
    let el = document
        .create_element("div")
        .expect("create div always succeeds");
    let _ = el.set_attribute("class", "niri-column");
    let _ = el.set_attribute("data-entity", &column.entity);
    el
}

/// Resize handle that sits between two adjacent columns. Carries
/// `data-after-column` so the pointer handler can look up which
/// column it's dragging in the live layout. The handle is its own
/// flex item with a fixed basis — it doesn't grow with the strip.
fn create_resize_handle(document: &Document, after_column: &str) -> Element {
    let el = document
        .create_element("div")
        .expect("create div always succeeds");
    let _ = el.set_attribute("class", "niri-resize");
    let _ = el.set_attribute("data-after-column", after_column);
    el
}

/// Map handle elements by the column entity they sit after.
fn collect_keyed_handles(parent: &Element) -> std::collections::HashMap<String, Element> {
    let mut out = std::collections::HashMap::new();
    let children = parent.children();
    for i in 0..children.length() {
        let Some(child) = children.item(i) else {
            continue;
        };
        if child.get_attribute("class").as_deref() != Some("niri-resize") {
            continue;
        }
        let Some(after) = child.get_attribute("data-after-column") else {
            continue;
        };
        out.insert(after, child);
    }
    out
}

/// Reflect `width` + `order` onto the column element. `width`
/// becomes a `flex` declaration so the browser does the pixel math
/// from the workspace's fractions; `order` is mirrored as
/// `data-order` for CSS hooks and debugging.
fn update_column_attrs(el: &Element, column: &Column) {
    set_if_changed(el, "data-order", &column.order);
    let flex = format!("flex: {} 1 0;", column.width);
    set_if_changed(el, "style", &flex);
}

fn create_tile_element(document: &Document, tile: &Tile, host: &Element) -> Element {
    let el = document
        .create_element("div")
        .expect("create div always succeeds");
    let _ = el.set_attribute("class", "niri-tile");
    let _ = el.set_attribute("data-entity", &tile.entity);
    // Mount the tile body keyed by `kind`. Display tiles render a
    // single entity via `<tonk-display>`; concept tiles render an
    // entity list via `<tonk-concept>`. Unknown kinds get an inline
    // error placeholder so the strip's geometry stays intact while
    // making the misconfiguration visible.
    match tile.kind.as_str() {
        "display" => {
            if let Ok(display) = document.create_element("tonk-display") {
                copy_routing_attrs(host, &display);
                set_display_attrs(&display, tile);
                let _ = el.append_child(&display);
            }
        }
        "concept" => {
            if let Ok(concept) = document.create_element("tonk-concept") {
                copy_routing_attrs(host, &concept);
                set_concept_source(&concept, tile);
                // Default template — list one row per entity. Uses
                // `{name}` and `{this}` placeholders; concepts that
                // don't project `name` just show the URI. A future
                // iteration can let the tile carry a custom template.
                concept.set_inner_html(CONCEPT_TILE_TEMPLATE);
                let _ = el.append_child(&concept);
            }
        }
        other => {
            if let Ok(placeholder) = document.create_element("div") {
                let _ = placeholder.set_attribute("class", "niri-tile-error");
                let _ = placeholder.set_attribute("data-state", "error");
                placeholder.set_text_content(Some(&format!("unknown tile kind: {other}")));
                let _ = el.append_child(&placeholder);
            }
        }
    }
    el
}

/// Default per-row template for `<tonk-concept>` mounted inside a
/// concept tile. Shows the entity's name when the concept projects
/// one, falling back to the URI; clickable to support future
/// drill-in behaviour.
const CONCEPT_TILE_TEMPLATE: &str = r#"
<ul class="niri-concept-list">
  <template>
    <li class="niri-concept-row" data-entity="{this}">
      <span class="niri-concept-row-name">{name}</span>
      <span class="niri-concept-row-id">{this}</span>
    </li>
  </template>
</ul>
"#;

/// Reflect `height` + focus onto the tile element.
fn update_tile_attrs(el: &Element, tile: &Tile, focus: Option<&str>) {
    let flex = format!("flex: {} 1 0;", tile.height);
    set_if_changed(el, "style", &flex);
    if focus == Some(tile.entity.as_str()) {
        set_if_changed(el, "data-focused", "");
    } else {
        let _ = el.remove_attribute("data-focused");
    }
}

/// Set the `<tonk-concept>` element's `source` attribute from a
/// concept tile's `model` field (which carries the concept name).
fn set_concept_source(concept_el: &Element, tile: &Tile) {
    set_optional(concept_el, "source", tile.display_model.as_deref());
}

/// In-place update for the tile's body child, dispatched on `kind`.
/// Skipping equal-value sets via `set_if_changed` keeps the child
/// element from restarting its subscriptions for cosmetic re-folds.
fn update_tile_body_attrs(tile_el: &Element, tile: &Tile, host: &Element) {
    match tile.kind.as_str() {
        "display" => update_display_attrs(tile_el, tile, host),
        "concept" => {
            if let Some(concept) = tile_el
                .query_selector(":scope > tonk-concept")
                .ok()
                .flatten()
            {
                copy_routing_attrs(host, &concept);
                set_concept_source(&concept, tile);
            }
        }
        _ => {}
    }
}

/// If the tile already has a `<tonk-display>` child, update its
/// attributes from the tile row (no remount). Skipping equal-value
/// sets keeps `<tonk-display>` from restarting its subscriptions
/// for cosmetic re-folds.
fn update_display_attrs(tile_el: &Element, tile: &Tile, host: &Element) {
    if tile.kind != "display" {
        return;
    }
    let Some(display) = tile_el
        .query_selector(":scope > tonk-display")
        .ok()
        .flatten()
    else {
        return;
    };
    copy_routing_attrs(host, &display);
    set_display_attrs(&display, tile);
}

/// Set `entity` / `view` / `model` on a `<tonk-display>` from the
/// tile row's fields. Missing fields are mirrored as removals so a
/// previously-set attribute doesn't linger.
fn set_display_attrs(display: &Element, tile: &Tile) {
    set_optional(display, "entity", tile.display_entity.as_deref());
    set_optional(display, "view", tile.display_view.as_deref());
    set_optional(display, "model", tile.display_model.as_deref());
}

/// Copy the host's `space` / `branch` attributes onto a child node
/// so its own subscriptions route to the same repository / branch.
fn copy_routing_attrs(host: &Element, target: &Element) {
    for name in ["space", "branch"] {
        set_optional(target, name, host.get_attribute(name).as_deref());
    }
}

/// Anchor-driven incremental placement: if `target` is already at
/// `*anchor`'s position, just advance the anchor; otherwise move
/// `target` there. After the call, `*anchor` points to the node
/// that follows `target` in the parent's child list (or `None` if
/// `target` is now the last child).
fn place_node(parent: &Element, target: &Element, anchor: &mut Option<Node>) {
    let target_node: &Node = target.as_ref();
    let already_in_place = anchor
        .as_ref()
        .map(|n| n.is_same_node(Some(target_node)))
        .unwrap_or(false);
    if !already_in_place {
        let _ = parent.insert_before(target_node, anchor.as_ref());
    }
    *anchor = target.next_sibling();
}

/// Drop every node still in `existing` from the DOM — these are
/// columns / tiles whose entities vanished from the layout frame.
fn remove_remaining(existing: &std::collections::HashMap<String, Element>) {
    for el in existing.values() {
        el.remove();
    }
}

/// Build a `data-entity` → element map from `parent`'s direct
/// children whose `class` attribute equals `class_name`. The map
/// is the input to the diff: each walk-step removes the entity
/// being kept, so the leftovers at the end are the stale nodes.
fn collect_keyed_children(
    parent: &Element,
    class_name: &str,
) -> std::collections::HashMap<String, Element> {
    let mut out = std::collections::HashMap::new();
    let children = parent.children();
    for i in 0..children.length() {
        let Some(child) = children.item(i) else {
            continue;
        };
        if child.get_attribute("class").as_deref() != Some(class_name) {
            continue;
        }
        let Some(entity) = child.get_attribute("data-entity") else {
            continue;
        };
        out.insert(entity, child);
    }
    out
}

/// First child of `parent` whose `class` attribute equals
/// `class_name`. Used for the lone `<div class="niri-strip">`
/// container.
fn first_child_with_class(parent: &Element, class_name: &str) -> Option<Element> {
    let children = parent.children();
    for i in 0..children.length() {
        let child = children.item(i)?;
        if child.get_attribute("class").as_deref() == Some(class_name) {
            return Some(child);
        }
    }
    None
}

/// `setAttribute` only when the value actually differs. Cuts the
/// useless `attributeChangedCallback` firings that would otherwise
/// restart `<tonk-display>` on every refold.
fn set_if_changed(el: &Element, name: &str, value: &str) {
    if el.get_attribute(name).as_deref() != Some(value) {
        let _ = el.set_attribute(name, value);
    }
}

/// Same as `set_if_changed`, but a `None` removes the attribute.
fn set_optional(el: &Element, name: &str, value: Option<&str>) {
    match value {
        Some(v) => set_if_changed(el, name, v),
        None => {
            if el.has_attribute(name) {
                let _ = el.remove_attribute(name);
            }
        }
    }
}
