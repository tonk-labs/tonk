//! In-place DOM reconciler for the layout strip.
//!
//! Given a folded [`Layout`], [`Reconciler::apply`] patches the
//! host's DOM to match: it creates the strip container once, then
//! on every frame inserts new columns/tiles, removes departed
//! ones, reorders survivors, and updates their sizing and content
//! descriptor — all keyed by entity URI so an unchanged tile's
//! `<tonk-display>` is never torn down and remounted.
//!
//! The DOM shape (light DOM, no shadow root):
//!
//! ```text
//! <tonk-layout>
//!   <div class="tonk-layout-strip">      <!-- scrolls horizontally -->
//!     <div class="tonk-layout-rail">     <!-- flex row of columns -->
//!       <div class="tonk-layout-column" data-id=…>
//!         <div class="tonk-layout-tile" data-id=…>
//!           <tonk-display entity=… view=… />
//!         </div>
//!       </div>
//!     </div>
//!   </div>
//! </tonk-layout>
//! ```
//!
//! Column width and tile height are written as CSS custom
//! properties (`--tonk-layout-width`, `--tonk-layout-height`) so the layout
//! stays resolution-independent — the stylesheet turns the
//! fractions into pixels.

use wasm_bindgen::JsCast as _;
use web_sys::{Document, Element, window};

use crate::model::{Column, Layout, Tile};

/// CSS class on the strip container — a plain `<div>` that
/// scrolls horizontally. A `<wa-scroller>` was tried here but it
/// does not propagate height to its slotted child, collapsing
/// the rail; a `<div>` with `overflow-x: auto` is two lines of
/// CSS and keeps the height chain intact.
const STRIP_CLASS: &str = "tonk-layout-strip";
/// CSS class on the flex rail inside the strip — the row of
/// columns.
const RAIL_CLASS: &str = "tonk-layout-rail";
/// CSS class on each column.
const COLUMN_CLASS: &str = "tonk-layout-column";
/// CSS class on each tile.
const TILE_CLASS: &str = "tonk-layout-tile";
/// Attribute carrying a column's / tile's entity URI — the
/// reconciliation key.
const ID_ATTR: &str = "data-id";
/// Attribute set on the focused tile.
const FOCUSED_ATTR: &str = "data-focused";

/// Owns the strip container and patches it to match successive
/// [`Layout`] frames.
pub struct Reconciler {
    /// The `<tonk-layout>` host.
    host: Element,
    /// The `<wa-scroller>` strip container, created lazily on the
    /// first [`Reconciler::apply`].
    strip: Option<Element>,
    /// The `.tonk-layout-rail` flex row inside the scroller —
    /// where columns are reconciled.
    rail: Option<Element>,
    /// `space` attribute forwarded to every tile's
    /// `<tonk-display>`.
    space: String,
    /// `branch` attribute forwarded to every tile's
    /// `<tonk-display>`.
    branch: String,
}

impl Reconciler {
    /// Create a reconciler bound to `host`. `space` and `branch`
    /// are forwarded onto every tile's `<tonk-display>` so the
    /// tiles query the same repository the layout came from.
    pub fn new(host: Element, space: String, branch: String) -> Self {
        Self {
            host,
            strip: None,
            rail: None,
            space,
            branch,
        }
    }

    /// Patch the strip to match `layout`.
    pub fn apply(&mut self, layout: &Layout) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let rail = self.ensure_rail(&document);
        reconcile_columns(&document, &rail, layout, &self.space, &self.branch);
    }

    /// Remove the strip and everything in it. Used when the
    /// element disconnects or restarts.
    pub fn clear(&mut self) {
        if let Some(strip) = self.strip.take() {
            strip.remove();
        }
        self.rail = None;
    }

    /// Return the column rail, creating the strip `<div>` and its
    /// inner `.tonk-layout-rail` on first use.
    fn ensure_rail(&mut self, document: &Document) -> Element {
        if let Some(rail) = &self.rail {
            return rail.clone();
        }
        let strip = create(document, "div", STRIP_CLASS);
        let rail = create(document, "div", RAIL_CLASS);
        let _ = strip.append_child(&rail);
        let _ = self.host.append_child(&strip);
        self.strip = Some(strip);
        self.rail = Some(rail.clone());
        rail
    }
}

/// Reconcile the strip's column children against `layout`.
///
/// Departed columns are removed, new ones created, and all
/// columns re-appended in layout order — re-appending an existing
/// node moves it, preserving its identity and its tiles' live
/// `<tonk-display>` subscriptions.
fn reconcile_columns(
    document: &Document,
    strip: &Element,
    layout: &Layout,
    space: &str,
    branch: &str,
) {
    remove_departed(strip, &layout.columns, |c| &c.id);

    for column in &layout.columns {
        let element = find_child(strip, &column.id)
            .unwrap_or_else(|| create_keyed(document, "div", COLUMN_CLASS, &column.id));
        apply_column(document, &element, column, &layout.focus, space, branch);
        // Append in iteration order: a no-op when already last,
        // a move otherwise. After the loop the children sit in
        // exactly `layout` order.
        let _ = strip.append_child(&element);
    }
}

