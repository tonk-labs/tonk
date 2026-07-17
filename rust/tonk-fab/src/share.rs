//! The `<tonk-share>` custom element — mint an invite and copy it, on one click.
//!
//! It wraps the share button (authored directly in `markup.rs` — the deleted
//! `tonk:repository/fab-share` view used to supply it) and turns a click into
//! a single control that mints a fresh invite AND puts the resulting URL on
//! the clipboard, without a second click, then reverts to offering "share"
//! again.
//!
//! ## Why this needs an element at all
//!
//! `navigator.clipboard.writeText` requires *transient user activation*. The
//! mint is an async round-trip (claim → service worker → commit → subscription
//! frame), which outlives that activation — so "await the mint, then write the
//! clipboard" is rejected by the browser.
//!
//! The way out is [`ClipboardItem`]'s promise form: `navigator.clipboard.write`
//! accepts an item whose value is a `Promise<string>`. Construct the item
//! *synchronously inside the click handler*, while activation is still live,
//! and the browser holds the write open until the promise settles. So we open
//! the clipboard write on click, let the mint run, and resolve the promise with
//! the URL when it arrives.
//!
//! ## How the URL gets here
//!
//! This element is built on the shared `subscribing` scaffolding (like
//! `<ui-space-name>`), stamping its own `with` from its `space` attribute and
//! subscribing to [`crate::logic::invite_link_query_body`] — an INLINE
//! predicate over the raw `xyz.tonk.credential/link` attribute, not the
//! rule-derived `tonk:agent-invite` view the deleted `<tonk-display>` used to
//! read (rules, like views, are frozen at whatever `core.yaml` seeded a space
//! with). `render_reset`/`render_update` track the live link in
//! `current_link` and, if a copy is pending, settle it once a NEW link
//! (different from what was on screen at click time) arrives.
//!
//! ## Dispatching the mint
//!
//! There is no `<tonk-display>` delegate installed on this Rust-owned markup
//! to resolve the button's `onsubmit` binding (see `markup.rs`'s module doc),
//! so the click handler dispatches `tonk:invite` itself via
//! `window.tonk.transact`, mirroring `element.rs::dispatch_pause_from_cap`
//! and `space_name.rs::dispatch_rename`.
//!
//! A failed mint has no explicit error signal in this design (the deleted
//! `<tonk-display>`'s error slot is gone with it): a subscription simply
//! never yields a new link. A pending copy is only ever abandoned on
//! disconnect (see `disconnected_callback`) — there is no timeout. That
//! matches this crate's scope (fixing the orphaned mounts), not a redesign
//! of mint failure handling.
//!
//! [`ClipboardItem`]: https://developer.mozilla.org/en-US/docs/Web/API/ClipboardItem

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlElement, window};

use crate::logic::{COPIED_LINGER_MS, ShareState, invite_claim_json, invite_link_query_body};
use crate::subscribing;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = setTimeout)]
    fn set_timeout(handler: &Function, delay: i32) -> i32;

    #[wasm_bindgen(js_namespace = globalThis, js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

/// The clipboard write we opened on click, waiting on the mint.
///
/// `resolve` is the pending `ClipboardItem` promise's resolver: calling it with
/// the invite URL completes the clipboard write the browser has been holding
/// open since the click. `reject` abandons it (the browser drops the write and
/// leaves the existing clipboard contents alone).
struct PendingCopy {
    resolve: Function,
    reject: Function,
    /// The link already on screen when the click landed, if any.
    ///
    /// The invite display holds the LAST mint's link, and every subscription
    /// frame re-renders and re-dispatches `tonk-display:result` — so a frame
    /// carrying the previous link can arrive after our click and before the new
    /// mint commits. Copying that would hand out a stale invite. The mint
    /// supersedes the credential in place (cardinality-one on the subject), so
    /// the new link is always a *different* string: we wait for one that
    /// differs from this.
    stale: Option<String>,
}

/// Per-element state. One pending copy at a time — a click while a mint is in
/// flight is dropped (see [`ShareState::accepts_click`]).
#[derive(Default)]
struct ShareStateCell {
    pending: Option<PendingCopy>,
    /// The `setTimeout` that reverts a `Copied`/`Failed` confirmation to
    /// `Idle`. Cleared and re-armed on each settle so a fresh result always
    /// gets its full linger, and cancelled on disconnect.
    revert: Option<i32>,
}

