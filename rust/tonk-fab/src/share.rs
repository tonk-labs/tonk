//! The `<tonk-share>` custom element — mint an invite and copy it, on one click.
//!
//! It wraps the share zone's two `<tonk-display>`s (the mint trigger and the
//! invite link) and turns them into a single control. Clicking it mints a fresh
//! invite AND puts the resulting URL on the clipboard, without a second click,
//! then reverts to offering "share" again.
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
//! We do not plumb it out of the claim. The mint's own transact receipt is
//! discarded by the event delegate, but the minted invite lands on
//! `tonk:agent-invite` (with the assembled, shortened `link`), and the invite
//! `<tonk-display>` re-renders and dispatches a bubbling `tonk-display:result`
//! carrying that conclusion. This element listens for it and reads `link`.
//!
//! [`ClipboardItem`]: https://developer.mozilla.org/en-US/docs/Web/API/ClipboardItem

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{CustomEvent, HtmlElement, window};

use crate::logic::{COPIED_LINGER_MS, ShareState};

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

pub struct TonkShare {
    state: Rc<RefCell<ShareStateCell>>,
    listeners: Vec<ListenerEntry>,
}

impl Default for TonkShare {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(ShareStateCell::default())),
            listeners: Vec::new(),
        }
    }
}

impl CustomElement for TonkShare {
    /// No Shadow DOM — `<tonk-share>` is a transparent wrapper around the two
    /// `<tonk-display>`s the FAB template puts inside it.
    ///
    /// `custom-elements` defaults this to `true`, which attaches a shadow root.
    /// A shadow root with no `<slot>` renders none of the light-DOM children,
    /// so the mint button and the invite display would both vanish — the
    /// element would connect with an empty subtree and the bar would show an
    /// empty box where the share control belongs.
    fn shadow() -> bool {
        false
    }

    fn inject_children(&mut self, this: &HtmlElement) {
        set_state(this, ShareState::Idle);

        // Click: open the clipboard write while activation is live. This must
        // stay synchronous all the way to `clipboard.write()` — any `await`
        // before it spends the activation and the write is refused.
        //
        // We do NOT dispatch the mint ourselves. The inner `<form
        // onsubmit=tonk:invite>` (a space-branch view) already does, and its
        // binding is what carries the right routing context. We let the click
        // through to it and only arrange for the result to be copied.
        let state = Rc::clone(&self.state);
        let host = this.clone();
        let on_click = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let current = read_state(&host);
            if !current.accepts_click() {
                // A mint is already in flight holding the clipboard promise.
                // Cancel the activation so the form does not submit a second
                // mint, which would rotate the credential out from under the
                // copy we're about to complete.
                //
                // `preventDefault` only — NOT `stopPropagation`. The click still
                // has to reach `<tonk-fab>`, which toggles the roster for every
                // click in the share zone. Swallowing it outright would make the
                // menu unresponsive for as long as a mint is in flight.
                event.prevent_default();
                return;
            }
            // Whatever link is on screen right now is the PREVIOUS mint's. Note
            // it so a frame still carrying it isn't mistaken for our result.
            let stale = rendered_link(&host);
            match open_clipboard_write(Rc::clone(&state), stale) {
                Ok(()) => set_state(&host, ShareState::Copying),
                // No clipboard (an insecure context, a denied permission, or a
                // browser without the promise form). The mint still runs — the
                // click falls through to the form — so the link is minted and
                // rendered; it just isn't auto-copied. Better than blocking the
                // share outright.
                Err(e) => {
                    warn(&format!("share: clipboard unavailable: {e:?}"));
                    set_state(&host, ShareState::Copying);
                }
            }
        });
        add_listener(this, "click", &on_click);
        self.listeners.push(("click".to_owned(), on_click));

        // The invite `<tonk-display>` re-renders when the mint's conclusion
        // arrives and dispatches a bubbling `tonk-display:result` carrying it.
        // That is our "the URL exists now" signal.
        let state = Rc::clone(&self.state);
        let host = this.clone();
        let on_result = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            // Ignore frames that arrive when we aren't waiting on one: the
            // invite display re-renders on any subscription frame, including
            // ones no click of ours provoked.
            let stale = match state.borrow().pending.as_ref() {
                Some(pending) => pending.stale.clone(),
                None => return,
            };
            let Some(link) = event
                .dyn_ref::<CustomEvent>()
                .and_then(|e| link_from_detail(&e.detail()))
            else {
                return;
            };
            // Still the previous mint's link — the new one hasn't landed yet.
            // Keep the clipboard write open and wait for the next frame.
            if Some(&link) == stale.as_ref() {
                return;
            }
            settle(&host, &state, Ok(link));
        });
        add_listener(this, "tonk-display:result", &on_result);
        self.listeners
            .push(("tonk-display:result".to_owned(), on_result));

        // A failed mint surfaces as the display's error event. Without this the
        // control would sit in `Copying` forever and the clipboard would keep
        // an unresolved promise open.
        let state = Rc::clone(&self.state);
        let host = this.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if state.borrow().pending.is_none() {
                return;
            }
            settle(&host, &state, Err("the invite display reported an error"));
        });
        add_listener(this, "tonk-display:error", &on_error);
        self.listeners
            .push(("tonk-display:error".to_owned(), on_error));
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
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