/// Patch one column element: its width custom property and its
/// tile children.
fn apply_column(
    document: &Document,
    element: &Element,
    column: &Column,
    focus: &Option<String>,
    space: &str,
    branch: &str,
) {
    set_fraction(element, "--tonk-layout-width", column.width);

    remove_departed(element, &column.tiles, |t| &t.id);

    for tile in &column.tiles {
        let tile_el = find_child(element, &tile.id)
            .unwrap_or_else(|| create_keyed(document, "div", TILE_CLASS, &tile.id));
        apply_tile(document, &tile_el, tile, focus, space, branch);
        let _ = element.append_child(&tile_el);
    }
}

/// Patch one tile element: its height custom property, focused
/// flag, and the `<tonk-display>` it hosts.
fn apply_tile(
    document: &Document,
    element: &Element,
    tile: &Tile,
    focus: &Option<String>,
    space: &str,
    branch: &str,
) {
    set_fraction(element, "--tonk-layout-height", tile.height);

    if focus.as_deref() == Some(tile.id.as_str()) {
        let _ = element.set_attribute(FOCUSED_ATTR, "");
    } else {
        let _ = element.remove_attribute(FOCUSED_ATTR);
    }

    reconcile_display(document, element, tile, space, branch);
}

/// Ensure the tile hosts a `<tonk-display>` matching its content
/// descriptor.
///
/// A tile with no `entity` carries no display. A tile with one
/// gets a single `<tonk-display>` child; its attributes are
/// updated in place so `<tonk-display>`'s own
/// `attribute_changed_callback` restarts its flows without the
/// element being remounted.
fn reconcile_display(
    document: &Document,
    tile_el: &Element,
    tile: &Tile,
    space: &str,
    branch: &str,
) {
    let existing = first_element_child(tile_el);

    let Some(entity) = &tile.entity else {
        // Empty tile — drop any stale display.
        if let Some(display) = existing {
            display.remove();
        }
        return;
    };

    let display = match existing {
        Some(display) => display,
        None => {
            let Ok(display) = document.create_element("tonk-display") else {
                return;
            };
            let _ = tile_el.append_child(&display);
            display
        }
    };

    set_or_remove(&display, "entity", Some(entity));
    set_or_remove(&display, "view", tile.view.as_deref());
    set_or_remove(&display, "model", tile.model.as_deref());
    set_or_remove(&display, "space", Some(space));
    set_or_remove(&display, "branch", Some(branch));
}

/// Remove every keyed child of `parent` whose key is not present
/// in `keep`.
fn remove_departed<T>(parent: &Element, keep: &[T], key: impl Fn(&T) -> &String) {
    let children = parent.children();
    // Collect departed nodes first — removing while iterating a
    // live `HtmlCollection` would shift indices underfoot.
    let mut departed = Vec::new();
    for i in 0..children.length() {
        let Some(child) = children.item(i) else {
            continue;
        };
        let present = child
            .get_attribute(ID_ATTR)
            .map(|id| keep.iter().any(|item| key(item) == &id))
            .unwrap_or(false);
        if !present {
            departed.push(child);
        }
    }
    for child in departed {
        child.remove();
    }
}

/// Find a direct child of `parent` whose [`ID_ATTR`] equals `id`.
fn find_child(parent: &Element, id: &str) -> Option<Element> {
    let children = parent.children();
    for i in 0..children.length() {
        let child = children.item(i)?;
        if child.get_attribute(ID_ATTR).as_deref() == Some(id) {
            return Some(child);
        }
    }
    None
}

/// First element child of `parent`, if any.
fn first_element_child(parent: &Element) -> Option<Element> {
    parent.children().item(0)
}

/// Create an element with `class`.
fn create(document: &Document, tag: &str, class: &str) -> Element {
    let element = document
        .create_element(tag)
        .expect("create_element never fails for a static tag");
    let _ = element.set_attribute("class", class);
    element
}

/// Create an element with `class` and a [`ID_ATTR`] key.
fn create_keyed(document: &Document, tag: &str, class: &str, id: &str) -> Element {
    let element = create(document, tag, class);
    let _ = element.set_attribute(ID_ATTR, id);
    element
}

/// Write a `0..1` fraction as a CSS custom property on `element`'s
/// inline style. Falls through silently if the element is not an
/// `HtmlElement` (it always is here).
fn set_fraction(element: &Element, property: &str, fraction: f64) {
    if let Some(html) = element.dyn_ref::<web_sys::HtmlElement>() {
        let _ = html.style().set_property(property, &format!("{fraction}"));
    }
}

