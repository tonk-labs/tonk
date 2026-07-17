//! `<ui-profile-name>` — the signed-in member's display name, read live from
//! the PROFILE branch and renamed in place.
//!
//! Built on the shared `subscribing` scaffolding, like `<ui-space-name>` —
//! but it is the odd one out in the same way `<ui-space-switcher>` is: it
//! always reads the PROFILE branch, so its routing context is the fixed
//! literal `main@profile:tonk` rather than a space-derived `main@{did}`. It
//! overrides [`subscribing::Subscribing::resolve_with`] rather than relying
//! on the scaffolding's `space`-attribute default.
//!
//! Reads `xyz.tonk.profile/display-name` through ONE inline directory-mode
//! predicate (`this` unbound — the profile branch carries at most one such
//! row, the member's own override) — see [`crate::logic::profile_name_query_body`].
//! No concept is named, so nothing seeded on the profile branch is consulted
//! and the deleted `tonk:profile/name-view` is never referenced.
//!
//! Renders the same `<tonk-editable class="fab__name-input"
//! data-rename="tonk:profile">` the deleted view used to render — this
//! element only owns reading the live name INTO that chip; committing an
//! edit is still handled by `element.rs::attach_profile_name_commit`, a
//! `change` listener delegated on the whole `<tonk-fab>` host (installed
//! once, before this element's own subscription resolves — see that
//! function's doc for why delegation, not a direct listener here, is
//! required). This element renders a light-DOM `<tonk-editable>` child, so
//! the bubbling `change` event still reaches that delegate unchanged.
//!
//! Absent until the user renames (the worker's `petname` fallback is
//! computed, not persisted — see `tonk-worker::router::profile_name`), so an
//! empty render — no fallback text — is correct: unlike `<ui-space-name>`'s
//! "Untitled", there is no seeded default to fall back to here.

use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::Reflect;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlElement, window};

use crate::logic::profile_name_query_body;
use crate::subscribing;

const SUB_TAG: &str = "ui-profile-name";

/// The PROFILE branch's own routing context — fixed, not derived from any
/// attribute on this element, mirroring `<ui-space-switcher>`.
const PROFILE_WITH: &str = "main@profile:tonk";

#[derive(Default)]
pub struct UiProfileNameElement {
    scaffold: subscribing::Scaffold,
}

/// This element's [`subscribing::Subscribing`] behaviour: the fixed PROFILE
/// routing context, the directory-mode display-name query, and rendering a
/// delivered frame into the chip.
struct ProfileNameBehaviour;

impl subscribing::Subscribing for ProfileNameBehaviour {
    fn resolve_with(&self, _this: &HtmlElement) -> Option<String> {
        // Unlike `<ui-space-name>`, this element's routing context is never
        // derived from an attribute — it always reads the PROFILE branch.
        Some(PROFILE_WITH.to_owned())
    }

    fn query_body(&self, _this: &HtmlElement) -> Result<String, String> {
        // Directory mode binds no subject — nothing seeded on the profile
        // branch is consulted.
        Ok(profile_name_query_body())
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        if let Some(name) = read_name_from_frame(payload) {
            paint(host, &name);
        }
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        if let Some(name) = read_name_from_delta(payload) {
            paint(host, &name);
        }
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl CustomElement for UiProfileNameElement {
    fn inject_children(&mut self, this: &HtmlElement) {
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(editable) = document.create_element("tonk-editable") else {
            return;
        };
        let _ = editable.set_attribute("class", "fab__name-input");
        let _ = editable.set_attribute("data-rename", "tonk:profile");
        let _ = this.append_child(&editable);
    }

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &[]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(ProfileNameBehaviour);
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
    }
}

/// A subscription snapshot frame: read the first conclusion's `name`. `None`
/// (nothing asserted yet — the common case for a never-renamed member)
/// leaves the chip at its current (empty) text rather than writing a
/// fallback.
fn read_name_from_frame(payload: &JsValue) -> Option<String> {
    let conclusions = js_sys::Array::from(payload);
    read_name_field(&conclusions.get(0))
}

/// An incremental `update` frame: `{ asserted, retracted }`. `name` is
/// cardinality-one, so the newest asserted row carries the current value; a
/// bare retract (no asserted) leaves the chip where it is.
fn read_name_from_delta(payload: &JsValue) -> Option<String> {
    let asserted = Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    read_name_field(&rows.get(rows.length().saturating_sub(1)))
}

/// Read `conclusion.fields.name` off a raw subscription row. `None` for a
/// missing/empty row or a non-string value.
fn read_name_field(row: &JsValue) -> Option<String> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    Reflect::get(row, &"fields".into())
        .ok()
        .and_then(|fields| Reflect::get(&fields, &"name".into()).ok())
        .and_then(|v| v.as_string())
}

/// Paint the live name into the chip's `<tonk-editable>` child.
///
/// Skips the DOM write while the field is the active (focused) element — a
/// live frame arriving mid-edit must not clobber in-progress typing,
/// mirroring `<ui-space-name>`'s identical guard.
fn paint(host: &HtmlElement, name: &str) {
    let Some(editable) = host.query_selector("tonk-editable").ok().flatten() else {
        return;
    };
    let editing = window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .map(|active| active.is_same_node(Some(&editable)))
        .unwrap_or(false);
    if editing {
        return;
    }
    editable.set_text_content(Some(name));
}

/// Register `<ui-profile-name>`. Idempotent. Installs the prototype `reset`/
/// `update` method shims (forwarding to the per-instance `__tonkReset`/
/// `__tonkUpdate` delegates) so host subscription frames reach the element —
/// the same pattern every `subscribing`-built element uses.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiProfileNameElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
