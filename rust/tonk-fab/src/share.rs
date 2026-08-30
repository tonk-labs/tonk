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
//! ## When the mint is refused
//!
//! A space whose `main` has no sync remote cannot be shared: the worker refuses
//! to mint rather than hand out an invite that would strand whoever claimed it
//! in an empty space. It records that refusal as `xyz.tonk.share/{blocked,
//! detail,time}` on the space's subject, which this element reads on a SECOND
//! subscription (see [`ShareBlockedBehaviour`]) — separate from the link
//! subscription because a single predicate over both would resolve only when a
//! refusal AND a link are present, which never happens.
//!
//! A refusal the prompt can repair abandons the clipboard write and opens the
//! enable-sync dialog. Confirming it is a FRESH user activation, so a new
//! clipboard write opens there and the attach-then-mint the worker runs
//! settles it — one click, from refusal to link on the clipboard.
//!
//! Two classes are repairable, and they share the claim but not the wording
//! (see [`Repair`]). `not-synced` attaches a remote. `missing-revocation-relay`
//! means the space syncs fine but its remote carries no relay, so an invite
//! could never be withdrawn — every space whose remote predates in-band
//! revocation is in that state; confirming upserts the relay onto the remote
//! already there, leaving its address and its branch upstream untouched.
//!
//! The remaining classes (`unshareable-remote`, and `attach-failed` — an
//! attach the user already approved that failed anyway) have no action the
//! dialog could offer, but `detail` still has to reach the user: the button's
//! "failed" label is a static string. `handle_blocked` re-opens the same
//! dialog for these with the confirm button disabled, rather than routing the
//! sentence to a clipboard-rejection message nothing ever reads.
//!
//! Anything else that goes wrong still has no explicit error signal (the
//! deleted `<tonk-display>`'s error slot is gone with it): a subscription
//! simply never yields a new link. [`arm_timeout`] is the backstop there. It
//! is not the only one that matters: a control left on `copying` refuses every
//! later click ([`ShareState::accepts_click`]), so every way out of a mint has
//! to end somewhere clickable — [`fail_copy`] covers a click that never opened
//! a clipboard write for `settle` to consume, and `disconnected_callback`
//! covers an element re-parented mid-mint (`inject_children` runs once, so a
//! reconnect restores no state of its own).
//!
//! [`ClipboardItem`]: https://developer.mozilla.org/en-US/docs/Web/API/ClipboardItem

use std::cell::RefCell;
use std::rc::Rc;

use custom_elements::CustomElement;
use js_sys::{Array, Function, JSON, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement, window};

use crate::logic::{
    COPIED_LINGER_MS, SHARE_TIMEOUT_MS, ShareState, enable_sync_claim_json, invite_claim_json,
    invite_state_query_body,
};
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
    clipboard: PendingClipboard,
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

/// A clipboard write opened while user activation is still live and settled
/// later once an asynchronous mint returns its URL.
pub(crate) struct PendingClipboard {
    resolve: Function,
    reject: Function,
}

impl PendingClipboard {
    pub(crate) fn resolve(self, text: &str) {
        let _ = self.resolve.call1(&JsValue::NULL, &JsValue::from_str(text));
    }

    pub(crate) fn reject(self, reason: &str) {
        let _ = self
            .reject
            .call1(&JsValue::NULL, &JsValue::from_str(reason));
    }
}

/// A refusal delivered on the blocked subscription.
#[derive(Debug, Clone, PartialEq)]
struct Blocked {
    /// `account-required` | `not-synced` | `unshareable-remote` | `attach-failed`.
    code: String,
    /// The sentence to show.
    detail: String,
    /// The timestamp of the command this answers.
    time: f64,
}

/// The enable-sync prompt's id, and the attributes marking its confirm button,
/// its reason slot, and the line describing what confirming does. Authored in
/// `markup.rs`; every lookup here is `Option`-guarded, so the element still
/// shares (and still refuses) in a host that renders no dialog at all.
///
/// The dialog is not only for `not-synced`: [`handle_blocked`] re-opens it on
/// every refusal so `detail` always lands somewhere visible, disabling the
/// confirm button (see `open_enable_sync_dialog`) on the one class it cannot
/// repair.
/// The refusal class the enable-sync prompt can still repair: an account
/// with a provider, and a space not yet attached to it.
const BLOCKED_NOT_SYNCED: &str = tonk_worker_api::share::BLOCKED_NOT_SYNCED;

/// The account enrolled but never confirmed the emailed link.
const BLOCKED_NEEDS_ACTIVATION: &str = tonk_worker_api::share::BLOCKED_NEEDS_ACTIVATION;

const DIALOG_ID: &str = "fabb-connect-cluster";
const DIALOG_CONFIRM: &str = "[data-enable-sync-confirm]";
const DIALOG_DETAIL: &str = "[data-enable-sync-detail]";
const DIALOG_ACTION: &str = "[data-enable-sync-action]";
const DIALOG_STATEMENT: &str = "[data-enable-sync-statement]";
const DIALOG_REMOTE: &str = "[data-enable-sync-remote]";

/// Marks the dialog as answering a refusal whose repair is registration
/// rather than an attach, so the confirm handler navigates instead of
/// dispatching enable-sync.
const DIALOG_OUTCOME: &str = "data-repair-register";

/// Heading and confirm label for a refusal with no repair. The button stays
/// visible but disabled, so it needs wording that doesn't promise an action —
/// "Turn on sync & copy link" greyed out reads as a broken control rather than
/// an answer.
const TERMINAL_LABEL: &str = "this space cannot be shared";
const TERMINAL_CONFIRM: &str = "copy link";

/// What confirming the prompt is offering to do, for one refusal class.
///
/// The prompt serves two repairs that share a claim but not a sentence, plus
/// a terminal class it only reports. Holding the whole wording per class —
/// rather than swapping `detail` alone, as it used to — is what keeps a
/// synced space from being told to turn on sync: before this, EVERY refusal
/// re-used the `not-synced` copy, so a missing relay showed a greyed-out
/// "Turn on sync & copy link" under a sentence about a device-only space.
///
/// `None` from [`Repair::for_code`] is the terminal case: report `detail`,
/// no action line, confirm disabled.
struct Repair {
    /// The dialog's `label` attribute — its heading.
    label: &'static str,
    /// The line under `detail` saying what confirming does.
    action: &'static str,
    /// The confirm button's text.
    confirm: &'static str,
    /// What confirming does.
    outcome: RepairOutcome,
}

/// What the dialog's confirm button carries out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RepairOutcome {
    /// Attach the account's remote to this space, then mint — the
    /// one-click path that has always existed.
    EnableSync,
    /// Leave for `/account`, where registration lives. The bar is a
    /// sealed guest with no ceremony of its own, so this hands off
    /// rather than pretending to run one; the user shares again once
    /// they have an account.
    Register,
}

