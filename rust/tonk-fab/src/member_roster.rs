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
//! Renders one sibling `<tonk-mi>` row per member. Members whose role can
//! manage the roster get a `make admin` action on non-admin rows; everyone
//! else sees the roster as muted metadata.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use tonk_common::log;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{HtmlElement, window};

use crate::logic::{
    member_roster_query_body, role_manages_members, self_did_from_conclusions, self_did_query_body,
};
use crate::stack_rows;
use crate::subscribing;

const SUB_TAG: &str = "ui-member-roster";

#[derive(Default)]
pub struct UiMemberRosterElement {
    scaffold: subscribing::Scaffold,
    /// The live member set, keyed by each row's entity `this` so an `update`
    /// delta can upsert/retract individual rows rather than needing a full
    /// snapshot every time. Order is insertion order.
    members: Rc<RefCell<Vec<Member>>>,
    /// The signed-in member's profile DID. Their roster role decides whether
    /// promotion actions are offered.
    viewer: Rc<RefCell<Option<String>>>,
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
            viewer: self.viewer.clone(),
        });
        self.scaffold.connect(this, behaviour);
        if self.viewer.borrow().is_none() {
            resolve_viewer(this, self.members.clone(), self.viewer.clone());
        }
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "space" || old == new {
            return;
        }
        // The space landed (or moved): the roster subscription was opened
        // against the old value — or skipped entirely while it was blank.
        // Drop it and subscribe against the space that is actually here.
        self.scaffold.disconnect();
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// One roster row and the role-bearing membership it represents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    this: String,
    name: String,
    did: String,
    role: String,
}

/// This element's [`subscribing::Subscribing`] behaviour: the directory-mode
/// roster query, and rendering delivered frames as member rows.
struct MemberRosterBehaviour {
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
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
        render_rows(host, &members, self.viewer.borrow().as_deref());
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let retracted = Reflect::get(payload, &"retracted".into()).unwrap_or(JsValue::UNDEFINED);
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        let mut members = self.members.borrow_mut();

        let retracted_rows = js_sys::Array::from(&retracted);
        for i in 0..retracted_rows.length() {
            if let Some(row) = read_row(&retracted_rows.get(i)) {
                members.retain(|existing| existing.this != row.this);
            }
        }

        let asserted_rows = js_sys::Array::from(&asserted);
        for i in 0..asserted_rows.length() {
            if let Some(row) = read_row(&asserted_rows.get(i)) {
                match members
                    .iter_mut()
                    .find(|existing| existing.this == row.this)
                {
                    Some(existing) => *existing = row,
                    None => members.push(row),
                }
            }
        }

        render_rows(host, &members, self.viewer.borrow().as_deref());
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

/// Read a member off a raw subscription row. `None` for a missing/empty row,
/// a missing entity id, or any missing required string field.
fn read_row(row: &JsValue) -> Option<Member> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let field = |name: &str| {
        Reflect::get(&fields, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.as_string())
    };
    Some(Member {
        this: this_id,
        name: field("name")?,
        did: field("member")?,
        role: field("role")?,
    })
}

/// Rebuild the roster as one row per member, in `members`' order.
///
/// Rows are SIBLINGS in the share stack, not children of this element — see
/// [`crate::stack_rows`] for why. Rows are muted metadata unless the viewer's
/// role permits promoting that member.
fn render_rows(host: &HtmlElement, members: &[Member], viewer: Option<&str>) {
    stack_rows::clear_rows(host, SUB_TAG);
    let space = host.get_attribute("space").unwrap_or_default();
    let viewer_manages = viewer
        .and_then(|did| members.iter().find(|member| member.did == did))
        .is_some_and(|member| role_manages_members(&member.role));

    for member in members {
        let Some(row) = stack_rows::new_row(SUB_TAG) else {
            continue;
        };
        // A member's name is a user word — no `chrome`, no lowercasing.
        row.set_text_content(Some(&member.name));
        let _ = row.set_attribute("data-role", &member.role);

        if viewer_manages && !space.is_empty() && !role_manages_members(&member.role) {
            let _ = row.set_attribute("data-member-promote", &member.did);
            let _ = row.set_attribute("data-promote-space", &space);
            if let Some(document) = window().and_then(|window| window.document())
                && let Ok(action) = document.create_element("span")
            {
                action.set_class_name("sub");
                action.set_text_content(Some("make admin"));
                let _ = row.append_child(&action);
            }
        } else {
            let _ = row.set_attribute("muted", "");
        }
        stack_rows::insert_row(host, &row);
    }
}

/// Resolve the signed-in profile DID once, then repaint any roster rows that
/// arrived while the profile query was in flight.
fn resolve_viewer(
    host: &HtmlElement,
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
) {
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|value| value.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(query) = Reflect::get(&tonk, &"query".into())
        .ok()
        .and_then(|value| value.dyn_into::<Function>().ok())
    else {
        return;
    };
    let Ok(body) = js_sys::JSON::parse(&self_did_query_body()) else {
        return;
    };
    let Ok(result) = query.call1(&tonk, &body) else {
        return;
    };
    let Ok(promise) = result.dyn_into::<js_sys::Promise>() else {
        return;
    };

    let host = host.clone();
    spawn_local(async move {
        let rows = match JsFuture::from(promise).await {
            Ok(rows) => rows,
            Err(error) => {
                log!("ui-member-roster profile query failed: {error:?}");
                return;
            }
        };
        let Some(json) = js_sys::JSON::stringify(&rows)
            .ok()
            .and_then(|json| json.as_string())
        else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
            return;
        };
        let Some(did) = self_did_from_conclusions(&value) else {
            return;
        };
        *viewer.borrow_mut() = Some(did);
        if host.is_connected() {
            render_rows(&host, &members.borrow(), viewer.borrow().as_deref());
        }
    });
}

/// Register `<ui-member-roster>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiMemberRosterElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