/// An installed listener, paired with the `Closure` owning its JS-side memory.
/// Dropped (and removed) on disconnect.
type ListenerEntry = (String, Closure<dyn FnMut(web_sys::Event)>);

const SUB_TAG: &str = "tonk-share";

pub struct TonkShare {
    state: Rc<RefCell<ShareStateCell>>,
    /// The last link a subscription frame delivered, tracked here because
    /// there is no longer a DOM element (the deleted invite `<tonk-display>`)
    /// to read it back off at click time.
    current_link: Rc<RefCell<Option<String>>>,
    scaffold: subscribing::Scaffold,
    listeners: Vec<ListenerEntry>,
}

impl Default for TonkShare {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(ShareStateCell::default())),
            current_link: Rc::new(RefCell::new(None)),
            scaffold: subscribing::Scaffold::default(),
            listeners: Vec::new(),
        }
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: the space-derived
/// (default `resolve_with`) routing context, the raw-attribute invite-link
/// query, and settling a pending copy when a fresh link lands.
struct ShareLinkBehaviour {
    state: Rc<RefCell<ShareStateCell>>,
    current_link: Rc<RefCell<Option<String>>>,
}

impl subscribing::Subscribing for ShareLinkBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        let space = this.get_attribute("space").unwrap_or_default();
        invite_link_query_body(&space)
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        if let Some(link) = read_link_from_frame(payload) {
            handle_link(host, &self.state, &self.current_link, link);
        }
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        if let Some(link) = read_link_from_delta(payload) {
            handle_link(host, &self.state, &self.current_link, link);
        }
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl CustomElement for TonkShare {
    /// No Shadow DOM — `<tonk-share>` is a transparent wrapper around the
    /// share button `markup.rs` puts inside it.
    ///
    /// `custom-elements` defaults this to `true`, which attaches a shadow root.
    /// A shadow root with no `<slot>` renders none of the light-DOM children,
    /// so the mint button would vanish — the element would connect with an
    /// empty subtree and the bar would show an empty box where the share
    /// control belongs.
    fn shadow() -> bool {
        false
    }

    fn observed_attributes() -> &'static [&'static str] {
        &["space"]
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        set_state(this, ShareState::Idle);

        // Click: open the clipboard write while activation is live. This must
        // stay synchronous all the way to `clipboard.write()` — any `await`
        // before it spends the activation and the write is refused.
        //
        // Unlike the deleted `<tonk-display>`-mounted view, there is no
        // delegate to resolve the button's form submission into a claim, so
        // this handler ALSO dispatches the mint itself (`dispatch_invite`),
        // synchronously, after opening the clipboard write — both calls stay
        // in the same click task, so activation is still live for both.
        let state = Rc::clone(&self.state);
        let current_link = Rc::clone(&self.current_link);
        let host = this.clone();
        let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            // The button is `type="submit"` inside a `<form>` (for layout
            // parity with the deleted view's exact markup — see
            // `markup.rs`), but nothing resolves its native submission into
            // anything useful: always suppress it, then dispatch ourselves.
            event.prevent_default();

            let current = read_state(&host);
            if !current.accepts_click() {
                // A mint is already in flight holding the clipboard promise.
                // A second mint would rotate the credential out from under
                // the copy we're about to complete.
                return;
            }
            let Some(space) = host.get_attribute("space").filter(|s| !s.is_empty()) else {
                return;
            };
            // Whatever link the last subscription frame delivered is the
            // PREVIOUS mint's. Note it so a frame still carrying it isn't
            // mistaken for our result.
            let stale = current_link.borrow().clone();
            match open_clipboard_write(Rc::clone(&state), stale) {
                Ok(()) => set_state(&host, ShareState::Copying),
                // No clipboard (an insecure context, a denied permission, or a
                // browser without the promise form). The mint still runs — so
                // the link is minted and the subscription still updates; it
                // just isn't auto-copied. Better than blocking the share
                // outright.
                Err(e) => {
                    warn(&format!("share: clipboard unavailable: {e:?}"));
                    set_state(&host, ShareState::Copying);
                }
            }
            dispatch_invite(&space, js_sys::Date::now());
        });
        add_listener(this, "click", &on_click);
        self.listeners.push(("click".to_owned(), on_click));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        let behaviour: Rc<dyn subscribing::Subscribing> = Rc::new(ShareLinkBehaviour {
            state: Rc::clone(&self.state),
            current_link: Rc::clone(&self.current_link),
        });
        self.scaffold.connect(this, behaviour);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        self.scaffold.disconnect();
        for (event_type, closure) in self.listeners.drain(..) {
            let target: &web_sys::EventTarget = this.unchecked_ref();
            let _ = target
                .remove_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
        }
        let mut state = self.state.borrow_mut();
        if let Some(id) = state.revert.take() {
            clear_timeout(id);
        }
        // Abandon a clipboard write still held open by a mint that will now
        // never land — leaving it pending would hold the clipboard hostage.
        if let Some(pending) = state.pending.take() {
            let _ = pending.reject.call1(
                &JsValue::NULL,
                &JsValue::from_str("share: element detached"),
            );
        }
    }
}

