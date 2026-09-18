//! The concept panel: what the display matched, and why each field is
//! or is not on screen.
//!
//! Markers on the page can only show what rendered. The panel's job is
//! the opposite — to list every field either side knows about, whether
//! or not it produced a glyph, because the interesting cases all look
//! like a blank space:
//!
//! - the concept declares it and no slot reads it;
//! - a slot reads it and the concept does not declare it;
//! - both agree and this subject simply has no value.
//!
//! [`super::inspect`] decides which of those a field is in; this module
//! draws the result and reports what the pointer is over so the overlay
//! can highlight the matching slots on the page.
//!
//! The panel takes pointer events, unlike the rest of the layer. That
//! is safe because events retargeting to the overlay host do not reach
//! the machine, so hovering a row neither disarms the observation nor
//! reaches the page underneath.

use wasm_bindgen::JsCast;
use web_sys::{Document, Element};

use super::inspect::{Row, Status, rows};
use super::slot::Snapshot;
use super::source::{Piece, pieces};

/// Which half of the panel is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The concept: one row per field, and why it is or is not on
    /// screen.
    Concept,
    /// The view: the template text the display mounted, with every
    /// `{field}` and command marked.
    View,
}

impl Tab {
    fn key(self) -> &'static str {
        match self {
            Tab::Concept => "concept",
            Tab::View => "view",
        }
    }

    fn of(key: &str) -> Option<Self> {
        match key {
            "concept" => Some(Tab::Concept),
            "view" => Some(Tab::View),
            _ => None,
        }
    }
}

/// The panel's DOM and what it currently shows.
pub struct Panel {
    root: Element,
    head: Element,
    subject: Element,
    tabs: Element,
    body: Element,
    tail: Element,
    /// Which half is showing.
    tab: Tab,
    /// The `(signature, subject, tab)` the body was built for, so an
    /// unchanged frame does not rebuild it under the pointer.
    shown: Option<(String, String, &'static str)>,
}

impl Panel {
    /// Build the panel and attach it to `layer`. Hidden until shown.
    pub fn build(document: &Document, layer: &Element) -> Option<Self> {
        let root = div(document, "panel")?;
        let head = div(document, "head")?;
        let subject = div(document, "subject")?;
        let body = div(document, "body")?;
        let tail = div(document, "tail")?;
        let close = document.create_element("button").ok()?;
        let _ = close.set_attribute("class", "close");
        let _ = close.set_attribute("title", "stop observing");
        close.set_text_content(Some("\u{00d7}"));
        let _ = head.append_child(&close);

        let tabs = div(document, "tabs")?;
        for tab in [Tab::Concept, Tab::View] {
            let button = document.create_element("button").ok()?;
            let _ = button.set_attribute("class", "tab");
            let _ = button.set_attribute("data-tab", tab.key());
            button.set_text_content(Some(tab.key()));
            let _ = tabs.append_child(&button);
        }

        for part in [&head, &tabs, &subject, &body, &tail] {
            let _ = root.append_child(part);
        }
        let _ = layer.append_child(&root);
        Some(Self {
            root,
            head,
            subject,
            tabs,
            body,
            tail,
            tab: Tab::Concept,
            shown: None,
        })
    }

    /// Switch halves. A no-op if already there.
    pub fn select(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.shown = None;
        }
    }

    /// The tab a click landed on, if it landed on one.
    pub fn tab_of(target: &Element) -> Option<Tab> {
        let button = target.closest(".tab").ok().flatten()?;
        Tab::of(&button.get_attribute("data-tab")?)
    }

    /// The panel root, for installing listeners on.
    pub fn root(&self) -> &Element {
        &self.root
    }

    /// Stop showing anything.
    pub fn hide(&mut self) {
        let _ = self.root.set_attribute("style", "display:none");
        self.shown = None;
    }

    /// Draw `snapshot` for `subject`. `signature` identifies the frame,
    /// so a repeat call with the same frame and subject is free.
    pub fn show(
        &mut self,
        document: &Document,
        snapshot: &Snapshot,
        signature: &str,
        subject: Option<&str>,
        truncated: bool,
    ) {
        let _ = self.root.set_attribute("style", "display:flex");
        let key = (
            signature.to_owned(),
            subject.unwrap_or_default().to_owned(),
            self.tab.key(),
        );
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);

