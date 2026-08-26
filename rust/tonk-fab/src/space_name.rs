//! `<ui-space-name>` — a space's repository name, read live from its own branch
//! and renamed in place.
//!
//! Host chrome, NOT space content: it renders the same chip regardless of what
//! the space asserts, so a space choosing wild UI can never redefine or break
//! it — unlike a stdlib `tonk:view/*` view, which lives on the space branch and
//! would need per-space seeding. The `ui-` prefix marks it a host UI primitive,
//! distinct from the `tonk-` data elements.
//!
//! Reads `xyz.tonk.repo/name` through an inline predicate (no concept named,
//! nothing seeded) on its own `with="main@{did}"`, exactly as
//! `<ui-sync-status>` reads sync state — and, like that element, the host
//! delivers live frames by calling `reset`/`update` methods on this element
//! directly (see `tonk-host::ops::deliver_frame`), so a prototype shim forwards
//! them to per-instance closures.
//!
//! Writes go through a child `<tonk-editable>` (defined in `tonk-workspace`,
//! registered globally): committing an edit (Enter/blur) fires a `change`
//! event this element listens for directly — there is no `tonk-display`
//! delegate here to resolve a declarative `onchange=` binding, since this
//! markup is Rust-owned. The commit builds an inlined `tonk/rename-repository`
//! claim (mirroring `pause_claim_json`) and dispatches it routeless via
//! `window.tonk.transact`, exactly as the FAB's pause affordance does
//! (`element.rs:dispatch_pause_from_cap`).
//!
//! The chip never optimistically keeps the typed text: on commit it reverts
//! immediately to the last name the live subscription actually delivered, and
//! only shows the new name once (if) a fresh frame confirms the rename really
//! committed. A rename that looks successful but did nothing is the exact bug
//! this whole design exists to kill.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Function, JSON, Object, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{Element, Event, HtmlElement, window};

use crate::logic::{rename_repo_claim_json, repo_name_query_body};
use crate::subscribing;

/// Shown before the first frame and for a repo with no name — matches the
/// existing "Untitled" fallback the seeded view rendered.
const UNTITLED: &str = "Untitled";

const SUB_TAG: &str = "ui-space-name";

/// The class on a `readonly` chip's plain text node.
const READONLY_CLASS: &str = "ui-space-name__text";

/// The editable child's `change`-commit listener closure.
type ChangeClosure = Closure<dyn FnMut(Event)>;

#[derive(Default)]
pub struct UiSpaceNameElement {
    scaffold: subscribing::Scaffold,
    /// The last name the live subscription actually delivered — the value a
    /// no-op or failed rename reverts the chip to. Seeded to `UNTITLED` in
    /// `inject_children` so a commit issued before the first frame arrives
    /// reverts to the same placeholder the chip shows, rather than the
    /// zero-value empty string a bare `Default` would leave it at.
    current_name: Rc<RefCell<String>>,
    change: Rc<RefCell<Option<ChangeClosure>>>,
}

/// This element's [`subscribing::Subscribing`] behaviour: the raw-attribute
/// repo-name query, and rendering a delivered frame into the chip.
struct SpaceNameBehaviour {
    current_name: Rc<RefCell<String>>,
}

