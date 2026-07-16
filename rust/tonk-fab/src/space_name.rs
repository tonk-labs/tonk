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
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, Event, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

use crate::logic::{rename_repo_claim_json, repo_name_query_body};
use crate::retry::RetryPolicy;

/// Shown before the first frame and for a repo with no name — matches the
/// existing "Untitled" fallback the seeded view rendered.
const UNTITLED: &str = "Untitled";

const SUB_TAG: &str = "ui-space-name";

/// The per-instance frame-delegate closure shape: `(payload, opts)`, matching
/// what the host's `invoke_method_marked` calls with.
type FrameClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// The editable child's `change`-commit listener closure.
type ChangeClosure = Closure<dyn FnMut(Event)>;

#[derive(Default)]
pub struct UiSpaceNameElement {
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
    /// The last name the live subscription actually delivered — the value a
    /// no-op or failed rename reverts the chip to. Seeded to `UNTITLED` in
    /// `inject_children` so a commit issued before the first frame arrives
    /// reverts to the same placeholder the chip shows, rather than the
    /// zero-value empty string a bare `Default` would leave it at.
    current_name: Rc<RefCell<String>>,
    reset: Rc<RefCell<Option<FrameClosure>>>,
    update: Rc<RefCell<Option<FrameClosure>>>,
    change: Rc<RefCell<Option<ChangeClosure>>>,
}

impl CustomElement for UiSpaceNameElement {
    fn inject_children(&mut self, this: &HtmlElement) {
        *self.current_name.borrow_mut() = UNTITLED.to_owned();
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let Ok(editable) = document.create_element("tonk-editable") else {
            return;
        };
        editable.set_text_content(Some(UNTITLED));
        let _ = editable.set_attribute("data-rename", "tonk:repository");
        let _ = this.append_child(&editable);
    }

    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let Some(space) = this.get_attribute("space").filter(|s| !s.is_empty()) else {
            // No space yet (an unsubstituted `{id}` placeholder, say) — the
            // attribute callback re-runs this when it lands.
            return;
        };
        // Stamp our own routing context: `resolve_with` reads THIS element's
        // attribute and never walks ancestors.
        let _ = this.set_attribute("with", &crate::logic::space_with(&space));

        // Install the per-instance `reset` delegate: the host calls
        // `element.reset(conclusions, { tag })` for the first (and any
        // reconnect) frame, and the prototype shim installed in `register`
        // forwards it here.
        let current_name = self.current_name.clone();
        let host = this.clone();
        let reset: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                on_frame(&host, payload, &current_name);
            }));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        *self.reset.borrow_mut() = Some(reset);

        // Install the per-instance `update` delegate: subsequent name changes
        // arrive as an incremental delta, not another snapshot.
        let current_name = self.current_name.clone();
        let host = this.clone();
        let update: FrameClosure =
            Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                on_delta(&host, payload, &current_name);
            }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        *self.update.borrow_mut() = Some(update);

        // Attach the commit listener to the `<tonk-editable>` child. There is
        // no `tonk-display` event delegation here (this markup is Rust-owned,
        // not a resolved template), so the `change` binding is wired directly.
        if let Some(editable) = this.query_selector("tonk-editable").ok().flatten() {
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

        let subscription = self.subscription.clone();
        let retry = self.retry.clone();
        let host = this.clone();
        spawn_local(async move {
            if !host.is_connected() || subscription.borrow().is_some() {
                return;
            }
            subscribe_name(&host, &space, subscription, retry);
        });
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        self.subscription.borrow_mut().take();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
        self.change.borrow_mut().take();
    }
}

fn subscribe_name(
    host: &HtmlElement,
    space: &str,
    subscription: Rc<RefCell<Option<Subscription>>>,
    retry: Rc<RefCell<RetryPolicy>>,
) {
    let body = match repo_name_query_body(space) {
        Ok(body) => body,
        Err(err) => {
            tonk_common::log!("ui-space-name: query build failed: {err}");
            return;
        }
    };
    let Ok(parsed) = JSON::parse(&body) else {
        tonk_common::log!("ui-space-name: query JSON parse failed");
        return;
    };
    let consumer_el: Element = host.clone().into();
    let tag = JsValue::from_str(SUB_TAG);
    match consumer::subscribe(&consumer_el, &parsed, Some(&tag)) {
        Ok(sub) => {
            retry.borrow_mut().reset();
            *subscription.borrow_mut() = Some(sub);
        }
        Err(err) => {
            // Bounded, unlike the host's default resubscribe loop.
            let delay = retry.borrow_mut().next_delay_ms();
            match delay {
                Some(_) => {
                    tonk_common::log!("ui-space-name: subscribe failed, will retry: {err:?}")
                }
                None => {
                    tonk_common::log!("ui-space-name: subscribe failed, giving up: {err:?}");
                    let _ = host.set_attribute("data-state", "unavailable");
                }
            }
        }
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
    let registered = window()
        .map(|win| !win.custom_elements().get("ui-space-name").is_undefined())
        .unwrap_or(false);
    if registered {
        return;
    }
    UiSpaceNameElement::define("ui-space-name");
    install_frame_shims();
}

fn install_frame_shims() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("ui-space-name");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let update_fn = Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
}