/// Dispatch the `tonk:invite` claim via `window.tonk.transact`, routeless —
/// mirroring `element.rs::dispatch_pause_from_cap` and
/// `space_name.rs::dispatch_rename`. There is no `<tonk-display>` delegate
/// installed on this Rust-owned markup to resolve the button's form
/// submission into a claim, so the click handler dispatches it directly.
fn dispatch_invite(space: &str, time: f64) {
    let claim = invite_claim_json(space, time);
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

/// Open a clipboard write against a promise we resolve later.
///
/// Synchronous on purpose: it runs inside the click handler so the browser sees
/// a user-activated `clipboard.write()`. The returned promise is parked in
/// `state.pending`; the browser holds the write open until we settle it.
fn open_clipboard_write(
    state: Rc<RefCell<ShareStateCell>>,
    stale: Option<String>,
) -> Result<(), JsValue> {
    let clipboard = window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .navigator()
        .clipboard();

    // Capture the executor's resolve/reject so the mint can settle this promise
    // from outside.
    let slot: Rc<RefCell<Option<(Function, Function)>>> = Rc::new(RefCell::new(None));
    let captured = Rc::clone(&slot);
    let text = Promise::new(&mut move |resolve, reject| {
        *captured.borrow_mut() = Some((resolve, reject));
    });
    let (resolve, reject) = slot
        .borrow_mut()
        .take()
        .ok_or_else(|| JsValue::from_str("promise executor did not run"))?;
    let pending = PendingCopy {
        resolve,
        reject,
        stale,
    };

    // `new ClipboardItem({ "text/plain": <Promise<string>> })` — the promise
    // form. `writeText` cannot express this: it takes a resolved string, so it
    // would need the URL up front, which is the thing we don't have yet.
    let item_init = Object::new();
    Reflect::set(&item_init, &JsValue::from_str(MIME_TEXT), &text)?;
    let item = clipboard_item(&item_init)?;

    // A rejected write (permission denied, or the promise we reject when the
    // mint fails) must not surface as an unhandled rejection.
    let on_rejected = Closure::<dyn FnMut(JsValue)>::new(|e: JsValue| {
        warn(&format!("share: clipboard write failed: {e:?}"));
    });
    let _ = clipboard.write(&Array::of1(&item)).catch(&on_rejected);

    state.borrow_mut().pending = Some(pending);
    Ok(())
}

/// `new ClipboardItem(init)` via the global constructor. web-sys does not
/// expose a `ClipboardItem` binding in the features we enable, and we need the
/// promise-valued form regardless (its typed binding takes resolved values).
fn clipboard_item(init: &Object) -> Result<JsValue, JsValue> {
    let global = js_sys::global();
    let ctor = Reflect::get(&global, &JsValue::from_str("ClipboardItem"))?;
    let ctor = ctor
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("ClipboardItem is not constructible"))?;
    Reflect::construct(&ctor, &Array::of1(init.as_ref()))
}

const MIME_TEXT: &str = "text/plain";

/// Complete (or abandon) the clipboard write the click opened, and move the
/// control to its confirmation state.
fn settle(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>, result: Result<String, &str>) {
    let Some(pending) = state.borrow_mut().pending.take() else {
        return;
    };
    let settled = match result {
        Ok(link) => {
            let _ = pending
                .resolve
                .call1(&JsValue::NULL, &JsValue::from_str(&link));
            ShareState::Copied
        }
        Err(reason) => {
            let _ = pending
                .reject
                .call1(&JsValue::NULL, &JsValue::from_str(reason));
            ShareState::Failed
        }
    };
    set_state(host, settled);
    arm_revert(host, state);
}

