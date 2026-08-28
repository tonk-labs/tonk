//! `<ui-sync-status>` — a read-only, subscription-driven sync-status disc.
//!
//! Host chrome, NOT space content: it renders the same wireframe "disc"
//! indicator regardless of what a space asserts, so a space choosing wild UI
//! can never redefine or break it (unlike a stdlib `tonk:view/*` view, which
//! lives on the space branch and would need per-space seeding). It is defined
//! in Rust — the `ui-` prefix marks it as a host UI primitive, distinct from
//! the `tonk-` data elements.
//!
//! It SUBSCRIBES to the `tonk:sync` `state:here` status the service worker
//! stamps on the space branch (the same fact the topbar pause chip reads), so
//! it updates live as the background sync drain reconciles — the fix for the
//! old `<tonk-sync-badge>`, which one-shot-fetched on mount + commit + toggle
//! (none of which recur on the Hub, so it froze on a stale reading).
//!
//! Resolves its space from its `with="branch@repo"` attribute and subscribes
//! through that routing context — the same way every consumer reaches the
//! worker. Read-only: it renders the disc but dispatches nothing. The FAB's
//! alt/option-click pause reads this element's `with`/`onpause` and dispatches
//! from its own handler (the cap's `<ui-sync-status>` is cloned, so a listener
//! on it wouldn't reliably fire — see `tonk-fab`). The `.sync` / `.disc` CSS
//! lives in the app stylesheet, so the disc styles wherever this mounts; the
//! caller sizes it with `font-size`.

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{JSON, Reflect};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, window};

use tonk_host::consumer::{self, Subscription};

/// The `data-sync-status` value shown until the first frame lands — the
/// pending disc (matches the topbar chip's honest "syncing…" default, since a
/// status check / sync fires on load).
const INITIAL_STATUS: &str = "sync:pending";

/// The subscription tag. One subscription per element, so a fixed tag is fine
/// (frames are addressed to this element via its own `reset` method).
const SUB_TAG: &str = "ui-sync-status";

/// The reset delegate closure the host calls with each subscription frame.
type ResetClosure = Closure<dyn FnMut(JsValue, JsValue)>;

/// Per-element state: the live subscription (its `Drop` cancels upstream) and
/// the reset delegate closure, kept alive for the element's lifetime.
#[derive(Default)]
pub(crate) struct UiSyncStatus {
    subscription: Rc<RefCell<Option<Subscription>>>,
    reset: Rc<RefCell<Option<ResetClosure>>>,
    update: Rc<RefCell<Option<ResetClosure>>>,
    error: Rc<RefCell<Option<ResetClosure>>>,
}

impl CustomElement for UiSyncStatus {
    fn shadow() -> bool {
        // Light DOM: the disc's CSS lives in the app stylesheet, and the
        // element's `with` context resolves through the light tree.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["with"]
    }

    fn inject_children(&mut self, _this: &HtmlElement) {}

    fn connected_callback(&mut self, this: &HtmlElement) {
        // Render the disc immediately in its pending state so the badge is
        // present before the first frame.
        paint(this, INITIAL_STATUS);

        // Install the per-instance `reset` delegate: the host calls
        // `element.reset(conclusions, { tag })` for each subscription frame,
        // and the prototype shim (installed in `register`) forwards it here.
        let host = this.clone();
        let reset: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                on_frame(&host, payload);
            }));
        let _ = Reflect::set(this, &"__tonkReset".into(), reset.as_ref());
        *self.reset.borrow_mut() = Some(reset);

        // Install the per-instance `update` delegate: with incremental
        // subscriptions the initial frame is a `reset` (snapshot) but every
        // subsequent status change arrives as an `update` (delta). Without
        // this the disc would render the first status and never move —
        // paused/idle/offline transitions were silently dropped.
        let host = this.clone();
        let update: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |payload: JsValue, _opts: JsValue| {
                on_delta(&host, payload);
            }));
        let _ = Reflect::set(this, &"__tonkUpdate".into(), update.as_ref());
        *self.update.borrow_mut() = Some(update);

        // A transport error on the subscription means the worker is gone
        // (stopped, updating, network down): paint the hollow offline ring
        // so the disc COMMUNICATES the disconnect. The reconnect's next
        // frame repaints the real status, so this heals on its own.
        let host_for_error = this.clone();
        let error: Closure<dyn FnMut(JsValue, JsValue)> =
            Closure::wrap(Box::new(move |_payload: JsValue, _opts: JsValue| {
                paint(&host_for_error, "sync:offline");
            }));
        let _ = Reflect::set(this, &"__tonkError".into(), error.as_ref());
        *self.error.borrow_mut() = Some(error);

        subscribe_status(this, self.subscription.clone());
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        if name != "with" || old == new {
            return;
        }
        // The location landed (or moved): any subscription is addressed to
        // the branch the OLD `with` named — or failed outright while the
        // repo half was blank (`main@` is malformed, and the host refuses
        // it). Drop it (its `Drop` cancels upstream) and subscribe against
        // where `with` points now. This is how the disc comes alive on a
        // host whose first projection stamped a blank space.
        self.subscription.borrow_mut().take();
        subscribe_status(this, self.subscription.clone());
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Dropping the subscription cancels the upstream host subscription.
        self.subscription.borrow_mut().take();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
        self.error.borrow_mut().take();
    }
}

