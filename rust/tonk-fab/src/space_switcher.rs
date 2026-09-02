//! `<ui-space-switcher>` — the space rows of the bar's `open ▸` flyout: one
//! `<tonk-mi>` per space the profile knows about.
//!
//! Built on the shared `subscribing` scaffolding, like `<ui-space-name>` and
//! `<ui-member-roster>` — but it is the odd one out: it reads the PROFILE
//! branch, not a space, so its routing context is the fixed literal
//! `main@profile:tonk` rather than a space-derived `main@{did}`. It overrides
//! [`subscribing::Subscribing::resolve_with`] rather than relying on the
//! scaffolding's `space`-attribute default, proving that seam accepts either
//! shape.
//!
//! Reads `xyz.tonk.space/{subject,name,status}` through ONE inline
//! directory-mode predicate (`this` unbound, so every account-level directory
//! entry returns as a row) — see [`crate::logic::space_list_query_body`]. The
//! mirrored `name` is available even when this device has not replicated the
//! target space, and the rename command keeps it current.
//!
//! Account directory entries contain real spaces only. The active space
//! (this element's `current` attribute) is shown like the wireframe shows
//! it — marked `current`, always making the cut — and picking it merely
//! closes the stack (see `element.rs`): where you are is a fact, not a
//! navigation. A row stamps `data-status` from the directory status so
//! existing CSS can dim a still-seeding space.
//!
//! It renders ONLY the space rows. `new +` and `more ↖` belong to the stack
//! that hosts this flyout (`markup::STACKS_HTML`), not here — emitting them
//! in both places put a duplicate, unstyled pair inside the open menu.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use web_sys::HtmlElement;

use crate::logic::{reset_keyed_rows, space_list_query_body};
use crate::stack_rows;
use crate::subscribing;

const SUB_TAG: &str = "ui-space-switcher";

/// The PROFILE branch's own routing context — fixed, not derived from any
/// attribute on this element.
const PROFILE_WITH: &str = "main@profile:tonk";

/// How many spaces fly out before `more ↖` takes over. A stack is a glance,
/// not a directory; past this the Hub is the better answer.
const MAX_ROWS: usize = 7;

/// Vintage directory rows can predate the account-level name mirror.
const UNTITLED: &str = "Untitled";

/// One account-directory row: the fields the switcher needs to render it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    subject: String,
    name: Option<String>,
    status: String,
}

#[derive(Default)]
pub struct UiSpaceSwitcherElement {
    scaffold: subscribing::Scaffold,
    /// The live directory, keyed by each row's entity `this`, so an `update`
    /// delta can upsert/retract individual rows without a full snapshot.
    /// Order is insertion order.
    rows: Rc<RefCell<Vec<(String, Row)>>>,
}

impl CustomElement for UiSpaceSwitcherElement {
    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["current"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(SpaceSwitcherBehaviour {
            rows: self.rows.clone(),
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
        if name != "current" || old == new {
            return;
        }
        // The profile subscription can settle before the outer bar's routed
        // `space` attribute. Re-filter the rows already delivered when that
        // active subject lands; the profile query itself has not changed and
        // must stay open.
        render_menu(this, &self.rows.borrow());
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: the fixed PROFILE
/// routing context, the account-directory query, and rendering
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
        let mut delivered = Vec::new();
        for i in 0..conclusions.length() {
            if let Some(row) = read_row(&conclusions.get(i)) {
                delivered.push(row);
            }
        }
        reset_keyed_rows(&mut rows, delivered);
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

/// Read `(row.this, Row { subject, name?, status })` off a raw subscription
/// row. `None` for a missing/empty row, a missing entity id, or a missing
/// required field. `name` is optional so pre-mirror entries remain reachable.
fn read_row(row: &JsValue) -> Option<(String, Row)> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let this_id = Reflect::get(row, &"this".into()).ok()?.as_string()?;
    let fields = Reflect::get(row, &"fields".into()).ok()?;
    let subject = Reflect::get(&fields, &"subject".into())
        .ok()
        .and_then(|v| v.as_string())?;
    let name = Reflect::get(&fields, &"name".into())
        .ok()
        .and_then(|v| v.as_string());
    let status = Reflect::get(&fields, &"status".into())
        .ok()
        .and_then(|v| v.as_string())?;
    Some((
        this_id,
        Row {
            subject,
            name,
            status,
        },
    ))
}

/// Rebuild the host's children: one `<tonk-mi>` per surviving space, named
/// from the account directory so an unreplicated target is still legible.
///
/// No action rows. The stack that hosts this flyout already carries `new +`
/// and `more ↖` (see `markup::STACKS_HTML`); emitting them here as well is
/// what put a second, unstyled "+new" and "all spaces" inside the open menu.
///
/// The list is capped at [`MAX_ROWS`]: up to seven spaces fly out, and `more
/// ↖` — the stack's own last row — is the way to the rest. The current space
/// always makes the cut, trading the seventh slot for it when it would fall
/// past the cap.
fn render_menu(host: &HtmlElement, rows: &[(String, Row)]) {
    stack_rows::clear_rows(host, SUB_TAG);
    let current = host.get_attribute("current").unwrap_or_default();

    let mut cut: Vec<&(String, Row)> = rows.iter().take(MAX_ROWS).collect();
    if !current.is_empty()
        && !cut.iter().any(|(_, row)| row.subject == current)
        && let Some(active) = rows.iter().find(|(_, row)| row.subject == current)
    {
        cut.pop();
        cut.push(active);
    }
    for (_, row) in cut {
        let Some(item) = stack_rows::new_row(SUB_TAG) else {
            continue;
        };
        // The space is a user word — it passes through untouched, so this row
        // is deliberately NOT `chrome`.
        let _ = item.set_attribute("data-space", &row.subject);
        let _ = item.set_attribute("data-status", &row.status);
        if row.subject == current {
            let _ = item.set_attribute("current", "");
        }
        item.set_text_content(Some(row.name.as_deref().unwrap_or(UNTITLED)));
        stack_rows::insert_row(host, &item);
    }
}

/// Register `<ui-space-switcher>`. Idempotent.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiSpaceSwitcherElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