        self.draw_head(snapshot);
        self.draw_tabs();
        self.draw_subject(snapshot, subject);
        match self.tab {
            Tab::Concept => self.draw_rows(document, &rows(snapshot, subject)),
            Tab::View => self.draw_source(document, snapshot),
        }
        self.draw_tail(snapshot, truncated);
    }

    fn draw_tabs(&self) {
        let Ok(buttons) = self.tabs.query_selector_all(".tab") else {
            return;
        };
        for index in 0..buttons.length() {
            let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let selected = button.get_attribute("data-tab").as_deref() == Some(self.tab.key());
            let _ = button.set_attribute("class", if selected { "tab on" } else { "tab" });
        }
    }

    /// The template text, with every `{field}` and command marked.
    ///
    /// Marked spans carry `data-field`, the same key a concept row
    /// carries, so the highlight is two-way: rest on a row and its
    /// occurrences light up here, rest on an occurrence and its slots
    /// light up on the page.
    fn draw_source(&self, document: &Document, snapshot: &Snapshot) {
        self.body.set_inner_html("");
        let Some(template) = snapshot.template.as_deref() else {
            if let Some(empty) = div(document, "empty") {
                empty.set_text_content(Some("no view template mounted"));
                let _ = self.body.append_child(&empty);
            }
            return;
        };
        let Some(source) = div(document, "source") else {
            return;
        };
        for piece in pieces(template) {
            let (class, field) = match &piece {
                Piece::Literal { .. } => ("lit", None),
                Piece::Field { name, .. } => ("ref", Some(name.clone())),
                Piece::Command { name, .. } => ("cmd", Some(name.clone())),
            };
            let Ok(span) = document.create_element("span") else {
                continue;
            };
            let _ = span.set_attribute("class", class);
            if let Some(field) = field {
                let _ = span.set_attribute("data-field", &field);
            }
            span.set_text_content(Some(piece.text()));
            let _ = source.append_child(&span);
        }
        let _ = self.body.append_child(&source);
    }

    fn draw_head(&self, snapshot: &Snapshot) {
        let model = snapshot
            .model
            .clone()
            .or_else(|| snapshot.model_entity.clone())
            .unwrap_or_else(|| "?".to_owned());
        let facet = snapshot.facet.clone().unwrap_or_else(|| "?".to_owned());
        let mode = if snapshot.directory {
            "directory"
        } else {
            "detail"
        };
        // The close button is the head's only surviving child, so set
        // the text through a dedicated node rather than replacing the
        // head's content and losing it.
        let text = format!("{model} \u{00b7} {facet} \u{00b7} {mode}");
        let _ = self.head.set_attribute("data-title", &text);
    }

    fn draw_subject(&self, snapshot: &Snapshot, subject: Option<&str>) {
        let total = snapshot.subject_count();
        let text = match subject {
            Some(subject) => {
                let position = snapshot
                    .entities
                    .iter()
                    .position(|entity| entity.this == subject);
                match position {
                    Some(index) if total > 1 => {
                        format!("{subject}   ({} of {total})", index + 1)
                    }
                    _ => subject.to_owned(),
                }
            }
            None if total == 0 => "no subject in this frame".to_owned(),
            None => format!("{total} subject(s) — point at a row"),
        };
        self.subject.set_text_content(Some(&text));
    }

    fn draw_rows(&self, document: &Document, rows: &[Row]) {
        self.body.set_inner_html("");
        if rows.is_empty() {
            if let Some(empty) = div(document, "empty") {
                empty.set_text_content(Some("this concept declares no fields"));
                let _ = self.body.append_child(&empty);
            }
            return;
        }
        for row in rows {
            let Some(element) = div(document, &row_class(row)) else {
                continue;
            };
            let _ = element.set_attribute("data-field", &row.name);

            cell(document, &element, "name", &row.name);
            cell(
                document,
                &element,
                "sig",
                &row.declared
                    .as_ref()
                    .map(super::slot::Field::signature)
                    .unwrap_or_default(),
            );
            let value = cell(
                document,
                &element,
                "value",
                row.value.as_deref().unwrap_or(""),
            );
            // The column truncates; the native tooltip does not. A
            // value you cannot read in full is a panel that answered
            // half the question.
            if let (Some(value), Some(full)) = (value, row.value.as_deref())
                && !full.is_empty()
            {
                let _ = value.set_attribute("title", full);
            }
            cell(document, &element, "note", &note(row));
            let _ = self.body.append_child(&element);
        }
    }

    fn draw_tail(&self, snapshot: &Snapshot, truncated: bool) {
        let mut text = match &snapshot.template {
            Some(template) => format!(
                "view template \u{00b7} {} bytes \u{00b7} {} slot(s)",
                template.len(),
                snapshot.slots.len()
            ),
            None => "no view template mounted".to_owned(),
        };
        if truncated {
            text.push_str(" \u{00b7} not all of them are marked on the page");
        }
        self.tail.set_text_content(Some(&text));
    }
}