/// Revert a confirmation to `Idle` after [`COPIED_LINGER_MS`], so the control
/// goes back to offering "share" instead of latching on "copied" for the rest
/// of the session (which is what the old copy button did).
fn arm_revert(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>) {
    let mut cell = state.borrow_mut();
    if let Some(id) = cell.revert.take() {
        clear_timeout(id);
    }
    let host = host.clone();
    let state_for_timer = Rc::clone(state);
    let revert = Closure::once_into_js(move || {
        state_for_timer.borrow_mut().revert = None;
        // Don't stomp a mint the user started during the linger.
        if read_state(&host).is_transient() {
            set_state(&host, ShareState::Idle);
        }
    });
    cell.revert = Some(set_timeout(
        revert.unchecked_ref::<Function>(),
        COPIED_LINGER_MS,
    ));
}

/// Track a freshly delivered link and, if a copy is pending, settle it —
/// unless this is still the previous mint's link (the new one hasn't landed
/// yet, and the subscription can re-deliver the same row on an unrelated
/// frame).
///
/// `current_link` is updated UNCONDITIONALLY, pending copy or not: it is the
/// replacement for reading the old `<tonk-display>`'s rendered DOM text, and
/// that used to update on every frame regardless of whether a copy was in
/// flight.
fn handle_link(
    host: &HtmlElement,
    state: &Rc<RefCell<ShareStateCell>>,
    current_link: &Rc<RefCell<Option<String>>>,
    link: String,
) {
    let pending_stale = state
        .borrow()
        .pending
        .as_ref()
        .and_then(|pending| pending.stale.clone());
    let is_pending = state.borrow().pending.is_some();
    *current_link.borrow_mut() = Some(link.clone());
    if !is_pending {
        return;
    }
    // Still the previous mint's link — the new one hasn't landed yet. Keep
    // the clipboard write open and wait for the next frame.
    if Some(&link) == pending_stale.as_ref() {
        return;
    }
    settle(host, state, Ok(link));
}

/// A subscription snapshot (`reset`) frame: the invite link off the first
/// (and only — cardinality-one) conclusion row.
fn read_link_from_frame(payload: &JsValue) -> Option<String> {
    let conclusions = js_sys::Array::from(payload);
    read_link_field(&conclusions.get(0))
}

/// An incremental `update` frame: `{ asserted, retracted }`. `link` is
/// cardinality-one, so the newest asserted row carries the current value; a
/// bare retract (no asserted) is a no-op here.
fn read_link_from_delta(payload: &JsValue) -> Option<String> {
    let asserted =
        Reflect::get(payload, &JsValue::from_str("asserted")).unwrap_or(JsValue::UNDEFINED);
    let rows = js_sys::Array::from(&asserted);
    read_link_field(&rows.get(rows.length().saturating_sub(1)))
}

/// Read `conclusion.fields.link` off a raw subscription row — the assembled
/// (and shortened) invite URL the worker's mint asserted onto the space's
/// `xyz.tonk.credential/link` attribute. `None` for a missing/empty row or an
/// empty link.
fn read_link_field(row: &JsValue) -> Option<String> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    Reflect::get(row, &JsValue::from_str("fields"))
        .ok()
        .and_then(|fields| Reflect::get(&fields, &JsValue::from_str("link")).ok())
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())
}

/// Stamp the state on the host. The view stylesheet keys the label and spinner
/// off `data-share-state`, so the element owns the state machine and the
/// stylesheet owns the appearance.
fn set_state(host: &HtmlElement, state: ShareState) {
    let _ = host.set_attribute("data-share-state", state.as_str());
}

fn read_state(host: &HtmlElement) -> ShareState {
    match host.get_attribute("data-share-state").as_deref() {
        Some("copying") => ShareState::Copying,
        Some("copied") => ShareState::Copied,
        Some("failed") => ShareState::Failed,
        _ => ShareState::Idle,
    }
}

fn add_listener(
    host: &HtmlElement,
    event_type: &str,
    closure: &Closure<dyn FnMut(web_sys::Event)>,
) {
    let target: &web_sys::EventTarget = host.unchecked_ref();
    let _ = target.add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref());
}

fn warn(message: &str) {
    web_sys::console::warn_1(&JsValue::from_str(message));
}

