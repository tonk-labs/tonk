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

impl UiSyncStatus {
    /// Open the status subscription against the element's current `with`
    /// routing context. A no-op if already subscribed. Shared by
    /// `connected_callback` and `attribute_changed_callback` (a `with` that
    /// resolved after mount).
    ///
    /// Subscribes on a microtask, NOT synchronously: when a render pass runs
    /// inside an outer custom-element reaction, the reaction queue delivers
    /// this only after that reaction finishes — by which time the diff may have
    /// detached this element, and a dispatch from a detached tree never reaches
    /// the document listeners. By microtask time the DOM has settled; skip if
    /// no longer connected.
    fn subscribe_now(&self, this: &HtmlElement) {
        let subscription = self.subscription.clone();
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
}

impl CustomElement for UiSyncStatus {
    fn shadow() -> bool {
        // Light DOM: the disc's CSS lives in the app stylesheet, and the
        // element's `with` context resolves through the light tree.
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        // Observe `with` so a late-resolving routing context — the FAB stamps
        // `with="main@{space}"` and rewrites it once the space DID lands — drives
        // a (re)subscribe. Without this the chip subscribes once against the
        // still-unresolved `main@` (a malformed location the host rejects) and
        // stays stuck "offline".
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

        self.subscribe_now(this);
    }

    fn attribute_changed_callback(
        &mut self,
        this: &HtmlElement,
        name: String,
        old: Option<String>,
        new: Option<String>,
    ) {
        // The FAB rewrites `with="main@{space}"` to the resolved location once
        // the space DID lands. Re-subscribe against the new routing context.
        if name != "with" || old == new || !this.is_connected() {
            return;
        }
        // Drop the prior (failed-or-stale) subscription so `subscribe_now`
        // re-establishes it against the new `with`.
        self.subscription.borrow_mut().take();
        self.subscribe_now(this);
    }

    fn disconnected_callback(&mut self, _this: &HtmlElement) {
        // Dropping the subscription cancels the upstream host subscription.
        self.subscription.borrow_mut().take();
        self.reset.borrow_mut().take();
        self.update.borrow_mut().take();
        self.error.borrow_mut().take();
    }
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
/// `sync--local` / `sync--paused`), styled in the app stylesheet — the
/// component carries no inline color. Idempotent: reuses the nodes, only
/// swapping the modifier class, so a frame doesn't rebuild the DOM.
fn paint(host: &HtmlElement, status: &str) {
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
        // "sync:offline" and anything unrecognized.
        _ => "sync--offline",
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
