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

use web_sys::{Document, Element};

use super::inspect::{Row, Status, rows};
use super::slot::Snapshot;

/// The panel's DOM and what it currently shows.
pub struct Panel {
    root: Element,
    head: Element,
    subject: Element,
    body: Element,
    tail: Element,
    /// The `(signature, subject)` the rows were built for, so an
    /// unchanged frame does not rebuild them under the pointer.
    shown: Option<(String, String)>,
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
        for part in [&head, &subject, &body, &tail] {
            let _ = root.append_child(part);
        }
        let _ = layer.append_child(&root);
        Some(Self {
            root,
            head,
            subject,
            body,
            tail,
            shown: None,
        })
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
        let key = (signature.to_owned(), subject.unwrap_or_default().to_owned());
        if self.shown.as_ref() == Some(&key) {
            return;
        }
        self.shown = Some(key);

        self.draw_head(snapshot);
        self.draw_subject(snapshot, subject);
        self.draw_rows(document, &rows(snapshot, subject));
        self.draw_tail(snapshot, truncated);
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
            // What a hover on this row highlights on the page.
            let ids = row
                .slots
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let _ = element.set_attribute("data-slots", &ids);
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

/// The slot ids a panel row names, read back off the row element.
pub fn slots_of(row: &Element) -> Vec<u32> {
    row.get_attribute("data-slots")
        .unwrap_or_default()
        .split(',')
        .filter_map(|id| id.parse().ok())
        .collect()
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