/// Read the invite URL out of a `tonk-display:result` detail.
///
/// The detail is a serialized conclusion, `{ this, fields: { … } }` — the
/// concept's attributes live under `fields`, NOT at the top level. `link` is
/// the assembled (and shortened) invite URL the worker's mint asserted onto
/// `tonk:credential`.
///
/// Absent before a mint lands: the display also frames the bare
/// `tonk:repository` name (`fields: { name }`), which carries no link. Those
/// frames must not settle a pending copy.
fn link_from_detail(detail: &JsValue) -> Option<String> {
    let fields = Reflect::get(detail, &JsValue::from_str("fields")).ok()?;
    let link = Reflect::get(&fields, &JsValue::from_str("link")).ok()?;
    link.as_string().filter(|s| !s.is_empty())
}

/// The link the invite display is currently showing — i.e. the last mint's, if
/// any. The `fab-invite` view renders it into a hidden `.fab__invite-link`.
/// `None` before the first mint.
fn rendered_link(host: &HtmlElement) -> Option<String> {
    let el = host.query_selector(".fab__invite-link").ok().flatten()?;
    let text = el.text_content()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
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

    /// A host carrying the invite display's rendered link, exactly as the
    /// `fab-invite` view emits it.
    fn host_showing(link: Option<&str>) -> HtmlElement {
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("div")
            .expect("create host")
            .unchecked_into();
        if let Some(link) = link {
            host.set_inner_html(&format!(
                r#"<span class="fab__invite-link" hidden>{link}</span>"#
            ));
        }
        host
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

    /// A `tonk-display:result` detail, in the shape the display actually
    /// dispatches: a serialized conclusion `{ this, fields: { … } }`. The
    /// concept's attributes are nested under `fields` — reading `link` off the
    /// top level (as an earlier cut of this did) always finds nothing, and the
    /// copy hangs forever waiting for a link that is right there.
    fn detail_with_fields(pairs: &[(&str, &str)]) -> JsValue {
        let fields = Object::new();
        for (key, value) in pairs {
            Reflect::set(&fields, &JsValue::from_str(key), &JsValue::from_str(value))
                .expect("set field");
        }
        let detail = Object::new();
        Reflect::set(
            &detail,
            &JsValue::from_str("this"),
            &JsValue::from_str("did:key:zSubject"),
        )
        .expect("set this");
        Reflect::set(&detail, &JsValue::from_str("fields"), &fields).expect("set fields");
        detail.into()
    }

    #[wasm_bindgen_test]
    fn it_reads_the_link_out_of_the_conclusions_fields() {
        // The link is nested under `fields`, alongside the invite's other
        // attributes — exactly as the display serializes a real minted invite.
        assert_eq!(
            link_from_detail(&detail_with_fields(&[
                ("name", "home"),
                ("link", "https://tonk.xyz/@/abc"),
                ("code", "zSeed"),
            ])),
            Some("https://tonk.xyz/@/abc".to_owned()),
        );
    }

    #[wasm_bindgen_test]
    fn it_ignores_the_pre_mint_frame() {
        // Before a mint lands, the display still frames the repo's bare name
        // (`fields: { name }`) — no `link`. Treating that as the result settles
        // the copy with nothing, so the guard has to hold out for a real link.
        assert_eq!(
            link_from_detail(&detail_with_fields(&[("name", "home")])),
            None
        );
    }

    #[wasm_bindgen_test]
    fn it_ignores_a_malformed_or_empty_frame() {
        assert_eq!(link_from_detail(&Object::new().into()), None);
        assert_eq!(link_from_detail(&detail_with_fields(&[("link", "")])), None);
    }

    #[wasm_bindgen_test]
    fn it_sees_no_rendered_link_before_the_first_mint() {
        // A space that has never been shared renders no invite, so there is no
        // stale link to guard against.
        assert_eq!(rendered_link(&host_showing(None)), None);
    }

    #[wasm_bindgen_test]
    fn it_reads_the_previous_mints_link_off_the_dom() {
        // This is what makes the stale guard work: on click we can see which
        // link is already on screen, so we know to keep waiting when a frame
        // still carries it.
        assert_eq!(
            rendered_link(&host_showing(Some("https://tonk.xyz/@/old"))),
            Some("https://tonk.xyz/@/old".to_owned()),
        );
    }

    #[wasm_bindgen_test]
    fn it_defaults_to_idle_before_the_element_upgrades() {
        // No `data-share-state` yet — the button must read "share", not blank.
        assert_eq!(read_state(&host_showing(None)), ShareState::Idle);
    }

    #[wasm_bindgen_test]
    fn it_round_trips_every_state_through_the_dom() {
        // The stylesheet keys the label swap off this attribute, so a state
        // that doesn't survive the round-trip renders the wrong label.
        let host = host_showing(None);
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
        let host = host_showing(None);
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
        let host = host_showing(None);
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
        // The invite display re-renders on every subscription frame. Only the
        // first frame after a click may settle the copy; a later one must find
        // nothing pending and leave the confirmation alone.
        let host = host_showing(None);
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
