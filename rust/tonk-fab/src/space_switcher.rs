//! `<ui-space-switcher>` — the FAB's space-switcher menu: one row per space
//! the profile knows about, plus the static "all spots" / "new" actions.
//!
//! Built on the shared `subscribing` scaffolding, like `<ui-space-name>` and
//! `<ui-member-roster>` — but it is the odd one out: it reads the PROFILE
//! branch, not a space, so its routing context is the fixed literal
//! `main@profile:tonk` rather than a space-derived `main@{did}`. It overrides
//! [`subscribing::Subscribing::resolve_with`] rather than relying on the
//! scaffolding's `space`-attribute default, proving that seam accepts either
//! shape.
//!
//! Reads `xyz.tonk.replica/{subject,kind,status}` through ONE inline
//! directory-mode predicate (`this` unbound, so every replica record returns
//! as a row) — see [`crate::logic::space_list_query_body`]. `name` is
//! deliberately absent: each row renders the target space's OWN repo name via
//! a nested `<ui-space-name space={subject}>`, since the profile-side replica
//! name goes stale (`profile.yaml` states this trade for the active-space
//! chip; the same trade applies here). No concept is named, so nothing seeded
//! on the profile branch is consulted.
//!
//! Filtering mirrors the deleted seeded `fab-menu` view: the profile's own
//! self-replica (`kind == "tonk:profile"`) never renders as a row, and the
//! active space (this element's `exclude` attribute) is skipped too — so the
//! switcher never offers to navigate to the space you're already on. A
//! surviving row stamps `data-status` from the replica's sync status so the
//! existing CSS can dim a still-seeding spot.
//!
//! The static "all spots" and "new" action items render AFTER the rows,
//! copied verbatim from the deleted view (`profile.yaml`, search
//! `fab__menu-item--action`) — this element now owns that markup instead of
//! depending on a per-space-seeded view for it.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use web_sys::{Document, HtmlElement, window};

use crate::logic::space_list_query_body;
use crate::subscribing;

const SUB_TAG: &str = "ui-space-switcher";

/// The PROFILE branch's own routing context — fixed, not derived from any
/// attribute on this element.
const PROFILE_WITH: &str = "main@profile:tonk";

/// The replica kind marking the profile's own self-replica row, hidden from
/// the switcher (there is nothing to switch TO by navigating to yourself).
const PROFILE_KIND: &str = "tonk:profile";

/// One replica row: the fields the switcher needs to decide whether to skip
/// it and what to render if it doesn't.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    subject: String,
    kind: String,
    status: String,
}

#[derive(Default)]
pub struct UiSpaceSwitcherElement {
    scaffold: subscribing::Scaffold,
    /// The live replica set, keyed by each row's entity `this` (NOT
    /// `subject` — `this` is the row's own unique id) so an `update` delta
    /// can upsert/retract individual rows without a full snapshot. Order is
    /// insertion order.
    rows: Rc<RefCell<Vec<(String, Row)>>>,
}

