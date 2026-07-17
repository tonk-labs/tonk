//! `<ui-member-roster>` — a space's member roster, read live from its own
//! branch and rendered as the FAB's roster menu rows.
//!
//! Built on the shared subscribing scaffolding in [`crate::subscribing`]:
//! `shadow() -> false`, an observed `space` attribute, its own stamped
//! `with="main@{did}"`, plain `consumer::subscribe`, bounded retry, and
//! structural frame consumption via `reset`/`update` delegates. See that
//! module's doc for why frame consumption is structural rather than
//! optional — an element that subscribes and never renders is the exact bug
//! this whole scaffolding exists to catch.
//!
//! Reads all three `xyz.tonk.membership/*` fields through ONE inline
//! directory-mode predicate (`this` unbound, so every member returns as a
//! row) — see [`crate::logic::member_roster_query_body`]. No concept is
//! named, so nothing seeded on the space's branch is consulted.
//!
//! Renders one `<span class="fab__menu-item fab__menu-item--member">{name}
//! </span>` per member — the markup the deleted `fab-roster` view used to
//! supply.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlElement, window};

use crate::logic::member_roster_query_body;
use crate::subscribing;

const SUB_TAG: &str = "ui-member-roster";

#[derive(Default)]
pub struct UiMemberRosterElement {
    scaffold: subscribing::Scaffold,
    /// The live member set, keyed by each row's entity `this` so an `update`
    /// delta can upsert/retract individual rows rather than needing a full
    /// snapshot every time. Order is insertion order.
    members: Rc<RefCell<Vec<(String, String)>>>,
}

impl CustomElement for UiMemberRosterElement {
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: the directory-mode
/// roster query, and rendering delivered frames as member spans.
struct MemberRosterBehaviour {
    members: Rc<RefCell<Vec<(String, String)>>>,
}

impl subscribing::Subscribing for MemberRosterBehaviour {
    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        // Directory mode binds no subject — the query reads every member row
        // on whichever branch `with` (stamped from `space` by the
        // scaffolding's default `resolve_with`) points at.
        Ok(member_roster_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let conclusions = js_sys::Array::from(payload);
        let mut members = self.members.borrow_mut();
        members.clear();
        for i in 0..conclusions.length() {
            if let Some(row) = read_row(&conclusions.get(i)) {
                members.push(row);
            }
        }
        render_spans(host, &members);
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let retracted = Reflect::get(payload, &"retracted".into()).unwrap_or(JsValue::UNDEFINED);
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        let mut members = self.members.borrow_mut();

        let retracted_rows = js_sys::Array::from(&retracted);
        for i in 0..retracted_rows.length() {
            if let Some((id, _)) = read_row(&retracted_rows.get(i)) {
                members.retain(|(existing_id, _)| existing_id != &id);
            }
        }

        let asserted_rows = js_sys::Array::from(&asserted);
        for i in 0..asserted_rows.length() {
            if let Some((id, name)) = read_row(&asserted_rows.get(i)) {
                match members
                    .iter_mut()
                    .find(|(existing_id, _)| existing_id == &id)
                {
                    Some(existing) => existing.1 = name,
                    None => members.push((id, name)),
                }
            }
        }

        render_spans(host, &members);
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

/// Read `(row.this, row.fields.name)` off a raw subscription row. `None` for
/// a missing/empty row, a missing entity id, or a missing/non-string name —
/// mirroring the query's requirement that all three fields (and so the row's
/// `this`) are present for a row to appear at all.
fn read_row(row: &JsValue) -> Option<(String, String)> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let name = Reflect::get(row, &"fields".into())
        .ok()
        .and_then(|fields| Reflect::get(&fields, &"name".into()).ok())
        .and_then(|v| v.as_string())?;
    Some((this_id, name))
}

/// Rebuild the host's children as one member span per row, in `members`'
/// order — the markup the deleted `fab-roster` view used to supply.
fn render_spans(host: &HtmlElement, members: &[(String, String)]) {
    while let Some(child) = host.first_child() {
        let _ = host.remove_child(&child);
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    for (_, name) in members {
        let Ok(span) = document.create_element("span") else {
            continue;
        };
        let _ = span.set_attribute("class", "fab__menu-item fab__menu-item--member");
        span.set_text_content(Some(name));
        let _ = host.append_child(&span);
    }
}

/// Register `<ui-member-roster>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiMemberRosterElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