/// Open the status subscription for `this`, on a microtask.
///
/// NOT synchronously: when a render pass runs inside an outer custom-element
/// reaction, the reaction queue delivers the calling callback only after that
/// reaction finishes — by which time the diff may have detached this element
/// again, and a dispatch from a detached tree never reaches the document
/// listeners. By microtask time the DOM has settled; skip if no longer
/// connected (a real re-connection re-runs `connected_callback`) or if a
/// subscription is already live (a same-value attribute re-stamp must not
/// double-subscribe).
fn subscribe_status(this: &HtmlElement, subscription: Rc<RefCell<Option<Subscription>>>) {
    let host = this.clone();
    spawn_local(async move {
        if !host.is_connected() || subscription.borrow().is_some() {
            return;
        }
        let consumer: Element = host.into();
        match status_query_body() {
            Ok(body) => {
                let tag = JsValue::from_str(SUB_TAG);
                match consumer::subscribe(&consumer, &body, Some(&tag)) {
                    Ok(sub) => *subscription.borrow_mut() = Some(sub),
                    Err(err) => {
                        // Dispatch failure: leave the pending disc;
                        // nothing to subscribe to.
                        tonk_common::log!("ui-sync-status: subscribe failed: {err:?}");
                    }
                }
            }
            Err(err) => tonk_common::log!("ui-sync-status: query build failed: {err}"),
        }
    });
}

/// Build the subscribe body for the `tonk:sync` status at `state:here`.
///
/// The wire shape a consumer subscribe takes is `{ predicate, terms }` — the
/// same body `<tonk-display entity=state:here model=tonk:sync>` sends. We build
/// it as JSON directly (the concept is fixed: one `status` field on
/// `xyz.tonk.sync/status`, `this` pinned to the `state:here` singleton, `status`
/// a variable to read back), avoiding a typed-query→wire conversion for one
/// known query. Mirrors the `<tonk-site>` load-claim's hand-built JSON body.
fn status_query_body() -> Result<JsValue, String> {
    // `status` is a cardinality-one `entity` attribute; the description is
    // cosmetic and omitted. Matches the query the topbar chip already issues.
    let body = r#"{
      "predicate": { "with": { "status": {
        "the": "xyz.tonk.sync/status", "as": "Entity", "cardinality": "one"
      } } },
      "terms": { "this": "state:here", "status": { "?": { "name": "status" } } }
    }"#;
    JSON::parse(body).map_err(|e| format!("query JSON parse: {e:?}"))
}

/// A subscription frame: read the first conclusion's `status` and paint the
/// disc. The frame is a `Vec<Conclusion>` serialized as JS; `ReplicaSyncStatus`
/// only derives `Serialize` (it's a stamp type), so we read the `status` field
/// off the raw conclusion rather than deserializing into it. An empty frame (no
/// status stamped yet) leaves the current disc — the SW stamps a status on
/// load, so it's a brief gap, and clearing would flicker.
fn on_frame(host: &HtmlElement, payload: JsValue) {
    // payload is an array of conclusions: [{ this, fields: { status } }, …].
    let conclusions = js_sys::Array::from(&payload);
    let first = conclusions.get(0);
    // `conclusion.fields.status` is the status entity (e.g. "sync:idle").
    let status = (!first.is_undefined() && !first.is_null())
        .then(|| {
            Reflect::get(&first, &"fields".into())
                .ok()
                .and_then(|fields| Reflect::get(&fields, &"status".into()).ok())
                .and_then(|s| s.as_string())
        })
        .flatten();
    if let Some(status) = status {
        paint(host, &status);
    }
}