/// Set `attr` to `value`, or remove it when `value` is `None`.
fn set_or_remove(element: &Element, attr: &str, value: Option<&str>) {
    match value {
        Some(value) => {
            let _ = element.set_attribute(attr, value);
        }
        None => {
            let _ = element.remove_attribute(attr);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column, Tile};
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Build a detached `<tonk-layout>` host for a test.
    fn host() -> Element {
        window()
            .expect("window")
            .document()
            .expect("document")
            .create_element("tonk-layout")
            .expect("create host")
    }

    /// Shorthand for a `Tile` with the given id and entity.
    fn tile(id: &str, entity: Option<&str>) -> Tile {
        Tile {
            id: id.to_string(),
            order: 1.0,
            height: 1.0,
            entity: entity.map(str::to_string),
            view: None,
            model: None,
        }
    }

    /// Shorthand for a `Column` holding `tiles`.
    fn column(id: &str, tiles: Vec<Tile>) -> Column {
        Column {
            id: id.to_string(),
            order: 1.0,
            width: 0.5,
            tiles,
        }
    }

    #[dialog_common::test]
    fn it_renders_columns_and_tiles_into_the_strip() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        let layout = Layout {
            focus: None,
            columns: vec![
                column("col:a", vec![tile("tile:1", Some("did:key:zE1"))]),
                column("col:b", vec![tile("tile:2", None)]),
            ],
        };
        reconciler.apply(&layout);

        let strip = host
            .query_selector(".tonk-layout-rail")
            .unwrap()
            .expect("rail mounted");
        assert_eq!(strip.children().length(), 2, "two columns");
        let displays = host.query_selector_all("tonk-display").unwrap();
        assert_eq!(displays.length(), 1, "only the tile with an entity");
    }

    #[dialog_common::test]
    fn it_preserves_a_tiles_display_node_across_frames() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        let frame = || Layout {
            focus: None,
            columns: vec![column("col:a", vec![tile("tile:1", Some("did:key:zE1"))])],
        };
        reconciler.apply(&frame());
        let first = host
            .query_selector("tonk-display")
            .unwrap()
            .expect("display mounted");

        // A second identical frame must keep the very same node.
        reconciler.apply(&frame());
        let second = host.query_selector("tonk-display").unwrap().unwrap();
        assert!(first.is_same_node(Some(&second)), "display was remounted");
    }

    #[dialog_common::test]
    fn it_removes_a_departed_column() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:a", vec![]), column("col:b", vec![])],
        });
        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:b", vec![])],
        });
        let strip = host.query_selector(".tonk-layout-rail").unwrap().unwrap();
        assert_eq!(strip.children().length(), 1);
        assert_eq!(
            strip
                .children()
                .item(0)
                .unwrap()
                .get_attribute("data-id")
                .as_deref(),
            Some("col:b"),
        );
    }

    #[dialog_common::test]
    fn it_reorders_columns_to_match_layout_order() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:a", vec![]), column("col:b", vec![])],
        });
        // Swap the order.
        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:b", vec![]), column("col:a", vec![])],
        });
        let strip = host.query_selector(".tonk-layout-rail").unwrap().unwrap();
        let ids: Vec<Option<String>> = (0..strip.children().length())
            .map(|i| strip.children().item(i).unwrap().get_attribute("data-id"))
            .collect();
        assert_eq!(ids, [Some("col:b".to_string()), Some("col:a".to_string())]);
    }

    #[dialog_common::test]
    fn it_marks_the_focused_tile() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        reconciler.apply(&Layout {
            focus: Some("tile:1".to_string()),
            columns: vec![column(
                "col:a",
                vec![tile("tile:1", None), tile("tile:2", None)],
            )],
        });
        assert_eq!(
            host.query_selector_all("[data-focused]").unwrap().length(),
            1,
        );
        let focused = host
            .query_selector("[data-focused]")
            .unwrap()
            .expect("a focused tile");
        assert_eq!(focused.get_attribute("data-id").as_deref(), Some("tile:1"));
    }

    #[dialog_common::test]
    fn it_drops_the_display_when_a_tile_loses_its_entity() {
        let host = host();
        let mut reconciler = Reconciler::new(host.clone(), "home".into(), "main".into());
        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:a", vec![tile("tile:1", Some("did:key:zE1"))])],
        });
        assert_eq!(host.query_selector_all("tonk-display").unwrap().length(), 1);

        reconciler.apply(&Layout {
            focus: None,
            columns: vec![column("col:a", vec![tile("tile:1", None)])],
        });
        assert_eq!(host.query_selector_all("tonk-display").unwrap().length(), 0);
    }
}