impl Repair {
    /// The repair for a refusal class, or `None` when there is none to offer.
    fn for_code(code: &str) -> Option<Self> {
        match code {
            BLOCKED_NOT_SYNCED => Some(Self {
                label: "connect this space",
                action: "Connect it so the people you share with can open it.",
                confirm: "connect",
                outcome: RepairOutcome::EnableSync,
            }),
            // `needs-account` offers no repair here on purpose. Whether
            // a share issues a link or gets an account first is the
            // worker's decision, and it asks the page for registration
            // itself — so a prompt here would be a second, competing
            // path to the same dialog.
            BLOCKED_NEEDS_ACTIVATION => Some(Self {
                label: "confirm your email to share",
                action: "We sent you a link. Confirm your address, then share again.",
                confirm: "open account settings",
                outcome: RepairOutcome::Register,
            }),
            _ => None,
        }
    }
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
    /// The `setTimeout` that fails a copy nothing ever answered. Armed when
    /// the click opens the write, cleared on every settle.
    timeout: Option<i32>,
    /// The timestamp of the command the copy in flight belongs to — set on the
    /// share click and again on the enable-sync confirm, cleared wherever a
    /// copy concludes. A refusal only counts if it echoes this.
    ///
    /// It lives in the same cell as `pending` deliberately: it is the SOLE
    /// gate on acting on a refusal (a refusal has to reach the user whether or
    /// not a clipboard write ever opened), so it has to move in lockstep with
    /// the copy it names rather than drift in a cell of its own.
    pending_time: Option<f64>,
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
    /// Listeners installed on `document` rather than on the host, kept apart
    /// because they have to be REMOVED from `document` too: removing them from
    /// the host would silently do nothing and leave a live closure over this
    /// element's state behind after it is gone.
    document_listeners: Vec<ListenerEntry>,
}

impl Default for TonkShare {
    fn default() -> Self {
        Self {
            state: Rc::new(RefCell::new(ShareStateCell::default())),
            current_link: Rc::new(RefCell::new(None)),
            scaffold: subscribing::Scaffold::default(),
            listeners: Vec::new(),
            document_listeners: Vec::new(),
        }
    }
}

/// This element's [`subscribing::Subscribing`] behaviour: one row per
/// space saying where its invite has got to.
///
/// One subscription, not two. The control used to run a link query and a
/// refusal query and branch on five reason codes to pick a repair — which
/// put the judgement of *why* a share failed in the caller. The worker
/// makes that call now, and this renders the answer:
///
/// | `status` | The control |
/// |---|---|
/// | `invite:granted` | settle the copy with `url` |
/// | `invite:requested` | keep waiting |
/// | anything else | failed |
///
/// The default arm is what lets a new terminal status ship without
/// touching this file.
struct InviteStateBehaviour {
    state: Rc<RefCell<ShareStateCell>>,
    current_link: Rc<RefCell<Option<String>>>,
}

impl subscribing::Subscribing for InviteStateBehaviour {
    fn query_body(&self, this: &HtmlElement) -> Result<String, String> {
        let space = this.get_attribute("space").unwrap_or_default();
        invite_state_query_body(&space)
    }

    fn render_reset(&self, host: &HtmlElement, payload: &JsValue) {
        let rows = js_sys::Array::from(payload);
        if let Some(invite) = read_invite_row(&rows.get(0)) {
            self.apply(host, invite);
        }
    }

    fn render_update(&self, host: &HtmlElement, payload: &JsValue) {
        let asserted =
            Reflect::get(payload, &JsValue::from_str("asserted")).unwrap_or(JsValue::UNDEFINED);
        let rows = js_sys::Array::from(&asserted);
        if let Some(invite) = read_invite_row(&rows.get(rows.length().saturating_sub(1))) {
            self.apply(host, invite);
        }
    }

    fn tag(&self) -> &'static str {
        SUB_TAG
    }
}

impl InviteStateBehaviour {
    /// Render one row.
    fn apply(&self, host: &HtmlElement, invite: InviteRow) {
        match invite.status.as_str() {
            tonk_schema::command::InviteState::GRANTED => {
                let Some(url) = invite.url else {
                    // Granted with no url is a malformed row; waiting is
                    // safer than reporting a copy that never happened.
                    return;
                };
                handle_link(host, &self.state, &self.current_link, url);
            }
            tonk_schema::command::InviteState::REQUESTED => {}
            // Terminal. The button's "failed" label carries no reason, so
            // anything that wants to say more has to render the status
            // itself; this only stops the spinner.
            _ => {
                if self.state.borrow().pending.is_some() {
                    fail_copy(host, &self.state, "");
                } else {
                    set_state(host, ShareState::Blocked);
                }
            }
        }
    }
}

/// Read `conclusion.fields.{status,url}` off a raw subscription row.
///
/// `status` is required; `url` is optional, present only once granted —
/// which is why a request in flight still resolves and the control can
/// distinguish "waiting" from "nothing asked".
fn read_invite_row(row: &JsValue) -> Option<InviteRow> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let fields = Reflect::get(row, &JsValue::from_str("fields")).ok()?;
    let status = Reflect::get(&fields, &JsValue::from_str("status"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())?;
    let url = Reflect::get(&fields, &JsValue::from_str("url"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty());
    Some(InviteRow { status, url })
}

/// One `tonk:invite` row as the control reads it.
struct InviteRow {
    /// One of the `invite:*` markers.
    status: String,
    /// The invite URL, once granted.
    url: Option<String>,
}

/// Read `conclusion.fields.{blocked,detail,time}` off a raw subscription row.
/// Read `conclusion.fields.{blocked,detail,time}` off a raw subscription row.
/// `None` for a missing row or any missing field — all three are asserted
/// together, so a partial row is not a refusal.
fn read_blocked_row(row: &JsValue) -> Option<Blocked> {
    if row.is_undefined() || row.is_null() {
        return None;
    }
    let fields = Reflect::get(row, &JsValue::from_str("fields")).ok()?;
    let code = Reflect::get(&fields, &JsValue::from_str("blocked"))
        .ok()
        .and_then(|v| v.as_string())
        .filter(|s| !s.is_empty())?;
    let detail = Reflect::get(&fields, &JsValue::from_str("detail"))
        .ok()
        .and_then(|v| v.as_string())?;
    let time = Reflect::get(&fields, &JsValue::from_str("time"))
        .ok()
        .and_then(|v| v.as_f64())?;
    Some(Blocked { code, detail, time })
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
            // The claim's timestamp is also this click's identity: a refusal
            // echoes it back, and that is how a refusal answering THIS click
            // is told from one the overlay is replaying (see
            // `handle_blocked`).
            let time = js_sys::Date::now();
            state.borrow_mut().pending_time = Some(time);
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
            arm_timeout(&host, &state);
            dispatch_invite(&space, time);
        });
        add_listener(this, "click", &on_click);
        self.listeners.push(("click".to_owned(), on_click));
    }

    fn connected_callback(&mut self, this: &HtmlElement) {
        self.connect_subscriptions(this);
        self.install_confirm_listener(this);
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
        // The space landed (or moved). Any subscriptions were opened against
        // the old value — or skipped entirely while it was blank
        // (`resolve_with` returns `None` on an empty `space`, and
        // `connect_all` no-ops). Drop them and subscribe against the space
        // that is actually here; without this, a mint still fires on click
        // but the link it produces has no subscription left to arrive on,
        // and the copy can only time out.
        self.scaffold.disconnect();
        self.connect_subscriptions(this);
    }

    fn disconnected_callback(&mut self, this: &HtmlElement) {
        self.scaffold.disconnect();
        for (event_type, closure) in self.listeners.drain(..) {
            let target: &web_sys::EventTarget = this.unchecked_ref();
            let _ = target
                .remove_event_listener_with_callback(&event_type, closure.as_ref().unchecked_ref());
        }
        // The confirm listener is on `document`, not on the host — removing it
        // from the host would silently succeed and leave it installed, still
        // holding this element's state.
        if let Some(document) = window().and_then(|w| w.document()) {
            for (event_type, closure) in self.document_listeners.drain(..) {
                let target: &web_sys::EventTarget = document.unchecked_ref();
                let _ = target.remove_event_listener_with_callback(
                    &event_type,
                    closure.as_ref().unchecked_ref(),
                );
            }
        }
        // Hand the button back. `inject_children` is the only other
        // `set_state(Idle)` and it runs once, on first connect — so an element
        // re-parented mid-mint would otherwise come back stamped `copying`,
        // with nothing pending and no timer left to fail it. That state
        // refuses every click, and nothing would ever move it: a dead button
        // for the rest of the session.
        set_state(this, ShareState::Idle);
        let mut state = self.state.borrow_mut();
        state.pending_time = None;
        if let Some(id) = state.revert.take() {
            clear_timeout(id);
        }
        if let Some(id) = state.timeout.take() {
            clear_timeout(id);
        }
        // Abandon a clipboard write still held open by a mint that will now
        // never land — leaving it pending would hold the clipboard hostage.
        if let Some(pending) = state.pending.take() {
            pending.clipboard.reject("share: element detached");
        }
    }
}