/// Handle an incremental `update` frame: `{ asserted, retracted }`. The status
/// is cardinality-one on the `state:here` singleton, so a change supersedes the
/// prior value — the newest `asserted` row carries the current status. Retracts
/// alone (no asserted) leave the disc where it is: the SW always stamps a fresh
/// status, so a bare retract is a transient gap, and clearing would flicker
/// (same rationale as an empty `reset` frame).
fn on_delta(host: &HtmlElement, payload: JsValue) {
    let asserted = Reflect::get(&payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    // Cardinality-one: at most one asserted row, carrying the new status.
    let last = rows.get(rows.length().saturating_sub(1));
    let status = (!last.is_undefined() && !last.is_null())
        .then(|| {
            Reflect::get(&last, &"fields".into())
                .ok()
                .and_then(|fields| Reflect::get(&fields, &"status".into()).ok())
                .and_then(|s| s.as_string())
        })
        .flatten();
    if let Some(status) = status {
        paint(host, &status);
    }
}

/// Render (or update) the disc: `<span class="sync sync--<state>"><span
/// class="disc"></span></span>`. Coloring + fill/ring/pulse come from the
/// state MODIFIER CLASS (`sync--synced` / `sync--syncing` / `sync--offline` /
/// `sync--local` / `sync--paused` / `sync--revoked` / `sync--conflict` /
/// `sync--unavailable`), styled in the app stylesheet — the
/// component carries no inline color. Idempotent: reuses the nodes, only
/// swapping the modifier class, so a frame doesn't rebuild the DOM.
fn paint(host: &HtmlElement, status: &str) {
    // Headless: render nothing and report onto the parent instead, which is
    // how a host that draws its own disc (the FAB bar's circle) consumes this
    // subscription. Deliberately addressed to the PARENT rather than a named
    // tag, so this element stays ignorant of who is using it.
    if host.has_attribute("headless") {
        if let Some(parent) = host.parent_element() {
            // Only when it CHANGES.
            //
            // `state` is one of the parent bar's observed attributes, so
            // writing it re-enters that element's
            // `attributeChangedCallback` — and paint runs from inside a
            // callback that already holds the element's lock, which
            // panics with `cannot recursively acquire mutex`. The panic
            // kills the guest frame, the frame reloads, the bar
            // re-mounts and paints again: a crash loop that shows up as
            // endless commits in the worker log.
            //
            // A write that changes nothing has nothing to notify about,
            // so skipping it costs no correctness.
            // Off this turn. Skipping a no-op write covers the repeats,
            // but the FIRST paint genuinely changes the value — and it
            // runs from `connected_callback`, which the parent triggers
            // from inside its own `inject_children` while still holding
            // its lock. A microtask lets that callback finish first.
            defer_write(parent.as_ref(), "state", disc_state(status));
            // The disc has three shapes but sync has eight states, so the
            // precise one travels alongside it for the accessible name —
            // "revoked" and "conflict" both draw a hollow ring, and losing
            // the distinction from the label would make them unreportable.
            defer_write(parent.as_ref(), "data-sync-status", status);
        }
        return;
    }
    let Some(document) = window().and_then(|w| w.document()) else {
        return;
    };
    let modifier = modifier_class(status);
    // Reuse an existing `.sync` span if present; otherwise build it once.
    let sync = match host.query_selector(":scope > .sync") {
        Ok(Some(existing)) => existing,
        _ => {
            let Ok(sync) = document.create_element("span") else {
                return;
            };
            if let Ok(disc) = document.create_element("span") {
                let _ = disc.set_attribute("class", "disc");
                let _ = sync.append_child(&disc);
            }
            let _ = host.append_child(&sync);
            sync
        }
    };
    let _ = sync.set_attribute("class", &format!("sync {modifier}"));
}

/// Map a `sync:*` status entity to its BEM modifier class. An unknown value
/// falls back to `sync--offline` (the neutral hollow ring) rather than an
/// unstyled disc.
fn modifier_class(status: &str) -> &'static str {
    match status {
        "sync:idle" => "sync--synced",
        "sync:pending" => "sync--syncing",
        "sync:local" => "sync--local",
        "sync:paused" => "sync--paused",
        "sync:revoked" => "sync--revoked",
        "sync:conflict" => "sync--conflict",
        "sync:unavailable" => "sync--unavailable",
        "sync:offline" => "sync--offline",
        _ => "sync--unknown",
    }
}

