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
//! Renders names in the FABB's attached members panel.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlElement, window};

use crate::logic::{member_roster_query_body, self_member_query_body};
use crate::subscribing;

const SUB_TAG: &str = "ui-member-roster";
const VIEWER_TAG: &str = "ui-member-roster-viewer";

#[derive(Default)]
pub struct UiMemberRosterElement {
    scaffold: subscribing::Scaffold,
    /// The live member set, keyed by each row's entity `this` so an `update`
    /// delta can upsert/retract individual rows rather than needing a full
    /// snapshot every time. Order is insertion order.
    members: Rc<RefCell<Vec<Member>>>,
    /// The current membership DID, used to mark its row as "you".
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
        render_rows(
            this,
            &self.members.borrow(),
            self.viewer.borrow().as_deref(),
        );
        self.connect(this);
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
        self.viewer.borrow_mut().take();
        render_rows(this, &[], None);
        self.connect(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
        if let Some(panel) = member_panel(_this) {
            panel.set_text_content(None);
        }
    }
}

impl UiMemberRosterElement {
    /// Subscribe to the roster and to which member this device acts as, on
    /// the space `with` points at. The worker keeps the latter current, so
    /// a linked or forgotten account re-marks "you" without a refetch.
    fn connect(&self, this: &HtmlElement) {
        let roster: Rc<dyn subscribing::Subscribing> = Rc::new(MemberRosterBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
        });
        let viewer: Rc<dyn subscribing::Subscribing> = Rc::new(ViewerBehaviour {
            members: self.members.clone(),
            viewer: self.viewer.clone(),
        });
        self.scaffold.connect_all(this, vec![roster, viewer]);
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

/// Keep the stack compact, with the full roster in the modal body.
fn member_panel(host: &HtmlElement) -> Option<HtmlElement> {
    host.closest("tonk-fab")
        .ok()
        .flatten()
        .and_then(|bar| bar.shadow_root())
        .and_then(|root| root.query_selector(".members-list").ok().flatten())
        .and_then(|panel| panel.dyn_into::<HtmlElement>().ok())
}

fn render_rows(host: &HtmlElement, members: &[Member], viewer: Option<&str>) {
    let Some(panel) = member_panel(host) else {
        return;
    };
    panel.set_text_content(None);
    let Some(document) = window().and_then(|window| window.document()) else {
        return;
    };
    if members.is_empty() {
        let Ok(empty) = document.create_element("p") else {
            return;
        };
        empty.set_class_name("members-empty");
        empty.set_text_content(Some("no members are available"));
        let _ = panel.append_child(&empty);
        return;
    }
    for member in members {
        let Ok(row) = document.create_element("div") else {
            continue;
        };
        row.set_class_name("mem-row");
        let _ = row.set_attribute("role", "listitem");
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
        for (label, class) in [
            (is_self.then_some("you"), "mem-you"),
            ((!role.is_empty()).then_some(role), "mem-role"),
        ] {
            let Some(label) = label else { continue };
            let Ok(tag) = document.create_element("span") else {
                continue;
            };
            tag.set_class_name(&format!("mem-tag {class}"));
            tag.set_text_content(Some(label));
            let _ = row.append_child(&tag);
        }
        let _ = panel.append_child(&row);
    }
}

/// The `state:self-member` subscription: which member row is "you".
struct ViewerBehaviour {
    members: Rc<RefCell<Vec<Member>>>,
    viewer: Rc<RefCell<Option<String>>>,
}

impl ViewerBehaviour {
    fn apply(&self, host: &HtmlElement, rows: &JsValue) {
        let rows = js_sys::Array::from(rows);
        let member = (0..rows.length()).find_map(|i| {
            let fields = Reflect::get(&rows.get(i), &"fields".into()).ok()?;
            Reflect::get(&fields, &"member".into()).ok()?.as_string()
        });
        if let Some(member) = member {
            *self.viewer.borrow_mut() = Some(member);
            render_rows(
                host,
                &self.members.borrow(),
                self.viewer.borrow().as_deref(),
            );
        }
    }
}

impl subscribing::Subscribing for ViewerBehaviour {
    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        Ok(self_member_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        self.apply(host, payload);
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
        self.apply(host, &asserted);
    }

    fn tag(&self) -> &'static str {
        VIEWER_TAG
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscribing::Subscribing;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn roster_updates_names_and_viewer_labels() {
        crate::register();
        let document = window().unwrap().document().unwrap();
        let bar: HtmlElement = document
            .create_element("tonk-fab")
            .unwrap()
            .unchecked_into();
        bar.set_attribute("space", "did:key:members").unwrap();
        document.body().unwrap().append_child(&bar).unwrap();
        let host: HtmlElement = bar
            .query_selector("ui-member-roster")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let panel: HtmlElement = bar
            .shadow_root()
            .unwrap()
            .query_selector(".members-list")
            .unwrap()
            .unwrap()
            .unchecked_into();
        let behaviour = MemberRosterBehaviour {
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
            bar.shadow_root()
                .unwrap()
                .query_selector(".members span")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("view members")
        );
        assert_eq!(
            panel
                .query_selector(".mem-self")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("<Owner>")
        );
        assert_eq!(
            panel
                .query_selector(".mem-you")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("you")
        );
        assert_eq!(
            panel
                .query_selector(".mem-role")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("owner")
        );
        assert!(
            panel.query_selector("owner").unwrap().is_none(),
            "names remain plain text"
        );
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [member], "retracted": [] })),
        );
        assert_eq!(
            bar.shadow_root()
                .unwrap()
                .query_selector(".members span")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("view members")
        );
        *behaviour.viewer.borrow_mut() = Some("did:key:member".into());
        render_rows(
            &host,
            &behaviour.members.borrow(),
            behaviour.viewer.borrow().as_deref(),
        );
        assert_eq!(
            panel
                .query_selector(".mem-self")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Member")
        );
        assert_eq!(
            panel
                .query_selector(".mem-you")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("you")
        );
        behaviour.render_update(
            &host,
            &js(serde_json::json!({ "asserted": [], "retracted": [owner] })),
        );
        assert_eq!(panel.query_selector_all(".mem-row").unwrap().length(), 1);
        assert_eq!(
            panel
                .query_selector(".mem-self")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("Member")
        );
        assert_eq!(
            panel
                .query_selector(".mem-you")
                .unwrap()
                .unwrap()
                .text_content()
                .as_deref(),
            Some("you")
        );
        bar.remove();
    }
}