impl subscribing::Subscribing for SpaceNameBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        let space = this.get_attribute("space").unwrap_or_default();
        repo_name_query_body(&space)
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        on_frame(host, payload.clone(), &self.current_name);
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        on_delta(host, payload.clone(), &self.current_name);
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl CustomElement for UiSpaceNameElement {
    fn inject_children(&mut self, this: &HtmlElement) {
        *self.current_name.borrow_mut() = UNTITLED.to_owned();
        // Headless: the bar renders the name in its own space cell and
        // renames it with its own block cursor, so this element owns no DOM
        // at all — it is a subscription with an attribute for an output.
        if this.has_attribute("headless") {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        // `readonly`: a name being SHOWN, not offered for editing — a
        // switcher row naming some other space. It gets a plain span, so
        // nothing about it invites a rename that row could not perform
        // anyway (the claim is addressed to the space you are in).
        if this.has_attribute("readonly") {
            if let Ok(text) = document.create_element("span") {
                let _ = text.set_attribute("class", READONLY_CLASS);
                text.set_text_content(Some(UNTITLED));
                let _ = this.append_child(&text);
            }
            return;
        }
        let Ok(editable) = document.create_element("tonk-editable") else {
            return;
        };
        editable.set_text_content(Some(UNTITLED));
        let _ = editable.set_attribute("data-rename-repository", "tonk:repository");
        let _ = this.append_child(&editable);
    }

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.wire(this);
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
        // The space landed (or moved): the name subscription was opened
        // against the old value — or skipped entirely while it was blank.
        // Drop it and wire against the space that is actually here, which is
        // what makes the rename chip come alive on a host whose first
        // projection carried a blank `{id}`.
        self.scaffold.disconnect();
        self.wire(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.scaffold.disconnect();
        self.change.borrow_mut().take();
    }
}

impl UiSpaceNameElement {
    /// Attach the commit listener and open the name subscription — the whole
    /// of connecting, factored so the `space` attribute callback can re-run
    /// it once a late-arriving space lands.
    fn wire(&mut self, this: &HtmlElement) {
        if this
            .get_attribute("space")
            .filter(|s| !s.is_empty())
            .is_none()
        {
            // No space yet (an unsubstituted `{id}` placeholder, say) — the
            // attribute callback re-runs this when it lands.
            return;
        }

        // Headless: the rename arrives as the bar's own `fabb-rename`, fired
        // when its block cursor commits. Listened for on the BAR, not here —
        // this element has no DOM of its own to hear it on.
        if this.has_attribute("readonly") {
            // Nothing to commit — no listener, no claim.
        } else if this.has_attribute("headless") {
            if self.change.borrow().is_none()
                && let Some(bar) = bar_host(this)
            {
                let current_name = self.current_name.clone();
                let host = this.clone();
                let bar_target = bar.clone();
                let on_rename: ChangeClosure = Closure::wrap(Box::new(move |event: Event| {
                    handle_bar_rename(&event, &bar_target, &current_name, &host);
                }));
                let _ = bar.add_event_listener_with_callback(
                    "fabb-rename",
                    on_rename.as_ref().unchecked_ref(),
                );
                *self.change.borrow_mut() = Some(on_rename);
            }
        } else
        // Attach the commit listener to the `<tonk-editable>` child. There is
        // no `tonk-display` event delegation here (this markup is Rust-owned,
        // not a resolved template), so the `change` binding is wired directly.
        // Guarded: a re-wire from the attribute callback must not stack a
        // second listener on the same child.
        if self.change.borrow().is_none()
            && let Some(editable) = this.query_selector("tonk-editable").ok().flatten()
        {
            let claim_target = editable.clone();
            let current_name = self.current_name.clone();
            let host = this.clone();
            let on_change: ChangeClosure = Closure::wrap(Box::new(move |event: Event| {
                handle_commit(&event, &claim_target, &current_name, &host);
            }));
            let _ = editable
                .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
            *self.change.borrow_mut() = Some(on_change);
        }

        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(SpaceNameBehaviour {
            current_name: self.current_name.clone(),
        });
        self.scaffold.connect(this, behaviour);
    }
}

/// A subscription snapshot frame: read the first conclusion's `name` and
/// paint the chip. An empty frame (nothing asserted yet) leaves the chip at
/// its current text rather than clearing it, avoiding a flicker to blank.
fn on_frame(host: &HtmlElement, payload: JsValue, current_name: &Rc<RefCell<String>>) {
    let conclusions = js_sys::Array::from(&payload);
    let first = conclusions.get(0);
    let name = read_name_field(&first);
    if let Some(name) = name {
        paint(host, &name, current_name);
    }
}

/// An incremental `update` frame: `{ asserted, retracted }`. `name` is
/// cardinality-one, so the newest asserted row carries the current value; a
/// bare retract (no asserted) leaves the chip where it is.
fn on_delta(host: &HtmlElement, payload: JsValue, current_name: &Rc<RefCell<String>>) {
    let asserted = Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    let last = rows.get(rows.length().saturating_sub(1));
    let name = read_name_field(&last);
    if let Some(name) = name {
        paint(host, &name, current_name);
    }
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

/// Paint the live name into the chip's `<tonk-editable>` child and remember it
/// as the value a no-op or failed rename reverts to.
///
/// Skips the DOM write while the field is the active (focused) element — a
/// live frame arriving mid-edit must not clobber in-progress typing, mirroring
/// `<tonk-editable>`'s own value-setter guard, which this bypasses by writing
/// `textContent` directly.
fn paint(host: &HtmlElement, name: &str, current_name: &Rc<RefCell<String>>) {
    *current_name.borrow_mut() = name.to_owned();
    if host.has_attribute("headless") {
        // The bar is the renderer: it skips its own repaint while a rename is
        // in progress, so a frame landing mid-edit cannot clobber the typing.
        if let Some(bar) = bar_host(host) {
            let _ = bar.set_attribute("label", name);
        }
        return;
    }
    if host.has_attribute("readonly") {
        if let Some(text) = host
            .query_selector(&format!(".{READONLY_CLASS}"))
            .ok()
            .flatten()
        {
            text.set_text_content(Some(name));
        }
        return;
    }
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

/// Handle the editable's commit (`change`) event.
///
/// Never optimistically keeps the typed text: the chip reverts to the last
/// name the subscription actually delivered immediately, before the claim
/// even dispatches. If the rename commits, the live subscription's next frame
/// (via `on_frame`/`on_delta`) supersedes this with the confirmed name; if it
/// fails (or the field was cleared), the chip is already showing the honest,
/// unchanged prior name — never a phantom success.
fn handle_commit(
    event: &Event,
    editable: &Element,
    current_name: &Rc<RefCell<String>>,
    host: &HtmlElement,
) {
    let typed = event
        .target()
        .and_then(|t| t.dyn_into::<Element>().ok())
        .and_then(|t| t.text_content())
        .unwrap_or_default();
    let previous = current_name.borrow().clone();
    editable.set_text_content(Some(&previous));

    if typed.is_empty() || typed == previous {
        return;
    }
    let Some(space) = host.get_attribute("space").filter(|s| !s.is_empty()) else {
        return;
    };
    dispatch_rename(&space, &typed);
}

/// The bar this subscriber feeds — its own host element.
fn bar_host(this: &HtmlElement) -> Option<HtmlElement> {
    this.closest("tonk-fab")
        .ok()
        .flatten()
        .and_then(|el| el.dyn_into::<HtmlElement>().ok())
}

/// Handle a rename committed by the bar's block cursor.
///
/// Same contract as [`handle_commit`], which is the point: the chip and the
/// bar must not disagree about what a rename means. The name reverts to the
/// last one the subscription actually delivered BEFORE the claim dispatches,
/// so a rename that fails leaves the honest prior name on screen rather than
/// a phantom success; a rename that commits is superseded by the next live
/// frame.
fn handle_bar_rename(
    event: &Event,
    bar: &Element,
    current_name: &Rc<RefCell<String>>,
    host: &HtmlElement,
) {
    let typed = event
        .dyn_ref::<web_sys::CustomEvent>()
        .map(|e| e.detail())
        .and_then(|detail| Reflect::get(&detail, &"value".into()).ok())
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    let previous = current_name.borrow().clone();
    let _ = bar.set_attribute("label", &previous);

    if typed.is_empty() || typed == previous {
        return;
    }
    let Some(space) = host.get_attribute("space").filter(|s| !s.is_empty()) else {
        return;
    };
    dispatch_rename(&space, &typed);
}

/// Build the `tonk/rename-repository` claim and dispatch it via
/// `window.tonk.transact` — routeless, exactly as
/// `element.rs::dispatch_pause_from_cap` dispatches `tonk:pause-sync`. Lands
/// on the FAB's own `main@profile:tonk` context; the worker's handler reads
/// `space` off the command to rename that repository, so nothing space-side
/// is required.
fn dispatch_rename(space: &str, name: &str) {
    let claim = rename_repo_claim_json(space, name);
    let json_str = match serde_json::to_string(&claim) {
        Ok(s) => s,
        Err(_) => return,
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
    if let Ok(obj) = JSON::parse(&json_str) {
        transact.call1(&tonk, &obj).ok();
    }
}

/// Register `<ui-space-name>`. Idempotent. Installs the prototype `reset`/
/// `update` method shims (forwarding to the per-instance `__tonkReset`/
/// `__tonkUpdate` delegates) so host subscription frames reach the element —
/// the same pattern `<ui-sync-status>` uses.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    UiSpaceNameElement::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}