/// Map a `sync:*` status entity to one of the disc's three shapes: filled
/// (online and syncing), the 135° half-fill (deliberately paused), or the
/// hollow ring (everything else — not syncing, for whatever reason).
///
/// Lossy on purpose. The drawn vocabulary is three shapes, and inventing a
/// fourth for `revoked` or `conflict` would be an illustration, not a mark.
/// The precise status rides alongside as `data-sync-status` so nothing that
/// needs the distinction — the accessible name above all — has to lose it.
/// Write `attribute` only when its value differs.
///
/// Re-entrancy guard: several of these land on elements that observe
/// the attribute, and a redundant write still fires their callback.
/// Write an attribute on another element after this turn.
///
/// The parent bar observes what this writes, so the write runs its
/// `attributeChangedCallback` synchronously — and paint can be reached
/// from inside that parent's own `inject_children`, which still holds
/// its lock. On wasm that lock is not reentrant, so it panics with
/// `cannot recursively acquire mutex`, killing the guest frame; the
/// frame reloads, the bar re-mounts and paints again, and the crash
/// loops.
///
/// A microtask is enough: by then the callback that owns the lock has
/// returned. The write is still skipped when it changes nothing.
fn defer_write(element: &web_sys::Element, attribute: &str, value: &str) {
    if element.get_attribute(attribute).as_deref() == Some(value) {
        return;
    }
    let element = element.clone();
    let attribute = attribute.to_owned();
    let value = value.to_owned();
    spawn_local(async move {
        set_if_changed(&element, &attribute, &value);
    });
}

fn set_if_changed(element: &web_sys::Element, attribute: &str, value: &str) {
    if element.get_attribute(attribute).as_deref() == Some(value) {
        return;
    }
    let _ = element.set_attribute(attribute, value);
}

fn disc_state(status: &str) -> &'static str {
    match status {
        "sync:idle" | "sync:pending" => "synced",
        "sync:paused" => "paused",
        _ => "offline",
    }
}

/// Register `<ui-sync-status>`. Idempotent. Installs the prototype `reset`
/// method shim (forwarding to the per-instance `__tonkReset` delegate) so host
/// subscription frames reach the element.
pub(crate) fn register() {
    let Some(win) = window() else {
        return;
    };
    if win.custom_elements().get("ui-sync-status").is_undefined() {
        UiSyncStatus::define("ui-sync-status");
        install_reset_shim();
    }
}

/// Install the `reset` method on the element prototype, forwarding to the
/// per-instance `__tonkReset` closure. On the prototype (not each instance) so
/// `this`-binding is correct — the same pattern `<tonk-display>` uses.
fn install_reset_shim() {
    let Some(win) = window() else {
        return;
    };
    let constructor = win.custom_elements().get("ui-sync-status");
    if constructor.is_undefined() {
        return;
    }
    let Ok(proto) = Reflect::get(&constructor, &"prototype".into()) else {
        return;
    };
    let reset_fn = js_sys::Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkReset === 'function') this.__tonkReset(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"reset".into(), &reset_fn);
    let update_fn = js_sys::Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkUpdate === 'function') this.__tonkUpdate(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"update".into(), &update_fn);
    let error_fn = js_sys::Function::new_with_args(
        "payload, opts",
        "if (typeof this.__tonkError === 'function') this.__tonkError(payload, opts);",
    );
    let _ = Reflect::set(&proto, &"error".into(), &error_fn);
}

#[cfg(test)]
mod tests {
    use super::modifier_class;

    #[dialog_common::test]
    fn it_keeps_typed_and_unknown_failures_distinct_from_offline() {
        assert_eq!(modifier_class("sync:revoked"), "sync--revoked");
        assert_eq!(modifier_class("sync:conflict"), "sync--conflict");
        assert_eq!(modifier_class("sync:unavailable"), "sync--unavailable");
        assert_eq!(modifier_class("sync:offline"), "sync--offline");
        assert_eq!(modifier_class("sync:future"), "sync--unknown");
    }
}
