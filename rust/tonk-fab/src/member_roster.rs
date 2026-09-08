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
//! Renders a live count in the share stack and names in a scrollable members dialog.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, Object, Reflect};
use tonk_common::log;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{HtmlElement, window};

use crate::logic::{member_roster_query_body, self_did_from_conclusions, self_did_query_body};
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
    /// The signed-in profile DID, used to mark their row as "you".
    viewer: Rc<RefCell<Option<String>>>,
    dialog: Option<HtmlElement>,
    listeners: Vec<crate::shadow::Bound>,
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
        let Some(document) = window().and_then(|window| window.document()) else {
            return;
        };
        let Some(body) = document.body() else { return };
        let Ok(dialog) = document.create_element("tonk-dialog") else {
            return;
        };
        let dialog: HtmlElement = dialog.unchecked_into();
        let _ = dialog.set_attribute("heading", "members");
        dialog.set_class_name("fabb-members");
        let _ = body.append_child(&dialog);
        let popup = dialog.clone();
        self.listeners
            .push(crate::shadow::bind(this, "fabb-show-members", move |_| {
                crate::dialog::show_dialog(&popup);
            }));
        self.dialog = Some(dialog.clone());
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
            dialog: dialog.clone(),
        });
        render_rows(
            this,
            &self.members.borrow(),
            self.viewer.borrow().as_deref(),
            &dialog,
        );
        self.scaffold.connect(this, behaviour);
        if self.viewer.borrow().is_none() {
            resolve_viewer(this, self.members.clone(), self.viewer.clone(), dialog);
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
        self.members.borrow_mut().clear();
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        crate::dialog::close_dialog(dialog);
        render_rows(this, &[], self.viewer.borrow().as_deref(), dialog);
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
            dialog: dialog.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        self.scaffold.disconnect();
        self.listeners.clear();
        stack_rows::clear_rows(this, SUB_TAG);
        if let Some(dialog) = self.dialog.take() {
            dialog.remove();
        }
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
    dialog: HtmlElement,
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
        render_rows(
            host,
            &members,
            self.viewer.borrow().as_deref(),
            &self.dialog,
        );
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

        render_rows(
            host,
            &members,
            self.viewer.borrow().as_deref(),
            &self.dialog,
        );
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

/// Keep the stack compact, with the full roster in the modal body.
fn render_rows(host: &HtmlElement, members: &[Member], viewer: Option<&str>, dialog: &HtmlElement) {
    stack_rows::clear_rows(host, SUB_TAG);
    if let Some(row) = stack_rows::new_row(SUB_TAG) {
        let _ = row.set_attribute("data-share-members", "");
        let _ = row.set_attribute("muted", "");
        let _ = row.set_attribute("aria-haspopup", "dialog");
        row.set_text_content(Some(&format!(
            "{} {}",
            members.len(),
            if members.len() == 1 {
                "member"
            } else {
                "members"
            }
        )));
        stack_rows::insert_row(host, &row);
    }
    dialog.set_text_content(None);
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    for member in members {
        let Ok(row) = document.create_element("div") else {
            continue;
        };
        row.set_class_name("mem-row");
        let Ok(name) = document.create_element("span") else {
            continue;
        };
        name.set_text_content(Some(&member.name));
        let is_self = viewer == Some(member.did.as_str());
        if is_self {
            name.set_class_name("mem-self");
        }
        let _ = row.append_child(&name);
        let role = match member.role.as_str() {
            "tonk:founder" => "owner",
            "tonk:admin" => "admin",
            _ => "",
        };
        let label = match (is_self, role.is_empty()) {
            (true, false) => format!("you, {role}"),
            (true, true) => "you".to_owned(),
            (false, _) => role.to_owned(),
        };
        if !label.is_empty()
            && let Ok(tag) = document.create_element("span")
        {
            tag.set_class_name("mem-you");
            tag.set_text_content(Some(&label));
            let _ = row.append_child(&tag);
        }
        let _ = dialog.append_child(&row);
    }
}

/// Resolve the signed-in profile DID once, then repaint any roster rows that
/// arrived while the profile query was in flight.
fn resolve_viewer(
    host: &HtmlElement,
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
    dialog: HtmlElement,
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
        if host.is_connected() && dialog.is_connected() {
            render_rows(
                &host,
                &members.borrow(),
                viewer.borrow().as_deref(),
                &dialog,
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscribing::Subscribing;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn roster_updates_count_names_and_viewer_labels() {
        let document = window().unwrap().document().unwrap();
        let menu = document.create_element("tonk-menu").unwrap();
        let host: HtmlElement = document.create_element("div").unwrap().unchecked_into();
        menu.append_child(&host).unwrap();
        let dialog: HtmlElement = document.create_element("div").unwrap().unchecked_into();
        let behaviour = MemberRosterBehaviour {
            dialog: dialog.clone(),
            members: Rc::default(),
            viewer: Rc::new(RefCell::new(Some("did:key:owner".into()))),
        };
        let owner = serde_json::json!({ "this": "owner-membership", "fields": {
            "name": "<Owner>", "member": "did:key:owner", "role": "tonk:founder"
        }});
        let member = serde_json::json!({ "this": "member-membership", "fields": {
            "name": "Member", "member": "did:key:member", "role": "tonk:member"
        }});
        let js = |value: serde_json::Value| js_sys::JSON::parse(&value.to_string()).unwrap();
        behaviour.render_reset(&host, &js(serde_json::json!([owner])));
        assert_eq!(
            menu.query_selector("[data-share-members]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("1 member")
        );
        assert_eq!(
            dialog
                .query_selector(".mem-self")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("<Owner>")
        );
        assert_eq!(
            dialog
                .query_selector(".mem-you")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("you, owner")
        );
        assert!(
            dialog.query_selector("owner").unwrap().is_none(),
            "names remain plain text"
        );
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [member], "retracted": [] })),
        );
        assert_eq!(
            menu.query_selector("[data-share-members]")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("2 members")
        );
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [], "retracted": [owner] })),
        );
        assert_eq!(dialog.query_selector_all(".mem-row").unwrap().length(), 1);
        assert_eq!(dialog.text_content().as_deref(), Some("Member"));
    }
}