impl CustomElement for UiSpaceSwitcherElement {
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(SpaceSwitcherBehaviour {
            rows: self.rows.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: the fixed PROFILE
/// routing context, the directory-mode replica-list query, and rendering
/// delivered frames as switcher rows.
struct SpaceSwitcherBehaviour {
    rows: Rc<RefCell<Vec<(String, Row)>>>,
}

impl subscribing::Subscribing for SpaceSwitcherBehaviour {
    fn resolve_with(&self, _this: &HtmlElement) -> Option<String> {
        // Unlike `<ui-space-name>`/`<ui-member-roster>`, this element's
        // routing context is never derived from an attribute — it always
        // reads the PROFILE branch.
        Some(PROFILE_WITH.to_owned())
    }

    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        // Directory mode binds no subject and names no concept — nothing
        // seeded on the profile branch is consulted.
        Ok(space_list_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let conclusions = js_sys::Array::from(payload);
        let mut rows = self.rows.borrow_mut();
        rows.clear();
        for i in 0..conclusions.length() {
            if let Some(row) = read_row(&conclusions.get(i)) {
                rows.push(row);
            }
        }
        render_menu(host, &rows);
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let retracted = Reflect::get(payload, &"retracted".into()).unwrap_or(JsValue::UNDEFINED);
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        let mut rows = self.rows.borrow_mut();

        let retracted_rows = js_sys::Array::from(&retracted);
        for i in 0..retracted_rows.length() {
            if let Some((id, _)) = read_row(&retracted_rows.get(i)) {
                rows.retain(|(existing_id, _)| existing_id != &id);
            }
        }

        let asserted_rows = js_sys::Array::from(&asserted);
        for i in 0..asserted_rows.length() {
            if let Some((id, row)) = read_row(&asserted_rows.get(i)) {
                match rows.iter_mut().find(|(existing_id, _)| existing_id == &id) {
                    Some(existing) => existing.1 = row,
                    None => rows.push((id, row)),
                }
            }
        }

        render_menu(host, &rows);
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

/// Read `(row.this, Row { subject, kind, status })` off a raw subscription
/// row. `None` for a missing/empty row, a missing entity id, or any missing
/// field — mirroring the query's requirement that all three fields (and so
/// the row's `this`) are present for a row to appear at all.
fn read_row(row: &JsValue) -> Option<(String, Row)> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let subject = Reflect::get(&fields, &"subject".into())
        .ok()
        .and_then(|v| v.as_string())?;
    let kind = Reflect::get(&fields, &"kind".into())
        .ok()
        .and_then(|v| v.as_string())?;
    let status = Reflect::get(&fields, &"status".into())
        .ok()
        .and_then(|v| v.as_string())?;
    Some((
        this_id,
        Row {
            subject,
            kind,
            status,
        },
    ))
}

/// Rebuild the host's children: one `<a class="fab__menu-item">` per
/// surviving row (the self-replica and the `exclude`d active space filtered
/// out), each wrapping a nested `<ui-space-name>` for that space's OWN repo
/// name, followed by the static "all spots" / "new" action items — the
/// markup the deleted `fab-menu` view used to supply.
fn render_menu(host: &HtmlElement, rows: &[(String, Row)]) {
    while let Some(child) = host.first_child() {
        let _ = host.remove_child(&child);
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let exclude = host.get_attribute("exclude").unwrap_or_default();

    for (_, row) in rows {
        if row.kind == PROFILE_KIND || row.subject == exclude {
            continue;
        }
        let Ok(anchor) = document.create_element("a") else {
            continue;
        };
        let _ = anchor.set_attribute("class", "fab__menu-item");
        let _ = anchor.set_attribute("href", &format!("/space/{}", row.subject));
        let _ = anchor.set_attribute("data-status", &row.status);

        if let Ok(name_el) = document.create_element("ui-space-name") {
            let _ = name_el.set_attribute("space", &row.subject);
            let _ = anchor.append_child(&name_el);
        }
        let _ = host.append_child(&anchor);
    }

    append_action_items(host, &document);
}

/// The static "all spots" / "new" action items, copied verbatim (classes,
/// glyphs, labels, `data-dialog`) from the deleted `fab-menu` view
/// (`profile.yaml`, `fab__menu-item--action`).
fn append_action_items(host: &HtmlElement, document: &Document) {
    if let Ok(all_spots) = document.create_element("a") {
        let _ = all_spots.set_attribute("class", "fab__menu-item fab__menu-item--action");
        let _ = all_spots.set_attribute("href", "/");
        append_glyph(document, &all_spots, "\u{21B0}");
        let label = document.create_text_node(" all spots");
        let _ = all_spots.append_child(&label);
        let _ = host.append_child(&all_spots);
    }

    if let Ok(new_button) = document.create_element("button") {
        let _ = new_button.set_attribute("class", "fab__menu-item fab__menu-item--action");
        let _ = new_button.set_attribute("type", "button");
        let _ = new_button.set_attribute("data-dialog", "open fab-space-create");
        append_glyph(document, &new_button, "+");
        let label = document.create_text_node(" new");
        let _ = new_button.append_child(&label);
        let _ = host.append_child(&new_button);
    }
}

/// Append a `<span class="fab__menu-glyph" aria-hidden="true">{glyph}</span>`
/// child to `parent`, matching the deleted view's action-item markup.
fn append_glyph(document: &Document, parent: &web_sys::Element, glyph: &str) {
    let Ok(span) = document.create_element("span") else {
        return;
    };
    let _ = span.set_attribute("class", "fab__menu-glyph");
    let _ = span.set_attribute("aria-hidden", "true");
    span.set_text_content(Some(glyph));
    let _ = parent.append_child(&span);
}

/// Register `<ui-space-switcher>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiSpaceSwitcherElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