/// The note column: the finding, or how many slots render the field.
fn note(row: &Row) -> String {
    if row.status.is_finding() {
        return row.status.label().to_owned();
    }
    match (row.slots.len(), row.blank_slots) {
        (1, 0) => String::new(),
        (n, 0) => format!("{n} slots"),
        (n, blank) => format!("{n} slots, {blank} blank"),
    }
}

fn row_class(row: &Row) -> String {
    let status = match row.status {
        Status::Rendered => "rendered",
        Status::Absent => "absent",
        Status::Unrendered => "unrendered",
        Status::Undeclared => "undeclared",
    };
    format!("row {status}")
}

fn cell(document: &Document, row: &Element, class: &str, text: &str) -> Option<Element> {
    let element = div(document, class)?;
    element.set_text_content(Some(text));
    let _ = row.append_child(&element);
    Some(element)
}

fn div(document: &Document, class: &str) -> Option<Element> {
    let element = document.create_element("div").ok()?;
    let _ = element.set_attribute("class", class);
    Some(element)
}

/// The field a hovered part of the panel refers to — a concept row, or
/// a marked span in the template. Both carry `data-field`, so one
/// lookup serves either half.
pub fn field_of(target: &Element) -> Option<String> {
    let holder = target.closest("[data-field]").ok().flatten()?;
    holder.get_attribute("data-field")
}

/// Everything the panel draws. Concatenated into the overlay's sheet.
pub const CSS: &str = "\
.panel { position: fixed; display: none; flex-direction: column; right: 8px; bottom: 8px;
         width: min(46ch, calc(100vw - 16px)); max-height: min(52vh, 520px);
         pointer-events: auto; color: #e9e9ee; background: rgba(20,20,24,.95);
         border-radius: 4px; overflow: hidden; box-shadow: 0 6px 24px rgba(0,0,0,.35); }
.head { position: relative; padding: 6px 24px 6px 8px; font-weight: 600;
        border-bottom: 1px solid rgba(255,255,255,.12); }
.head::before { content: attr(data-title); }
.close { position: absolute; top: 3px; right: 4px; width: 18px; height: 18px; padding: 0;
         font: inherit; line-height: 16px; color: inherit; cursor: pointer;
         background: transparent; border: 0; border-radius: 2px; }
.close:hover { background: rgba(255,255,255,.14); }
.tabs { display: flex; gap: 2px; padding: 4px 8px 0; }
.tab { padding: 2px 8px; font: inherit; color: #9aa0ad; cursor: pointer;
       background: transparent; border: 0; border-radius: 2px 2px 0 0; }
.tab:hover { color: #e9e9ee; background: rgba(255,255,255,.08); }
.tab.on { color: #e9e9ee; background: rgba(255,255,255,.14); }
.source { padding: 6px 8px; white-space: pre-wrap; word-break: break-word; }
.source .lit { color: #9aa0ad; }
.source .ref { color: #22a06b; background: color-mix(in srgb, #22a06b 16%, transparent);
               border-radius: 2px; }
.source .cmd { color: #f06595; background: color-mix(in srgb, #d6336c 18%, transparent);
               border-radius: 2px; }
.source [data-field]:hover { outline: 1px solid currentColor; }
.subject { padding: 4px 8px; color: #9aa0ad; word-break: break-all;
           border-bottom: 1px solid rgba(255,255,255,.08); }
.body { overflow: auto; }
.row { display: grid; grid-template-columns: minmax(6ch, 1fr) auto minmax(8ch, 1.4fr) auto;
       gap: 6px; align-items: baseline; padding: 3px 8px; }
.row:hover { background: rgba(255,255,255,.08); }
.row .name { color: #22a06b; overflow: hidden; text-overflow: ellipsis; }
.row .sig { color: #6f7684; }
.row .value { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row .note { color: #9aa0ad; }
.row.absent .value::before { content: '\\2014'; color: #6f7684; }
.row.absent .note, .row.unrendered .note { color: #bf8700; }
.row.undeclared .name { color: #c92a2a; }
.row.undeclared .note { color: #c92a2a; }
.empty { padding: 6px 8px; color: #9aa0ad; }
.tail { padding: 4px 8px; color: #6f7684;
        border-top: 1px solid rgba(255,255,255,.08); }
";