/// Register `<tonk-share>`. Idempotent.
pub fn register() {
    let Some(win) = window() else {
        return;
    };
    if !win.custom_elements().get("tonk-share").is_undefined() {
        return;
    }
    TonkShare::define("tonk-share");
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Object;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    use web_sys::{Element, window};

    wasm_bindgen_test_configure!(run_in_browser);

    /// A fresh, unconnected host — plenty for tests that only exercise the
    /// state-machine helpers (`set_state`/`read_state`/`settle`), not a
    /// delivered subscription frame.
    fn fresh_host() -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        document
            .create_element("div")
            .expect("create host")
            .unchecked_into()
    }

    /// A host mounted inside an OPEN share segment, the way the FAB nests it:
    /// `.fab__share.is-open > <tonk-share>`. Returns the segment so a test can
    /// assert on its classes.
    fn host_in_open_segment() -> (HtmlElement, Element) {
        let document = window().expect("window").document().expect("document");
        let segment = document.create_element("span").expect("create segment");
        segment.set_class_name("fab__seg fab__share is-open");
        let host: HtmlElement = document
            .create_element("tonk-share")
            .expect("create host")
            .unchecked_into();
        segment.append_child(&host).expect("nest host");
        (host, segment)
    }

    /// A subscription row, in the shape a real delivered conclusion takes:
    /// `{ fields: { … } }`. The concept's attributes are nested under
    /// `fields` — reading `link` off the top level always finds nothing.
    fn row_with_fields(pairs: &[(&str, &str)]) -> JsValue {
        let fields = Object::new();
        for (key, value) in pairs {
            Reflect::set(&fields, &JsValue::from_str(key), &JsValue::from_str(value))
                .expect("set field");
        }
        let row = Object::new();
        Reflect::set(&row, &JsValue::from_str("fields"), &fields).expect("set fields");
        row.into()
    }

    /// A `reset` snapshot payload: a bare array of one conclusion row.
    fn reset_payload(pairs: &[(&str, &str)]) -> JsValue {
        let rows = js_sys::Array::new();
        rows.push(&row_with_fields(pairs));
        rows.into()
    }

    /// An `update` delta payload: `{ asserted, retracted }`.
    fn update_payload(pairs: &[(&str, &str)]) -> JsValue {
        let asserted = js_sys::Array::new();
        asserted.push(&row_with_fields(pairs));
        let payload = Object::new();
        Reflect::set(&payload, &JsValue::from_str("asserted"), &asserted).expect("set asserted");
        payload.into()
    }

    #[wasm_bindgen_test]
    fn it_reads_the_link_out_of_the_conclusions_fields() {
        // The link is nested under `fields`, alongside the invite's other
        // attributes — exactly as a real minted invite's row is shaped.
        assert_eq!(
            read_link_field(&row_with_fields(&[
                ("link", "https://tonk.xyz/@/abc"),
                ("code", "zSeed"),
            ])),
            Some("https://tonk.xyz/@/abc".to_owned()),
        );
    }

    #[wasm_bindgen_test]
    fn it_ignores_a_malformed_or_empty_frame() {
        assert_eq!(read_link_field(&Object::new().into()), None);
        assert_eq!(read_link_field(&row_with_fields(&[("link", "")])), None);
        assert_eq!(read_link_field(&JsValue::UNDEFINED), None);
    }

    #[wasm_bindgen_test]
    fn it_reads_a_reset_snapshot_and_an_update_delta() {
        assert_eq!(
            read_link_from_frame(&reset_payload(&[("link", "https://tonk.xyz/@/a")])),
            Some("https://tonk.xyz/@/a".to_owned()),
        );
        assert_eq!(
            read_link_from_delta(&update_payload(&[("link", "https://tonk.xyz/@/b")])),
            Some("https://tonk.xyz/@/b".to_owned()),
        );
    }

    #[wasm_bindgen_test]
    fn it_sees_no_current_link_before_the_first_frame() {
        // A space that has never been shared has delivered no frame yet, so
        // there is no stale link to guard against.
        let current_link: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        assert_eq!(*current_link.borrow(), None);
    }

    #[wasm_bindgen_test]
    fn it_tracks_the_link_from_every_frame_even_with_no_copy_pending() {
        // Mirrors what reading the old `<tonk-display>`'s rendered DOM text
        // used to do: `current_link` reflects the last delivered frame
        // regardless of whether a copy is in flight.
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        let current_link: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        handle_link(
            &host,
            &state,
            &current_link,
            "https://tonk.xyz/@/first".to_owned(),
        );

        assert_eq!(
            *current_link.borrow(),
            Some("https://tonk.xyz/@/first".to_owned())
        );
        // No copy was pending, so nothing settles.
        assert_eq!(read_state(&host), ShareState::Idle);
    }

    #[wasm_bindgen_test]
    fn it_settles_a_pending_copy_when_a_fresh_link_lands() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        let current_link: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        // The link on screen at click time — what the click captured as
        // `stale`.
        open_clipboard_write(Rc::clone(&state), Some("https://tonk.xyz/@/old".to_owned()))
            .expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_link(
            &host,
            &state,
            &current_link,
            "https://tonk.xyz/@/new".to_owned(),
        );

        assert_eq!(read_state(&host), ShareState::Copied);
        assert!(
            state.borrow().pending.is_none(),
            "a fresh link must settle the pending copy",
        );
    }

    #[wasm_bindgen_test]
    fn it_keeps_waiting_when_a_frame_still_carries_the_stale_link() {
        // The subscription can re-deliver the previous mint's row before the
        // new one lands — that must not be mistaken for the result.
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        let current_link: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        open_clipboard_write(Rc::clone(&state), Some("https://tonk.xyz/@/old".to_owned()))
            .expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_link(
            &host,
            &state,
            &current_link,
            "https://tonk.xyz/@/old".to_owned(),
        );

        assert_eq!(
            read_state(&host),
            ShareState::Copying,
            "a frame still carrying the stale link must not settle the copy",
        );
        assert!(state.borrow().pending.is_some());
    }

    #[wasm_bindgen_test]
    fn it_defaults_to_idle_before_the_element_upgrades() {
        // No `data-share-state` yet — the button must read "share", not blank.
        assert_eq!(read_state(&fresh_host()), ShareState::Idle);
    }

    #[wasm_bindgen_test]
    fn it_round_trips_every_state_through_the_dom() {
        // The stylesheet keys the label swap off this attribute, so a state
        // that doesn't survive the round-trip renders the wrong label.
        let host = fresh_host();
        for state in [
            ShareState::Idle,
            ShareState::Copying,
            ShareState::Copied,
            ShareState::Failed,
        ] {
            set_state(&host, state);
            assert_eq!(read_state(&host), state);
        }
    }

    #[wasm_bindgen_test]
    fn it_settles_a_pending_copy_and_shows_copied() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        assert!(state.borrow().pending.is_some(), "the copy is pending");

        settle(&host, &state, Ok("https://tonk.xyz/@/new".to_owned()));

        assert_eq!(read_state(&host), ShareState::Copied);
        assert!(
            state.borrow().pending.is_none(),
            "settling must consume the pending copy, so a later frame can't re-settle it",
        );
    }

    #[wasm_bindgen_test]
    fn it_leaves_the_roster_alone_when_the_copy_settles() {
        // `<tonk-share>` does not touch the dropdown. `<tonk-fab>` toggles
        // `.is-open` on the segment for every click in the share zone, so the
        // menu opens on the first click and closes on the second. Force-closing
        // it here would desync that toggle: the click after an auto-close would
        // re-OPEN the menu instead of closing it.
        let (host, segment) = host_in_open_segment();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");

        settle(&host, &state, Ok("https://tonk.xyz/@/new".to_owned()));

        assert!(
            segment.class_list().contains("is-open"),
            "settling must leave the menu's open state to <tonk-fab>'s toggle",
        );
    }

    #[wasm_bindgen_test]
    fn it_shows_failed_when_the_mint_errors() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        settle(&host, &state, Err("mint failed"));

        // Failed, not stuck on `copying` — and the clipboard promise is
        // rejected, so the browser abandons the write instead of holding it.
        assert_eq!(read_state(&host), ShareState::Failed);
        assert!(state.borrow().pending.is_none());
    }

    #[wasm_bindgen_test]
    fn it_settles_only_once() {
        // Only the first frame after a click may settle the copy; a later
        // one must find nothing pending and leave the confirmation alone.
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");

        settle(&host, &state, Ok("https://tonk.xyz/@/first".to_owned()));
        assert_eq!(read_state(&host), ShareState::Copied);

        // A second frame with no pending copy: a no-op, not a state change.
        settle(&host, &state, Err("late error"));
        assert_eq!(
            read_state(&host),
            ShareState::Copied,
            "a frame arriving after the copy settled must not flip it to failed",
        );
    }
}