impl TonkShare {
    /// Open the element's two subscriptions: the minted link, and the refusal
    /// that says no link is coming. The scaffolding routes each frame back by
    /// the tag its behaviour subscribed under.
    ///
    /// Run from `connected_callback`, and again from the `space` attribute
    /// callback once a late-arriving space lands — `connect_all` is built to
    /// be re-run (it no-ops while the routing context is unresolvable and
    /// dedupes live tags).
    fn connect_subscriptions(&self, this: &HtmlElement) {
        let invite: Rc<dyn subscribing::Subscribing> = Rc::new(InviteStateBehaviour {
            state: Rc::clone(&self.state),
            current_link: Rc::clone(&self.current_link),
        });
        self.scaffold.connect_all(this, vec![invite]);
    }

    /// Listen for the enable-sync prompt's confirm, wherever it is in the
    /// document.
    ///
    /// Delegated rather than bound to the button: `<tonk-share>` and the dialog
    /// are set as one `innerHTML` string, so a direct lookup at connect time
    /// can race the dialog into existence.
    ///
    /// The confirm click is a FRESH user activation, which is the whole reason
    /// this can complete the copy: a new clipboard write opens here and the
    /// browser holds it through the attach and the mint that follow. The write
    /// the refused click opened is long gone — [`abandon`] dropped it rather
    /// than leaving it hostage to a question the user had not answered yet.
    fn install_confirm_listener(&mut self, this: &HtmlElement) {
        // `connected_callback` runs again every time the element is re-parented;
        // installing a second copy would leave the first behind on `document`.
        if !self.document_listeners.is_empty() {
            return;
        }
        let Some(document) = window().and_then(|w| w.document()) else {
            return;
        };
        let host = this.clone();
        let state = Rc::clone(&self.state);
        let current_link = Rc::clone(&self.current_link);
        let on_confirm = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            let Some(confirm) = target.closest(DIALOG_CONFIRM).ok().flatten() else {
                return;
            };
            event.prevent_default();
            // `open_enable_sync_dialog` disables this button on an
            // unrepairable refusal (there is no attach to retry) but leaves
            // the dialog itself open so the detail sentence stays visible.
            // Nothing native backs "disabled" on a custom-element button, so
            // this has to be checked explicitly rather than relied on to
            // block the event.
            if confirm.has_attribute("disabled") {
                return;
            }
            // A second confirm while the first attach is still running would
            // orphan the clipboard write this one opened — the browser would
            // hold it open forever, with nothing left able to settle it.
            if !read_state(&host).accepts_click() {
                return;
            }
            let Some(space) = host.get_attribute("space").filter(|s| !s.is_empty()) else {
                return;
            };
            // Registration runs in the TOP page, the only document with
            // both a `window` and the user gesture WebAuthn wants. This
            // frame has neither the ceremony nor the account UI, so it
            // asks through the portal bridge and the dialog opens over
            // the space rather than navigating away from it.
            //
            // The space rides along so the dialog can finish what this
            // click started: once an account exists, the share it
            // interrupted mints and hands over the link.
            //
            // The clipboard write is abandoned rather than held. A
            // ceremony consumes transient activation, so a write
            // attempted after it costs a second permission prompt; the
            // link gets a copy button instead.
            if let Some(reason) =
                enable_sync_dialog().and_then(|dialog| dialog.get_attribute(DIALOG_OUTCOME))
            {
                fail_copy(&host, &state, "");
                close_enable_sync_dialog();
                tonk_host::request_registration(
                    &serde_json::json!({ "reason": reason, "space": space }).to_string(),
                );
                return;
            }
            let wants_share = enable_sync_dialog().is_some_and(|dialog| {
                dialog.get_attribute("data-share").as_deref() == Some("true")
            });

            let time = js_sys::Date::now();
            state.borrow_mut().pending_time = Some(time);
            if wants_share {
                let stale = current_link.borrow().clone();
                if let Err(e) = open_clipboard_write(Rc::clone(&state), stale) {
                    warn(&format!("share: clipboard unavailable: {e:?}"));
                }
                set_state(&host, ShareState::Copying);
                arm_timeout(&host, &state);
            }
            if let Some(narrator) = enable_sync_dialog()
                .and_then(|dialog| dialog.query_selector(DIALOG_DETAIL).ok().flatten())
            {
                narrator.set_text_content(Some("connecting…"));
            }
            let _ = confirm.set_attribute("disabled", "");
            // No remote: the worker resolves where this account syncs.
            // Deriving it here meant asking a sealed guest for its own
            // origin — `about:srcdoc`, so `location.origin` is the
            // opaque `"null"` until the portal bridge injects the real
            // one — and a share before that arrived returned silently.
            dispatch_enable_sync(&space, "", wants_share, time);
        });
        let target: &web_sys::EventTarget = document.unchecked_ref();
        let _ =
            target.add_event_listener_with_callback("click", on_confirm.as_ref().unchecked_ref());
        self.document_listeners
            .push(("click".to_owned(), on_confirm));

        let host = this.clone();
        let on_bail = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            if target.id() != DIALOG_ID {
                return;
            }
            close_enable_sync_dialog();
            if read_state(&host) == ShareState::Blocked {
                set_state(&host, ShareState::Idle);
            }
        });
        let target: &web_sys::EventTarget = document.unchecked_ref();
        let _ =
            target.add_event_listener_with_callback("fabb-bail", on_bail.as_ref().unchecked_ref());
        self.document_listeners
            .push(("fabb-bail".to_owned(), on_bail));

        let on_commit = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(field) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            else {
                return;
            };
            if !field.matches(DIALOG_REMOTE).unwrap_or(false) {
                return;
            }
            let Some(confirm) = enable_sync_dialog()
                .and_then(|dialog| dialog.query_selector(DIALOG_CONFIRM).ok().flatten())
            else {
                return;
            };
            confirm.unchecked_ref::<HtmlElement>().click();
        });
        let target: &web_sys::EventTarget = document.unchecked_ref();
        let _ = target
            .add_event_listener_with_callback("fabb-commit", on_commit.as_ref().unchecked_ref());
        self.document_listeners
            .push(("fabb-commit".to_owned(), on_commit));
    }
}

