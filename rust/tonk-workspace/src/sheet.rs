//! `<tonk-sheet>` — a single sheet, like `<wa-tab>`/`<wa-tab-panel>`
//! rolled into one.
//!
//! The view that mounts a sheet sets its attributes from the sheet's
//! fields:
//!
//! - `sheet` — the sheet's entity id (the binder's key).
//! - `order` — a lexicographic sort key for the tab strip.
//! - `title` — the tab label and card-header name.
//! - `subtitle` — a dimmed metadata line shown after the name.
//! - `icon`  — an optional tab icon name.
//!
//! Unlike a bare `<wa-tab-panel>`, the sheet **projects its own card
//! header** (a status dot + the title + the subtitle) from those
//! attributes, the same way [`super::binder`] projects the tab strip
//! from sheet attributes. The header is chrome the sheet owns; the
//! mounting view supplies only the attributes and the content. The
//! sheet's element children are the card body (the panel shown in the
//! canvas when this sheet is active).
//!
//! No shadow DOM: the projected header lives in light DOM so the
//! consuming workspace stylesheet styles it (`.tonk-sheet__head` and
//! friends).

use custom_elements::CustomElement;
use web_sys::{Document, Element, HtmlElement, window};

/// `<tonk-sheet>` — projects a card header from its attributes; its
/// children are the card body.
#[derive(Default)]
pub(crate) struct TonkSheet;

impl CustomElement for TonkSheet {
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["title", "subtitle", "icon"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        project_head(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {}

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        _name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if old == new {
            return;
        }
        project_head(this);
    }
}

/// CSS class names the consuming workspace view styles.
const HEAD: &str = "tonk-sheet__head";
const DOT: &str = "tonk-sheet__dot";
const NAME: &str = "tonk-sheet__name";
const META: &str = "tonk-sheet__meta";

/// Build/refresh the card header (status dot + title + subtitle) from
/// the sheet's attributes, as the sheet's first child. Idempotent.
fn project_head(this: &HtmlElement) {
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let title = this.get_attribute("title").unwrap_or_default();
    let subtitle = this.get_attribute("subtitle").unwrap_or_default();

    let head = ensure_head(this, &document);
    if let Some(name) = head.query_selector(&format!(".{NAME}")).ok().flatten() {
        name.set_text_content(Some(&title));
    }
    if let Some(meta) = head.query_selector(&format!(".{META}")).ok().flatten() {
        meta.set_text_content(Some(&subtitle));
    }
}

/// Find or create the sheet-owned header as the first child.
fn ensure_head(this: &HtmlElement, document: &Document) -> Element {
    if let Ok(Some(existing)) = this.query_selector(&format!(":scope > .{HEAD}")) {
        return existing;
    }
    let head = document.create_element("div").expect("create head div");
    let _ = head.set_attribute("class", HEAD);
    let _ = head.set_attribute("part", "head");

    let dot = document.create_element("span").expect("create dot");
    let _ = dot.set_attribute("class", DOT);
    let name = document.create_element("span").expect("create name");
    let _ = name.set_attribute("class", NAME);
    let meta = document.create_element("span").expect("create meta");
    let _ = meta.set_attribute("class", META);

    let _ = head.append_child(&dot);
    let _ = head.append_child(&name);
    let _ = head.append_child(&meta);

    // Prepend so the header sits above the content children.
    let _ = this.insert_before(&head, this.first_child().as_ref());
    head
}

/// Register `<tonk-sheet>`. Idempotent.
pub(crate) fn register() {
    if already_registered() {
        return;
    }
    TonkSheet::define("tonk-sheet");
}

fn already_registered() -> bool {
    let Some(win) = window() else {
        return false;
    };
    !win.custom_elements().get("tonk-sheet").is_undefined()
}
