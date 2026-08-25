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
use js_sys::{Function, Object, Reflect};
use tonk_common::log;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, HtmlElement, window};

use crate::logic::member_roster_query_body;
use crate::subscribing;

const SUB_TAG: &str = "ui-member-roster";

#[derive(Default)]
pub struct UiMemberRosterElement {
    scaffold: subscribing::Scaffold,
    /// The live member set, keyed by each row's entity `this` so an `update`
    /// delta can upsert/retract individual rows rather than needing a full
    /// snapshot every time. Order is insertion order.
    members: Rc<RefCell<Vec<Member>>>,
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
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// One roster row: the membership entity, the name shown, the DID the
/// membership is keyed on, and the role stamped on it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Member {
    this: String,
    name: String,
    did: String,
    role: String,
}

/// This element's [`subscribing::Subscribing`] behaviour: the directory-mode
/// roster query, and rendering delivered frames as member spans.
struct MemberRosterBehaviour {
    members: Rc<RefCell<Vec<Member>>>,
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
fn read_row(row: &JsValue) -> Option<Member> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let field = |name: &str| {
        Reflect::get(&fields, &name.into())
            .ok()
            .and_then(|value| value.as_string())
    };
    Some(Member {
        this: this_id,
        name: field("name")?,
        did: field("member")?,
        role: field("role").unwrap_or_default(),
    })
}

/// Rebuild the host's children as one member span per row, in `members`'
/// order — the markup the deleted `fab-roster` view used to supply. A
/// member who is not already an admin or the founder gets a "Make admin"
/// button; the worker refuses the rest, but there is no point offering
/// what it will refuse.
fn render_spans(host: &HtmlElement, members: &[Member]) {
    while let Some(child) = host.first_child() {
        let _ = host.remove_child(&child);
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let space = host.get_attribute("space").unwrap_or_default();
    for member in members {
        let Ok(span) = document.create_element("span") else {
            continue;
        };
        let _ = span.set_attribute("class", "fab__menu-item fab__menu-item--member");
        let _ = span.set_attribute("data-role", &member.role);
        span.set_text_content(Some(&member.name));
        if !space.is_empty() && member.role != "tonk:founder" && member.role != "tonk:admin" {
            if let Ok(button) = document.create_element("button") {
                let _ = button.set_attribute("type", "button");
                let _ = button.set_attribute("class", "fab__member-promote");
                let _ =
                    button.set_attribute("aria-label", &format!("Make {} an admin", member.name));
                button.set_text_content(Some("Make admin"));
                install_promote(&button, space.clone(), member.did.clone());
                let _ = span.append_child(&button);
            }
        }
        let _ = host.append_child(&span);
    }
}

/// Wire a "Make admin" button: on click, ask the outer page to delegate
/// `/` on the space to the member's account (the passkey ceremony runs
/// there, inside this click's activation), then dispatch `member/promote`
/// carrying the hop it minted. Errors are logged; the roster shows the
/// outcome when the role row arrives through the subscription.
fn install_promote(button: &Element, space: String, member: String) {
    let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
        let space = space.clone();
        let member = member.clone();
        spawn_local(async move {
            match delegate(&space, "/", &member).await {
                Ok(chain) => transact(&crate::logic::promote_claim_json(&space, &member, &chain)),
                Err(error) => log!("member/promote: the page did not delegate: {error:?}"),
            }
        });
    });
    let _ = button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref());
    on_click.forget();
}

/// `window.tonk.delegate({subject, command, audience})`: the outer page
/// mints `root -> audience` under the passkey and answers with the base58
/// chain.
async fn delegate(subject: &str, command: &str, audience: &str) -> Result<String, JsValue> {
    let win = window().ok_or_else(|| JsValue::from_str("no window"))?;
    let tonk = Reflect::get(&win, &"tonk".into())?
        .dyn_into::<Object>()
        .map_err(|_| JsValue::from_str("no window.tonk"))?;
    let delegate = Reflect::get(&tonk, &"delegate".into())?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("window.tonk.delegate is missing"))?;
    let request = Object::new();
    Reflect::set(&request, &"subject".into(), &JsValue::from_str(subject))?;
    Reflect::set(&request, &"command".into(), &JsValue::from_str(command))?;
    Reflect::set(&request, &"audience".into(), &JsValue::from_str(audience))?;
    let promise: js_sys::Promise = delegate.call1(&tonk, &request)?.dyn_into()?;
    let answer = JsFuture::from(promise).await?;
    answer
        .as_string()
        .ok_or_else(|| JsValue::from_str("the page answered without a chain"))
}

/// Dispatch a `TransactRequest` JSON body via `window.tonk.transact(...)`,
/// routeless, so it lands on the FAB portal's own context where the
/// command lives.
fn transact(claim: &serde_json::Value) {
    let Ok(json) = serde_json::to_string(claim) else {
        return;
    };
    let Some(win) = window() else { return };
    let Some(tonk) = Reflect::get(&win, &"tonk".into())
        .ok()
        .and_then(|v| v.dyn_into::<Object>().ok())
    else {
        return;
    };
    let Some(transact) = Reflect::get(&tonk, &"transact".into())
        .ok()
        .and_then(|v| v.dyn_into::<Function>().ok())
    else {
        return;
    };
    if let Ok(request) = js_sys::JSON::parse(&json) {
        let _ = transact.call1(&tonk, &request);
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