/// Dispatch the `tonk:invite` claim via `window.tonk.transact`, routeless —
/// mirroring `element.rs::dispatch_pause_from_cap` and
/// `space_name.rs::dispatch_rename`. There is no `<tonk-display>` delegate
/// installed on this Rust-owned markup to resolve the button's form
/// submission into a claim, so the click handler dispatches it directly.
fn dispatch_invite(space: &str, time: f64) {
    dispatch_claim(&invite_claim_json(space, time));
}

/// Dispatch the `tonk:enable-sync` claim, asking the worker to attach `remote`
/// to this space and — because `share` is set — mint the invite the refused
/// click was after, as soon as the attach lands.
fn dispatch_enable_sync(space: &str, remote: &str, share: bool, time: f64) {
    dispatch_claim(&enable_sync_claim_json(space, remote, share, time));
}

/// Hand a claim to `window.tonk.transact`. A no-op wherever the bridge is not
/// installed, rather than an error the user would see.
fn dispatch_claim(claim: &serde_json::Value) {
    let json_str = match serde_json::to_string(claim) {
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
    let clipboard = open_deferred_clipboard_write()?;
    state.borrow_mut().pending = Some(PendingCopy { clipboard, stale });
    Ok(())
}

/// Open a promise-backed clipboard write synchronously for an asynchronous
/// operation to settle later.
pub(crate) fn open_deferred_clipboard_write() -> Result<PendingClipboard, JsValue> {
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

    // `new ClipboardItem({ "text/plain": <Promise<string>> })` — the promise
    // form. `writeText` cannot express this: it takes a resolved string, so it
    // would need the URL up front, which is the thing we don't have yet.
    let item_init = Object::new();
    Reflect::set(&item_init, &JsValue::from_str(MIME_TEXT), &text)?;
    let item = clipboard_item(&item_init)?;

    // A rejected write (permission denied, or the promise we reject when the
    // mint fails) must not surface as an unhandled rejection.
    //
    // `forget` hands the closure's memory to JS for good. The write is still
    // in flight when this scope ends, so dropping the closure here would free
    // it while the browser still holds the reference: the eventual rejection
    // would then invoke freed memory and throw ("closure invoked recursively
    // or after being dropped") rather than warn. The leak is one small closure
    // per share click, bounded by user clicks.
    let on_rejected = Closure::<dyn FnMut(JsValue)>::new(|e: JsValue| {
        warn(&format!("share: clipboard write failed: {e:?}"));
    });
    let _ = clipboard.write(&Array::of1(&item)).catch(&on_rejected);
    on_rejected.forget();

    Ok(PendingClipboard { resolve, reject })
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
    {
        let mut cell = state.borrow_mut();
        // The click this answers is answered: nothing later may claim it.
        cell.pending_time = None;
        if let Some(id) = cell.timeout.take() {
            clear_timeout(id);
        }
    }
    let Some(pending) = state.borrow_mut().pending.take() else {
        return;
    };
    let settled = match result {
        Ok(link) => {
            pending.clipboard.resolve(&link);
            ShareState::Copied
        }
        Err(reason) => {
            pending.clipboard.reject(reason);
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

/// Fail a copy that nothing ever answered, so the control never pins on
/// `copying` (which refuses further clicks) when a mint dies silently.
fn arm_timeout(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>) {
    let mut cell = state.borrow_mut();
    if let Some(id) = cell.timeout.take() {
        clear_timeout(id);
    }
    let host = host.clone();
    let state_for_timer = Rc::clone(state);
    let expire = Closure::once_into_js(move || {
        // Clear our own handle FIRST: `settle` clears `timeout` too, and it
        // must not try to cancel the timer that is currently running.
        state_for_timer.borrow_mut().timeout = None;
        fail_copy(&host, &state_for_timer, "share: timed out");
    });
    cell.timeout = Some(set_timeout(
        expire.unchecked_ref::<Function>(),
        SHARE_TIMEOUT_MS,
    ));
}

/// Fail a copy: settle it if a clipboard write is open, and make sure the
/// control leaves `copying` either way.
///
/// The second half is not redundant. A click that could not open a write (no
/// promise-form `ClipboardItem`, an insecure context, a denied permission)
/// stamped `copying` with nothing pending, so `settle` finds nothing to consume
/// and returns without touching the state — and having just cleared the
/// timeout, it leaves no backstop either. That is a button stuck on `copying`,
/// which [`ShareState::accepts_click`] refuses: dead for the rest of the
/// session. `settle` cannot be the one to fix it — it must stay a no-op with
/// nothing pending, or a late frame would flip an already-`copied` control to
/// `failed` (see `it_settles_only_once`).
fn fail_copy(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>, reason: &str) {
    settle(host, state, Err(reason));
    if read_state(host) == ShareState::Copying {
        set_state(host, ShareState::Failed);
        arm_revert(host, state);
    }
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

/// Act on a refusal, if it answers the click currently awaiting an answer.
///
/// The refusal fact is cardinality-one on the space's subject, so the overlay
/// keeps the last one and redelivers it on every resubscribe. Matching on the
/// echoed timestamp is what separates "this click was refused" from "here is
/// an old refusal again" — without it, one refused share would poison every
/// later one for the rest of the session.
///
/// That timestamp is the ONLY gate. A refusal has to reach the user whether or
/// not a clipboard write ever opened: gating on `pending` as well would drop
/// the refusal on exactly the browsers that cannot open one (no promise-form
/// `ClipboardItem`), so the user would sit through the timeout instead of being
/// offered the repair. Clearing `pending_time` here is what keeps the replay
/// hole shut — a stale refusal's `time` can only match a click that has not
/// already been answered.
fn handle_blocked(host: &HtmlElement, state: &Rc<RefCell<ShareStateCell>>, blocked: Blocked) {
    if state.borrow().pending_time != Some(blocked.time) {
        return;
    }
    state.borrow_mut().pending_time = None;

    let Some(repair) = Repair::for_code(&blocked.code) else {
        // Nothing the prompt could fix — including an attach that failed after
        // the user already accepted it. Say so rather than leaving the button
        // spinning until the timeout. Through `fail_copy`, not bare `settle`,
        // because there may be no write open to consume.
        fail_copy(host, state, &blocked.detail);
        // The button's static "failed" label carries no reason — it's the
        // same static string every time. The detail sentence has to reach the
        // user somewhere, so re-open the dialog to show it. There is no
        // action it could offer (the remote is unshareable, or the attach the
        // user just approved already failed), so the confirm button is
        // disabled rather than absent — visible, but not a second attempt at
        // something that cannot help.
        open_enable_sync_dialog(&blocked.detail, None);
        return;
    };

    // Repairable: abandon the copy and ask. `settle` would stamp `Failed` and
    // arm the revert timer, which is wrong while a question is on screen — the
    // control would quietly flip back to "share" underneath the dialog.
    abandon(state, &blocked.detail);
    set_state(host, ShareState::Blocked);
    open_enable_sync_dialog(&blocked.detail, Some(repair));
}

/// Drop a pending clipboard write without moving the control's state. The
/// browser releases the write and leaves the existing clipboard alone.
fn abandon(state: &Rc<RefCell<ShareStateCell>>, reason: &str) {
    let mut cell = state.borrow_mut();
    if let Some(id) = cell.timeout.take() {
        clear_timeout(id);
    }
    if let Some(pending) = cell.pending.take() {
        pending.clipboard.reject(reason);
    }
}

/// Show the enable-sync prompt, filling in the reason. A no-op when the
/// dialog is absent, so the element still works in a host that does not
/// render it.
///
/// `repair` is the refusal's offer, or `None` for a class the dialog can only
/// report (`unshareable-remote`, `attach-failed`): that disables the confirm
/// button — there is no attach to retry — while leaving the dialog open so
/// `detail` stays visible, and blanks the action line, which would otherwise
/// promise something the greyed-out button cannot do.
///
/// Every slot is written on every open, never left at its markup default: the
/// dialog is one element reused across refusal classes, so anything not
/// rewritten here is the *previous* refusal's wording. That is the bug this
/// signature exists to make hard — a repairable refusal must also restore
/// what a terminal one disabled.
fn open_enable_sync_dialog(detail: &str, repair: Option<Repair>) {
    open_enable_sync_ceremony(detail, repair, true);
}

fn open_enable_sync_ceremony(detail: &str, repair: Option<Repair>, wants_share: bool) {
    let Some(dialog) = enable_sync_dialog() else {
        return;
    };
    if let Ok(Some(slot)) = dialog.query_selector(DIALOG_DETAIL) {
        slot.set_text_content(Some(detail));
    }
    // The terminal wording, used wherever `repair` is `None`.
    let label = repair
        .as_ref()
        .map_or(TERMINAL_LABEL, |repair| repair.label);
    let action = repair.as_ref().map_or("", |repair| repair.action);
    let confirm_label = repair
        .as_ref()
        .map_or(TERMINAL_CONFIRM, |repair| repair.confirm);

    if let Ok(Some(slot)) = dialog.query_selector(DIALOG_ACTION) {
        slot.set_text_content(Some(action));
    }
    if let Ok(Some(slot)) = dialog.query_selector(DIALOG_STATEMENT) {
        slot.set_text_content(Some(label));
    }
    let _ = dialog.set_attribute("label", label);
    let _ = dialog.set_attribute("data-share", if wants_share { "true" } else { "false" });
    // Stamp what confirming does, so the confirm handler branches on the
    // refusal it is answering rather than assuming enable-sync.
    match repair.as_ref().map(|repair| repair.outcome) {
        Some(RepairOutcome::Register) => {
            let _ = dialog.set_attribute(DIALOG_OUTCOME, "register");
        }
        _ => {
            let _ = dialog.remove_attribute(DIALOG_OUTCOME);
        }
    }
    if let Ok(Some(confirm)) = dialog.query_selector(DIALOG_CONFIRM) {
        confirm.set_text_content(Some(confirm_label));
        // Visible either way — an unrepairable refusal greys the button rather
        // than removing it, so the dialog reads as an answer and not as a form
        // with a button missing. `disabled` is the whole mechanism: the click
        // handler checks it explicitly, since nothing native backs the
        // attribute on a custom-element button.
        if repair.is_some() {
            let _ = confirm.remove_attribute("disabled");
        } else {
            let _ = confirm.set_attribute("disabled", "");
        }
    }
    let _ = dialog.remove_attribute("hidden");
    if let Some(banner) = window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(crate::bar::CONNECT_BANNER_ID))
    {
        let _ = banner.set_attribute("hidden", "");
    }
}

/// Open the same editable connect ceremony from the local-only condition
/// banner. Unlike the share refusal, this attaches without minting a link.
pub(crate) fn open_enable_sync_from_banner() {
    open_enable_sync_ceremony(
        "This space only exists on this device.",
        Repair::for_code(BLOCKED_NOT_SYNCED),
        false,
    );
}

fn close_enable_sync_dialog() {
    let Some(dialog) = enable_sync_dialog() else {
        return;
    };
    let _ = dialog.set_attribute("hidden", "");
    if let Some(banner) = window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(crate::bar::CONNECT_BANNER_ID))
    {
        let _ = banner.remove_attribute("hidden");
    }
}

fn enable_sync_dialog() -> Option<Element> {
    window()?.document()?.get_element_by_id(DIALOG_ID)
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
    // The row the user actually sees is a sibling in the bar's share stack —
    // this element is headless there. Stamp the state onto it too so the row
    // can answer in place ("copy link" → "copying…" → "copied"), which is the
    // same word-answers grammar the Hub's rows use. Absent (a fixture, or the
    // element used standalone) this simply finds nothing.
    if let Some(bar) = host.closest("tonk-fab").ok().flatten()
        && let Ok(Some(row)) = bar.query_selector("[data-share-link]")
    {
        let _ = row.set_attribute("data-share-state", state.as_str());
    }
}

fn read_state(host: &HtmlElement) -> ShareState {
    match host.get_attribute("data-share-state").as_deref() {
        Some("copying") => ShareState::Copying,
        // Without this arm `Blocked` reads straight back as `Idle`, and every
        // later decision that reads the control's state — the confirm guard,
        // the revert timer's `is_transient` check — silently sees the wrong
        // thing.
        Some("blocked") => ShareState::Blocked,
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

/// Register `<tonk-share>`. Idempotent. Installs the prototype `reset`/
/// `update` method shims (forwarding to the per-instance `__tonkReset`/
/// `__tonkUpdate` delegates) so host subscription frames reach the element —
/// the same pattern every other `subscribing`-built element uses.
///
/// Without the shims the element subscribes fine and then goes deaf: the host
/// delivers a frame by calling `element.reset(...)`, finds no such method, and
/// drops it. No frame ever carries the minted link in, so the pending copy
/// never settles and the control pins on `Copying` — which
/// [`ShareState::accepts_click`] refuses, leaving the button dead for the rest
/// of the session.
pub fn register() {
    if subscribing::already_registered(SUB_TAG) {
        return;
    }
    TonkShare::define(SUB_TAG);
    subscribing::install_frame_shims(SUB_TAG);
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::subscribing::Subscribing;
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

    /// A refusal row, as the blocked subscription delivers it: `blocked` and
    /// `detail` are text, `time` is a float (an echoed `dom.event/time-stamp`),
    /// so this cannot reuse [`row_with_fields`]'s all-strings shape.
    fn invite_row(status: &str, url: Option<&str>) -> JsValue {
        let fields = Object::new();
        Reflect::set(&fields, &"status".into(), &JsValue::from_str(status)).expect("set status");
        if let Some(url) = url {
            Reflect::set(&fields, &"url".into(), &JsValue::from_str(url)).expect("set url");
        }
        let row = Object::new();
        Reflect::set(&row, &"fields".into(), &fields).expect("set fields");
        row.into()
    }

    fn invite_reset_payload(status: &str, url: Option<&str>) -> JsValue {
        let rows = js_sys::Array::new();
        rows.push(&invite_row(status, url));
        rows.into()
    }

    fn invite_update_payload(status: &str, url: Option<&str>) -> JsValue {
        let asserted = js_sys::Array::new();
        asserted.push(&invite_row(status, url));
        let payload = Object::new();
        Reflect::set(&payload, &"asserted".into(), &asserted).expect("set asserted");
        payload.into()
    }

    fn blocked_row(code: &str, time: f64) -> JsValue {
        let fields = Object::new();
        Reflect::set(&fields, &"blocked".into(), &JsValue::from_str(code)).expect("set blocked");
        Reflect::set(&fields, &"detail".into(), &JsValue::from_str("no remote"))
            .expect("set detail");
        Reflect::set(&fields, &"time".into(), &JsValue::from_f64(time)).expect("set time");
        let row = Object::new();
        Reflect::set(&row, &"fields".into(), &fields).expect("set fields");
        row.into()
    }

    fn blocked_reset_payload(code: &str, time: f64) -> JsValue {
        let rows = js_sys::Array::new();
        rows.push(&blocked_row(code, time));
        rows.into()
    }

    /// An `update` delta payload carrying one refusal — the shape production
    /// actually delivers a refusal in (see `share.rs`'s module doc: a
    /// refusal always arrives after the subscription is already open).
    fn blocked_update_payload(code: &str, time: f64) -> JsValue {
        let asserted = js_sys::Array::new();
        asserted.push(&blocked_row(code, time));
        let payload = Object::new();
        Reflect::set(&payload, &"asserted".into(), &asserted).expect("set asserted");
        payload.into()
    }

    /// A stand-in for `markup.rs`'s `#fab-enable-sync` dialog: just enough
    /// structure (the id, the detail slot, the action line, the confirm
    /// button) for `open_enable_sync_dialog`'s lookups to find something.
    /// Attached to `document.body` — `enable_sync_dialog` reads through
    /// `document.get_element_by_id`, not a passed-in root.
    ///
    /// The action line is not returned: only the tests about per-refusal
    /// wording read it, and they do so through [`action_text`].
    fn dialog_stub() -> (Element, Element, Element) {
        remove_refusal_dialog();
        let document = window().expect("window").document().expect("document");
        let dialog = document.create_element("div").expect("create dialog");
        dialog.set_id(DIALOG_ID);
        let detail = document.create_element("p").expect("create detail");
        detail
            .set_attribute("data-enable-sync-detail", "")
            .expect("mark detail");
        let action = document.create_element("p").expect("create action");
        action
            .set_attribute("data-enable-sync-action", "")
            .expect("mark action");
        let confirm = document.create_element("button").expect("create confirm");
        confirm
            .set_attribute("data-enable-sync-confirm", "")
            .expect("mark confirm");
        dialog.append_child(&detail).expect("attach detail");
        dialog.append_child(&action).expect("attach action");
        dialog.append_child(&confirm).expect("attach confirm");
        document
            .body()
            .expect("body")
            .append_child(&dialog)
            .expect("attach dialog");
        (dialog, detail, confirm)
    }

    /// Refusal dialogs use a fixed document id in production. Keep test
    /// fixtures unique too so one test cannot update a stale duplicate.
    fn remove_refusal_dialog() {
        let document = window().expect("window").document().expect("document");
        if let Some(dialog) = document.get_element_by_id(DIALOG_ID) {
            dialog.remove();
        }
    }

    /// The prompt's action line — what confirming is being offered as.
    fn action_text(dialog: &Element) -> String {
        dialog
            .query_selector(DIALOG_ACTION)
            .expect("query action")
            .expect("action slot")
            .text_content()
            .unwrap_or_default()
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
        // `Blocked` is in here for a reason: it is stamped by `handle_blocked`
        // and read back by every later click and timer, so a missing arm in
        // `read_state` would silently degrade it to `Idle`.
        for state in [
            ShareState::Idle,
            ShareState::Copying,
            ShareState::Blocked,
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

    /// Mount the bar's real markup so the refusal dialogs a handler opens are
    /// the authored ones, not a fixture's idea of them. Returns the mounted
    /// host, which the caller removes.
    fn mounted_bar() -> HtmlElement {
        remove_refusal_dialog();
        let document = window().expect("window").document().expect("document");
        let host: HtmlElement = document
            .create_element("tonk-fab")
            .expect("create host")
            .unchecked_into();
        let _ = host.set_attribute("space", "did:key:zShareFixture");
        document
            .body()
            .expect("body")
            .append_child(&host)
            .expect("mount");
        // The refusal prompts are mounted on <body> by the bar, not authored
        // inside it — see `element::mount_refusal_dialogs`.
        crate::element::mount_refusal_dialogs();
        host
    }

    /// A refusal with no repair still has to explain itself. The confirm stays
    /// on screen and inert — greyed, so the dialog reads as an answer rather
    /// than a form with a missing button.
    ///
    /// It used to set `hidden` as well, which did nothing: Web Awesome styles
    /// `wa-button`'s display, and an author rule beats the UA's `[hidden]`.
    /// Restoring `[hidden]`'s meaning app-wide turned that dead attribute into
    /// a live one and made the button vanish, so the attribute goes and
    /// `disabled` — which the click handler checks anyway — does the work.
    #[dialog_common::test]
    fn it_keeps_an_unrepairable_refusals_confirm_visible_and_inert() {
        let bar = mounted_bar();

        open_enable_sync_dialog("This space's sync server can't be shared.", None);

        let confirm = window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(DIALOG_ID))
            .and_then(|dialog| dialog.query_selector(DIALOG_CONFIRM).ok().flatten())
            .expect("the sync prompt authors a confirm");
        let hidden = confirm.has_attribute("hidden");
        let disabled = confirm.has_attribute("disabled");
        bar.remove();
        remove_refusal_dialog();

        assert!(!hidden, "the confirm stays on screen");
        assert!(disabled, "and inert");
    }

    /// A refusal whose timestamp matches the pending click abandons the copy
    /// and moves the control to `blocked`.
    #[dialog_common::test]
    fn it_blocks_on_a_matching_refusal() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(42.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "not-synced".to_owned(),
                detail: "This space only exists on this device.".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(read_state(&host), ShareState::Blocked);
        assert!(
            state.borrow().pending.is_none(),
            "the clipboard write is abandoned, not left open"
        );
    }

    /// A refusal from an earlier click is a replay and must be ignored — the
    /// fact is cardinality-one and redelivered on every resubscribe.
    #[dialog_common::test]
    fn it_ignores_a_refusal_from_an_earlier_click() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(99.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "not-synced".to_owned(),
                detail: "stale".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(read_state(&host), ShareState::Copying);
        assert!(state.borrow().pending.is_some(), "copy still pending");
        assert_eq!(
            state.borrow().pending_time,
            Some(99.0),
            "the click still in flight keeps its claim on the next refusal",
        );
    }

    /// An unrepairable refusal fails outright rather than offering a working
    /// prompt — the dialog it also opens (see the pair of tests below) has
    /// nothing the user can click through.
    #[dialog_common::test]
    fn it_fails_without_prompting_on_an_unshareable_remote() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(42.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "unshareable-remote".to_owned(),
                detail: "This space's sync server can't be shared.".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(read_state(&host), ShareState::Failed);
    }

    /// An attach that fails AFTER the user accepted the prompt still reaches
    /// them: the worker echoes the enable-sync command's own timestamp, so the
    /// refusal matches the write the confirm click opened.
    #[dialog_common::test]
    fn it_fails_when_the_attach_itself_is_refused() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(7.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "attach-failed".to_owned(),
                detail: "Could not turn on sync: offline".to_owned(),
                time: 7.0,
            },
        );

        assert_eq!(
            read_state(&host),
            ShareState::Failed,
            "the button must not spin on after the repair itself failed",
        );
        assert!(state.borrow().pending.is_none());
    }

    /// The whole point of `xyz.tonk.share/detail`: on a refusal the prompt
    /// cannot repair, the sentence still has to land somewhere a user can
    /// read it — the button's "failed" label is a static string, not this
    /// text. `handle_blocked` re-opens the enable-sync dialog to show it, and
    /// must leave the confirm button unusable: there is no attach to retry.
    #[dialog_common::test]
    fn it_shows_the_detail_and_disables_confirm_on_an_unrepairable_refusal() {
        let (dialog, detail, confirm) = dialog_stub();
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(7.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "attach-failed".to_owned(),
                detail: "Could not turn on sync: offline".to_owned(),
                time: 7.0,
            },
        );

        assert_eq!(
            detail.text_content().as_deref(),
            Some("Could not turn on sync: offline"),
            "the sentence must reach the DOM, not just the rejected clipboard promise",
        );
        assert!(
            confirm.has_attribute("disabled"),
            "there is no recovery action to offer",
        );

        dialog.remove();
    }

    /// The same, for the other unrepairable class — an invite that could
    /// never have worked, distinct from an attach that was tried and failed.
    #[dialog_common::test]
    fn it_shows_the_detail_on_an_unshareable_remote_too() {
        let (dialog, detail, confirm) = dialog_stub();
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(42.0);
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "unshareable-remote".to_owned(),
                detail: "This space's sync server can't be shared.".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(
            detail.text_content().as_deref(),
            Some("This space's sync server can't be shared.")
        );
        assert!(confirm.has_attribute("disabled"));

        dialog.remove();
    }

    /// A confirm button left disabled by an earlier unrepairable refusal must
    /// not accept a click if one somehow lands on it anyway — the guard in
    /// `install_confirm_listener` is the actual backstop, since nothing native
    /// makes a custom-element button's `disabled` attribute block dispatch.
    #[dialog_common::test]
    fn it_ignores_a_click_on_a_disabled_confirm_button() {
        let (dialog, _detail, confirm) = dialog_stub();
        let host = fresh_host();
        host.set_attribute("space", "did:key:z6Mk").expect("space");
        confirm
            .set_attribute("disabled", "")
            .expect("disable confirm");

        let mut element = TonkShare::default();
        let state = Rc::clone(&element.state);
        element.install_confirm_listener(&host);

        confirm.unchecked_ref::<HtmlElement>().click();

        assert!(
            state.borrow().pending_time.is_none(),
            "a disabled confirm button must not start a fresh attach",
        );
        assert_eq!(read_state(&host), ShareState::Idle);

        element.disconnected_callback(&host);
        dialog.remove();
    }

    /// A repairable refusal must leave (or restore) a USABLE confirm button —
    /// including after an earlier unrepairable refusal disabled it, since the
    /// same static dialog is reused across every refusal this element sees.
    #[dialog_common::test]
    fn it_re_enables_confirm_on_a_repairable_refusal_after_an_earlier_disable() {
        let (dialog, detail, confirm) = dialog_stub();
        confirm.set_attribute("disabled", "").expect("pre-disable");
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(1.0);
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "not-synced".to_owned(),
                detail: "This space only exists on this device.".to_owned(),
                time: 1.0,
            },
        );

        assert_eq!(
            detail.text_content().as_deref(),
            Some("This space only exists on this device.")
        );
        assert!(
            !confirm.has_attribute("disabled"),
            "a repairable refusal must offer a working confirm button",
        );

        dialog.remove();
    }

    /// A terminal refusal must not leave the previous repair's promise on
    /// screen beside a button that can no longer keep it.
    #[dialog_common::test]
    fn it_clears_the_action_line_on_an_unrepairable_refusal() {
        let (dialog, _detail, confirm) = dialog_stub();

        open_enable_sync_dialog(
            "This space only exists on this device.",
            Repair::for_code(BLOCKED_NOT_SYNCED),
        );
        assert!(!action_text(&dialog).is_empty(), "the repair promises one");

        open_enable_sync_dialog("This space's sync server can't be shared.", None);
        let action = action_text(&dialog);
        let confirm_label = confirm.text_content().unwrap_or_default();
        let disabled = confirm.has_attribute("disabled");
        dialog.remove();

        assert_eq!(action, "", "a terminal refusal promises nothing");
        assert_eq!(confirm_label, TERMINAL_CONFIRM);
        assert!(disabled);
    }

    /// A copy nothing ever answered gives the button back, whether or not a
    /// clipboard write was ever open.
    #[dialog_common::test]
    fn it_frees_the_button_when_a_copy_times_out() {
        let with_write = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&with_write, ShareState::Copying);

        fail_copy(&with_write, &state, "share: timed out");

        assert_eq!(read_state(&with_write), ShareState::Failed);
        assert!(state.borrow().pending.is_none());

        // The clipboard-unavailable click: `copying`, but nothing pending for
        // `settle` to consume.
        let no_write = fresh_host();
        let empty = Rc::new(RefCell::new(ShareStateCell::default()));
        set_state(&no_write, ShareState::Copying);

        fail_copy(&no_write, &empty, "share: timed out");

        assert_eq!(
            read_state(&no_write),
            ShareState::Failed,
            "a click that never opened a write must not pin the button on copying",
        );
    }

    /// The backstop is armed by the click and cancelled by the result, so a
    /// settled copy leaves no timer behind to fail it later.
    #[dialog_common::test]
    fn it_clears_the_backstop_when_a_copy_settles() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);

        arm_timeout(&host, &state);
        assert!(state.borrow().timeout.is_some(), "the backstop is armed");

        settle(&host, &state, Ok("https://tonk.xyz/@/new".to_owned()));

        assert_eq!(read_state(&host), ShareState::Copied);
        assert!(
            state.borrow().timeout.is_none(),
            "a settled copy must leave no timer to fail it after the fact",
        );
    }

    /// The prompt must appear even where no clipboard write could be opened —
    /// a browser without the promise form of `ClipboardItem`, an insecure
    /// context, a denied permission. The refusal is the thing the user needs.
    #[dialog_common::test]
    fn it_prompts_on_a_refusal_with_no_write_open() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(42.0);
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "not-synced".to_owned(),
                detail: "This space only exists on this device.".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(read_state(&host), ShareState::Blocked);
    }

    /// The same, unrepairable: it must land somewhere clickable. `settle` would
    /// cancel the backstop and then find nothing to consume, leaving the
    /// control on `copying` with nothing left able to move it.
    #[dialog_common::test]
    fn it_frees_the_button_on_an_unrepairable_refusal_with_no_write_open() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        state.borrow_mut().pending_time = Some(42.0);
        arm_timeout(&host, &state);
        set_state(&host, ShareState::Copying);

        handle_blocked(
            &host,
            &state,
            Blocked {
                code: "unshareable-remote".to_owned(),
                detail: "This space's sync server can't be shared.".to_owned(),
                time: 42.0,
            },
        );

        assert_eq!(read_state(&host), ShareState::Failed);
        assert!(
            read_state(&host).accepts_click(),
            "the user must be able to try again",
        );
    }

    /// An element re-parented mid-mint comes back clickable. `inject_children`
    /// is the only other `set_state`, and it runs once — so a host left on
    /// `copying` here would refuse every click for the rest of the session,
    /// with no write pending and no timer left to fail it.
    #[dialog_common::test]
    fn it_gives_the_button_back_when_the_element_is_disconnected_mid_mint() {
        let host = fresh_host();
        let mut element = TonkShare::default();
        let state = Rc::clone(&element.state);
        state.borrow_mut().pending_time = Some(42.0);
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        arm_timeout(&host, &state);
        set_state(&host, ShareState::Copying);

        element.disconnected_callback(&host);

        assert_eq!(read_state(&host), ShareState::Idle);
        assert!(state.borrow().pending.is_none(), "the write is abandoned");
        assert_eq!(
            state.borrow().pending_time,
            None,
            "no click is awaiting an answer any more",
        );
        assert!(state.borrow().timeout.is_none(), "no timer left running");
    }

    /// A terminal status stops the spinner, delivered as the host
    /// delivers it: a frame, not a direct call.
    #[dialog_common::test]
    fn it_fails_the_copy_on_a_terminal_status() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_reset(&host, &invite_reset_payload("invite:suspended", None));

        assert_eq!(read_state(&host), ShareState::Failed);
    }

    /// The path production takes: the row is already subscribed when the
    /// click lands, so an answer arrives as an `update` delta rather
    /// than a `reset` snapshot. `render_reset` being right proves
    /// nothing about `render_update`.
    #[dialog_common::test]
    fn it_fails_the_copy_on_a_terminal_status_delivered_as_a_delta() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_update(&host, &invite_update_payload("invite:unshareable", None));

        assert_eq!(read_state(&host), ShareState::Failed);
    }

    /// A status the control has never heard of is a failure, not a
    /// panic and not a hang.
    ///
    /// This is what lets the worker ship a new terminal status without
    /// touching the control.
    #[dialog_common::test]
    fn it_treats_an_unknown_status_as_a_failure() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_reset(&host, &invite_reset_payload("invite:something-new", None));

        assert_eq!(read_state(&host), ShareState::Failed);
    }

    /// A request in flight leaves the control spinning.
    ///
    /// The worker writes `requested` while it goes off to get an
    /// account or attach a remote; treating that as an answer would
    /// stop the button mid-share.
    #[dialog_common::test]
    fn it_keeps_waiting_while_the_request_is_open() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_reset(&host, &invite_reset_payload("invite:requested", None));

        assert_eq!(read_state(&host), ShareState::Copying);
    }

    /// A granted row settles the pending copy with its url.
    #[dialog_common::test]
    fn it_settles_the_copy_when_the_invite_is_granted() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_reset(
            &host,
            &invite_reset_payload("invite:granted", Some("https://example.com/join#seed")),
        );

        assert_eq!(read_state(&host), ShareState::Copied);
    }

    /// Granted with no url is malformed; waiting beats reporting a copy
    /// that never happened.
    #[dialog_common::test]
    fn it_keeps_waiting_when_a_granted_row_carries_no_url() {
        let host = fresh_host();
        let state = Rc::new(RefCell::new(ShareStateCell::default()));
        open_clipboard_write(Rc::clone(&state), None).expect("clipboard write opens");
        set_state(&host, ShareState::Copying);
        let behaviour = InviteStateBehaviour {
            state: Rc::clone(&state),
            current_link: Rc::new(RefCell::new(None)),
        };

        behaviour.render_reset(&host, &invite_reset_payload("invite:granted", None));

        assert_eq!(read_state(&host), ShareState::Copying);
    }

    /// The optional url really is optional: a row with only a status
    /// still reads.
    #[dialog_common::test]
    fn it_reads_a_row_with_no_url() {
        let row = invite_row("invite:requested", None);
        let parsed = read_invite_row(&row).expect("a status-only row reads");
        assert_eq!(parsed.status, "invite:requested");
        assert!(parsed.url.is_none());
    }

    #[dialog_common::test]
    fn it_reads_the_three_refusal_fields_off_a_row() {
        let row = blocked_row("not-synced", 42.0);
        assert_eq!(
            read_blocked_row(&row),
            Some(Blocked {
                code: "not-synced".to_owned(),
                detail: "no remote".to_owned(),
                time: 42.0,
            }),
        );
    }

    /// All three attributes are asserted together, so a row missing one is not
    /// a refusal — acting on it would mean acting on an unknown timestamp.
    #[dialog_common::test]
    fn it_ignores_a_partial_refusal_row() {
        let partial = Object::new();
        let fields = Object::new();
        Reflect::set(&fields, &"blocked".into(), &"not-synced".into()).expect("set blocked");
        Reflect::set(&partial, &"fields".into(), &fields).expect("set fields");
        assert_eq!(read_blocked_row(&partial.into()), None);
        assert_eq!(read_blocked_row(&JsValue::UNDEFINED), None);
    }

    /// The confirm listener lives on `document`, not on the host, so it must be
    /// torn down against `document` — removing it from the host silently fails
    /// and leaves a live closure over element state behind.
    #[dialog_common::test]
    fn it_removes_the_confirm_listener_from_the_document_on_disconnect() {
        let document = window().expect("window").document().expect("document");
        let host = fresh_host();
        host.set_attribute("space", "did:key:z6Mk").expect("space");
        let (dialog, _detail, button) = dialog_stub();
        dialog
            .set_attribute("data-share", "true")
            .expect("share ceremony");
        let remote = document.create_element("tonk-field").expect("remote field");
        remote
            .set_attribute("data-enable-sync-remote", "")
            .expect("mark remote");
        remote
            .set_attribute("value", "https://example.test/ucan/")
            .expect("remote value");
        dialog.append_child(&remote).expect("attach remote");

        let mut element = TonkShare::default();
        let state = Rc::clone(&element.state);
        element.install_confirm_listener(&host);

        button.unchecked_ref::<HtmlElement>().click();
        assert!(
            state.borrow().pending_time.is_some(),
            "the confirm click is heard through the document",
        );
        assert_eq!(
            read_state(&host),
            ShareState::Copying,
            "the confirm opens a fresh clipboard write and waits for the link",
        );
        assert!(state.borrow().pending.is_some(), "a new write is open");

        element.disconnected_callback(&host);
        state.borrow_mut().pending_time = None;
        // Back to `idle` on purpose: the handler refuses a click while a copy
        // is in flight, so leaving it on `copying` would make this pass
        // whether or not the listener was actually removed.
        set_state(&host, ShareState::Idle);
        button.unchecked_ref::<HtmlElement>().click();

        assert!(
            state.borrow().pending_time.is_none(),
            "a click after disconnect must reach nothing",
        );
        dialog.remove();
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
