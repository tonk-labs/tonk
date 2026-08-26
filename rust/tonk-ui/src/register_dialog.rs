//! The registration dialog, raised over whatever the user was reading.
//!
//! Sharing a spot needs an account, so the share control's refusal
//! (`needs-account`) asks for one here rather than sending the user to
//! `/account` and losing what they were doing. This is the top page, so
//! the ceremony can run in place: WebAuthn needs a `window` and a user
//! gesture, and neither the service worker nor the profile frame that
//! hosts the bar has both.
//!
//! Being the top page is also what makes **conditional mediation**
//! possible. The address input carries `autocomplete="username webauthn"`
//! and a non-modal `credentials.get` runs alongside it, so a returning
//! user on a new browser but the same OS picks their passkey out of the
//! input's own autofill and never types an address at all.
//!
//! What the dialog does NOT do is decide anything. It asserts
//! `account/check-email` as the user types and renders whatever the
//! `EmailStatus` overlay row says: create an account, sign in, or why
//! neither is on offer. The answer is a fact, arrived at by a command,
//! and this only draws it.

use std::cell::Cell;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;
use web_sys::{Element, HtmlElement};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_host::consumer::{self, Subscription};

thread_local! {
    /// The dialog is a singleton: a second refusal while it is up is
    /// already answered by the registration in progress.
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// The live subscription to the answer row, held for as long as the
    /// dialog is up so the frames keep arriving, and dropped on close.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    static ANSWERS: RefCell<Option<Subscription>> = const { RefCell::new(None) };
    /// The frame delegates, kept alive alongside the subscription: the
    /// host calls them by name off the element's own properties, so
    /// dropping them would silently stop delivery.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    static DELEGATES: RefCell<Vec<Closure<dyn FnMut(JsValue, JsValue)>>> =
        const { RefCell::new(Vec::new()) };
}

/// The dialog's host id, and the parts the handlers address.
const DIALOG_ID: &str = "tonk-register";
const EMAIL_INPUT: &str = "#tonk-register-email";
const SUBMIT: &str = "#tonk-register-submit";
const DISMISS: &str = "#tonk-register-dismiss";
const STATUS: &str = "#tonk-register-status";
const LEDE: &str = "#tonk-register-lede";

/// `wa-*` throughout, the same vocabulary the rest of the app uses. The
/// loader on this page auto-registers any `<wa-…>` it finds, so these
/// upgrade without anything imported here.
///
/// `autocomplete="username webauthn"` is what conditional mediation
/// binds to: the browser offers a discoverable passkey inside this
/// input's autofill. `wa-input` forwards the attribute to the inner
/// native input, which is where it has to land.
const DIALOG_HTML: &str = r#"
<style>
  /* Every state's copy is in the markup; the answer selects which of it
     shows. The one write is `data-state` on the host, so what a state
     looks like is decided here rather than by DOM writes scattered
     through the handler.

     `[data-when]` lists the states a line belongs to, matched on word
     boundaries so `pending` does not also match `pending-ceremony`. */
  #tonk-register [data-when] { display: none; }
  /* Before any answer there is no `data-state`, so the neutral label
     has to show on its own. Without this the button collapses to
     nothing and the dialog renders with no footer at all. */
  #tonk-register:not([data-state]) [data-when~="idle"],
  #tonk-register[data-state=""] [data-when~="idle"],
  #tonk-register[data-state="unregistered"] [data-when~="unregistered"],
  #tonk-register[data-state="active"] [data-when~="active"],
  #tonk-register[data-state="pending"] [data-when~="pending"],
  #tonk-register[data-state="suspended"] [data-when~="suspended"],
  #tonk-register[data-state="invalid"] [data-when~="invalid"],
  #tonk-register[data-state="unavailable"] [data-when~="unavailable"],
  #tonk-register[data-state="registering"] [data-when~="registering"],
  #tonk-register[data-state="checking"] [data-when~="checking"] {
    display: revert;
  }

  /* Nothing to submit: no answer yet, or an answer that no ceremony
     would help with. */
  #tonk-register:not([data-state]) #tonk-register-submit,
  #tonk-register[data-state=""] #tonk-register-submit,
  #tonk-register[data-state="checking"] #tonk-register-submit,
  #tonk-register[data-state="registering"] #tonk-register-submit,
  #tonk-register[data-state="suspended"] #tonk-register-submit,
  #tonk-register[data-state="invalid"] #tonk-register-submit,
  #tonk-register[data-state="unavailable"] #tonk-register-submit {
    pointer-events: none;
    opacity: .5;
  }

  #tonk-register-status {
    margin: .6rem 0 0;
    min-height: 1.2em;
    font-size: .9em;
    color: var(--wa-color-text-quiet, #9a9aa0);
  }
</style>
<wa-dialog id="tonk-register-dialog" label="Share needs a hosted copy"
           style="--width: 26rem">
  <p id="tonk-register-lede" style="margin:0 0 1rem">
    Sharing a spot means someone else can open it, so it needs a copy
    that our service hosts. Linking it to an account is what lets us
    host one.
  </p>
  <form id="tonk-register-form">
    <wa-input id="tonk-register-email" name="email" type="email"
              label="Email" placeholder="you@example.com"
              autocomplete="username webauthn" required
              autofocus></wa-input>
    <p id="tonk-register-status">
      <span data-when="checking">Checking…</span>
      <span data-when="active">You already have an account. Sign in to finish sharing.</span>
      <span data-when="pending">This address is enrolled. Sign in, then confirm your email.</span>
      <span data-when="suspended">This account is suspended, so it cannot host a copy.</span>
      <span data-when="invalid">That does not look like an email address.</span>
      <span data-when="unavailable">Could not reach the service. Check your connection.</span>
      <span data-when="registering">Setting up your account…</span>
    </p>
  </form>
  <wa-button id="tonk-register-dismiss" slot="footer" appearance="plain"
             variant="neutral">Not now</wa-button>
  <wa-button id="tonk-register-submit" slot="footer" variant="brand">
    <span data-when="idle checking unregistered invalid unavailable registering suspended">Link to an account</span>
    <span data-when="active pending">Sign in</span>
  </wa-button>
</wa-dialog>
"#;

/// Raise the dialog. A no-op while one is already up.
pub fn open() {
    if OPEN.with(|open| open.replace(true)) {
        return;
    }
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        OPEN.with(|open| open.set(false));
        return;
    };
    let (Some(body), Ok(host)) = (document.body(), document.create_element("div")) else {
        OPEN.with(|open| open.set(false));
        return;
    };
    host.set_id(DIALOG_ID);
    host.set_inner_html(DIALOG_HTML);
    let _ = body.append_child(&host);

    on_click(&host, DISMISS, close);
    on_click(&host, SUBMIT, submit);
    watch_address(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    watch_answers(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    open_when_upgraded(&host);
}

/// Turn on the footer region, then open the dialog.
///
/// `wa-dialog` renders a footer only when its `withFooter` property is
/// set; it does NOT infer one from slotted content. The FAB's dialogs
/// get away without setting it because their markup is parsed as part
/// of the document, so their children exist before the element
/// upgrades. This dialog is built by assigning `innerHTML` to a
/// detached div, where `wa-dialog` upgrades mid-parse with no children
/// yet — it rendered no footer, the `slot="footer"` buttons were
/// assigned to a slot that did not exist, and the dialog came up with
/// no buttons at all.
///
/// Deferred a task so the custom element is upgraded before either
/// property is set.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn open_when_upgraded(host: &Element) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let host = host.clone();
    let raise = Closure::<dyn FnMut()>::new(move || {
        let Some(dialog) = host.query_selector("#tonk-register-dialog").ok().flatten() else {
            return;
        };
        let _ = js_sys::Reflect::set(
            dialog.as_ref(),
            &"withFooter".into(),
            &wasm_bindgen::JsValue::TRUE,
        );
        let _ = js_sys::Reflect::set(
            dialog.as_ref(),
            &"open".into(),
            &wasm_bindgen::JsValue::TRUE,
        );
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(raise.as_ref().unchecked_ref(), 0);
    raise.forget();
}

/// Take the dialog down.
pub fn close() {
    OPEN.with(|open| open.set(false));
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        ANSWERS.with(|held| {
            if let Some(mut subscription) = held.borrow_mut().take() {
                subscription.cancel();
            }
        });
        DELEGATES.with(|held| held.borrow_mut().clear());
    }
    if let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
    {
        host.remove();
    }
}

/// Ask about the address as it is typed, so the dialog can offer
/// sign-in rather than a ceremony that would fail at the end.
///
/// Debounced here rather than in the worker: a question with no network
/// answer belongs in the component, and the command should not run per
/// keystroke. `wa-input`'s own `required` / `type="email"` handle the
/// format, so this only asks about addresses the browser already
/// considers well-formed.
fn watch_address(host: &Element) {
    let Some(input) = host.query_selector(EMAIL_INPUT).ok().flatten() else {
        return;
    };
    let listener = Closure::<dyn FnMut()>::new(move || {
        schedule_check();
    });
    let _ = input.add_event_listener_with_callback("input", listener.as_ref().unchecked_ref());
    listener.forget();
}

thread_local! {
    /// The pending debounce timer, so a keystroke replaces the last one
    /// rather than queueing another lookup behind it.
    static PENDING: Cell<i32> = const { Cell::new(0) };
}

/// How long the address has to stop changing before it is asked about.
const IDLE_MS: i32 = 400;

fn schedule_check() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let previous = PENDING.with(|pending| pending.replace(0));
    if previous != 0 {
        window.clear_timeout_with_handle(previous);
    }
    let fire = Closure::<dyn FnMut()>::new(move || {
        PENDING.with(|pending| pending.set(0));
        check_now();
    });
    if let Ok(handle) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        fire.as_ref().unchecked_ref(),
        IDLE_MS,
    ) {
        PENDING.with(|pending| pending.set(handle));
    }
    fire.forget();
}

/// Dispatch `account/check-email` for whatever is typed now.
fn check_now() {
    let Some(email) = address() else {
        clear_state();
        return;
    };
    if !is_plausible(&email) {
        // Not worth asking about, so there is no answer to show. Drop
        // the state rather than leaving the last address's answer up:
        // an answer about something the user has since edited away from
        // reads as an answer about what is typed now.
        clear_state();
        return;
    }
    // No "Checking…" painted here: the handler asserts `checking` for
    // this address before it makes the lookup, and the row is what the
    // dialog renders. Painting it here too would be a second source of
    // truth that disagrees with the row while the lookup runs.
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = crate::api::transact_profile(check_email_claim(&email)).await {
            tonk_common::log!("register: could not ask about the address: {error}");
            clear_state();
        }
    });
}

/// Drop the rendered answer, back to the dialog's opening state.
fn clear_state() {
    if let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
    {
        let _ = host.remove_attribute("data-state");
    }
}

/// The subscription tag for the answer row.
const ANSWER_TAG: &str = "tonk-register-answer";

/// The overlay row the worker writes each answer to.
const ANSWER_SUBJECT: &str = "state:email-status";

/// The query for the answer row: the two raw attributes
/// `account/check-email` writes, bound to the one entity it writes them
/// to.
///
/// Raw attribute URIs rather than a concept name, so a profile seeded
/// from an older `profile.yaml` cannot break the read.
pub(crate) fn answer_query_body() -> String {
    serde_json::json!({
        "predicate": { "with": {
            "address": {
                "the": "xyz.tonk.email-status/address",
                "as": "Text", "cardinality": "one"
            },
            "state": {
                "the": "xyz.tonk.email-status/state",
                "as": "Text", "cardinality": "one"
            }
        } },
        "terms": {
            "this": ANSWER_SUBJECT,
            "address": { "?": { "name": "address" } },
            "state": { "?": { "name": "state" } }
        }
    })
    .to_string()
}

/// Subscribe to the answer row and render it as it arrives.
///
/// Without this the dialog only ever writes: `check_now` asserts the
/// command and paints "Checking…", and nothing puts the answer back on
/// screen, so the wait never ends. The worker's answer is a fact on the
/// profile overlay, and this is the read half of that loop.
///
/// The host is installed on this page (`tonk_host::install()` in
/// `bin/ui.rs`), so a plain `consumer::subscribe` works. The routing
/// context is the fixed profile branch — the overlay row is written to
/// `main@profile:tonk` — rather than anything derived from an
/// attribute.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn watch_answers(host: &Element) {
    let _ = host.set_attribute("with", "main@profile:tonk");

    let mut delegates = Vec::new();
    for method in ["reset", "update"] {
        let target = host.clone();
        let is_delta = method == "update";
        let delegate =
            Closure::<dyn FnMut(JsValue, JsValue)>::new(move |payload: JsValue, _opts: JsValue| {
                let _ = &target;
                if let Some(answer) = read_answer(&payload, is_delta) {
                    show_answer(&answer);
                }
            });
        if js_sys::Reflect::set(host.as_ref(), &method.into(), delegate.as_ref()).is_err() {
            return;
        }
        delegates.push(delegate);
    }
    DELEGATES.with(|held| *held.borrow_mut() = delegates);

    // Subscribe only once the service worker is up. `tonk-subscribe`
    // is dispatched synchronously and the host answers it inline, so a
    // subscribe fired during cold start fails outright ("host did not
    // write detail.subscription") and, with nothing to retry it, the
    // dialog would wait on frames that never come.
    let host = host.clone();
    wasm_bindgen_futures::spawn_local(async move {
        tonk_host::ready::wait().await;
        let body = answer_query_body();
        let Ok(query) = js_sys::JSON::parse(&body) else {
            return;
        };
        // The dialog may already be gone by the time the gate opens.
        if !host.is_connected() {
            return;
        }
        match consumer::subscribe(&host, &query, Some(&ANSWER_TAG.into())) {
            Ok(subscription) => ANSWERS.with(|held| *held.borrow_mut() = Some(subscription)),
            Err(error) => {
                tonk_common::log!("register: could not watch for the answer: {error:?}");
            }
        }
    });
}

/// An answer as the dialog reads it: which address it is about, and what
/// the service said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Answer {
    /// The address the answer is about.
    pub(crate) address: String,
    /// One of the `state::*` strings the worker writes.
    pub(crate) state: String,
}

/// Pull the newest answer out of a frame.
///
/// A `reset` carries a bare array of the current rows; an `update`
/// carries `{ asserted, retracted }`. Both attributes are
/// cardinality-one on a single entity, so the last asserted row is the
/// current answer, and a bare retract says nothing new.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn read_answer(payload: &JsValue, is_delta: bool) -> Option<Answer> {
    let rows = if is_delta {
        js_sys::Reflect::get(payload, &"asserted".into()).ok()?
    } else {
        payload.clone()
    };
    let rows = js_sys::Array::from(&rows);
    let last = rows.get(rows.length().checked_sub(1)?);
    if last.is_undefined() || last.is_null() {
        return None;
    }
    let fields = js_sys::Reflect::get(&last, &"fields".into()).ok()?;
    let read = |name: &str| {
        js_sys::Reflect::get(&fields, &name.into())
            .ok()
            .and_then(|value| value.as_string())
    };
    Some(Answer {
        address: read("address")?,
        state: read("state")?,
    })
}

/// Render an answer, ignoring one about an address the user has since
/// edited away from.
///
/// The row carries the address for exactly this reason: answers arrive
/// out of order behind a debounce, and a late answer about two
/// keystrokes ago must not overwrite the current one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn show_answer(answer: &Answer) {
    let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
    else {
        return;
    };
    let typed = address().unwrap_or_default();
    if !typed.eq_ignore_ascii_case(answer.address.trim()) {
        return;
    }
    // The one write. Every visible consequence of the answer — which
    // status line shows, what the button reads, whether it can be
    // pressed — is a `[data-when]` rule in `DIALOG_HTML` keyed on this
    // attribute.
    let _ = host.set_attribute("data-state", &answer.state);
}

/// Whether an address is worth asking the service about.
///
/// The browser's own `type="email"` validity is the real gate; this is
/// the cheap structural check that keeps a half-typed address from
/// becoming a lookup.
pub(crate) fn is_plausible(email: &str) -> bool {
    let trimmed = email.trim();
    match trimmed.rsplit_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && trimmed.len() <= 254
        }
        None => false,
    }
}

/// The `account/check-email` claim, in the shape the command decodes.
pub(crate) fn check_email_claim(email: &str) -> serde_json::Value {
    claim("Ask whether an address is registered.", email)
}

/// The `account/register` claim.
pub(crate) fn register_claim(email: &str) -> serde_json::Value {
    claim("Register an account for this address.", email)
}

/// Both commands read one field from the same read-path, so they differ
/// only in the descriptor's description — which is what mints a distinct
/// command entity for each.
fn claim(description: &str, email: &str) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": description,
                        "with": {
                            "email": {
                                "the": "dom.event.current-target.elements.email/value",
                                "as": "Text"
                            }
                        }
                    }
                },
                "parameters": { "email": email }
            }
        }]
    })
}

/// Start the ceremony for the typed address, then finish the share it
/// interrupted.
fn submit() {
    let Some(email) = address().filter(|email| is_plausible(email)) else {
        set_status("Enter the address you want to use.");
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        // Dispatch and stop. A successful transact means the COMMAND was
        // accepted, not that an account exists: the handler asks the page
        // to run a WebAuthn ceremony and returns without awaiting it.
        //
        // Treating `Ok` as "registered" is what made this jump straight
        // to "the share link is on its way" with no passkey prompt ever
        // shown. What happens next arrives as facts — the handler
        // publishes `registering`, and enrollment lands an
        // `AccountCustomer` row — and the subscription renders them.
        if let Err(error) = crate::api::transact_profile(register_claim(&email)).await {
            tonk_common::log!("register: could not start registration: {error}");
            clear_state();
        }
    });
}

/// Run the signup ceremony the worker asked for.
///
/// Reached from `custody_relay`'s `CreateAccount` branch: the worker
/// cannot create an account (WebAuthn needs a `window` and a user
/// gesture) so it asks the page, and this is the page's half.
///
/// **Not implemented yet.** The ceremony currently lives inline in
/// `/account`'s create-submit handler, interleaved with that panel's
/// `set_busy` / `show_error` / `set_mode` calls, so there is nothing to
/// call from here. Extracting it is the `account/ceremony-complete`
/// work in `plan/system-page-commands.md`: the page runs the ceremony
/// and asserts a command carrying its output, and a handler applies it
/// — which also stops the panel and this dialog from keeping two
/// copies of the same logic.
///
/// Until then this says so rather than silently doing nothing. A
/// dropped request is what made the dialog report "the share link is on
/// its way" with no ceremony ever run.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn run_signup_ceremony() {
    tonk_common::log!(
        "register: the worker asked for a signup ceremony, which is not wired yet \
         (see plan/system-page-commands.md, account/ceremony-complete)"
    );
    set_status("Account creation is not available here yet.");
}

/// The `tonk:enable-sync` claim that finishes an interrupted share:
/// attach where the account syncs, and mint.
///
/// No remote is named. The worker resolves it from the account's own
/// recorded provider, which is the point of registering at all.
///
/// `time` is passed in rather than read here so the shape stays pure
/// and testable off-browser; the caller supplies the click's moment,
/// which is what makes each dispatch a distinct transient.
pub(crate) fn enable_sync_claim(space: &str, time: f64) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Attach a sync remote to a spot, and share it.",
                        "with": {
                            "time": { "the": "dom.event/time-stamp", "as": "Float" },
                            "space": { "the": "xyz.tonk.enable-sync/space", "as": "Entity" },
                            "share": { "the": "xyz.tonk.enable-sync/share", "as": "Entity" },
                            "marker": {
                                "the": "dom.event.current-target.dataset/enable-sync",
                                "as": "Entity"
                            }
                        }
                    }
                },
                "parameters": {
                    "time": time,
                    "space": space,
                    "share": "tonk:share",
                    "marker": "tonk:enable-sync"
                }
            }
        }]
    })
}

/// What is typed in the address field.
fn address() -> Option<String> {
    let input = web_sys::window()?
        .document()?
        .query_selector(EMAIL_INPUT)
        .ok()??;
    js_sys::Reflect::get(input.as_ref(), &"value".into())
        .ok()?
        .as_string()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The line under the input, where an answer or a wait shows.
fn set_status(text: &str) {
    if let Some(slot) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(STATUS).ok().flatten())
    {
        slot.set_text_content(Some(text));
    }
}

/// What the guest asked for: why it could not share, and what it was
/// trying to share.
#[derive(Debug, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Request {
    /// The refusal class that raised the dialog.
    #[serde(default)]
    pub reason: String,
    /// The spot the interrupted click was sharing, so it can be
    /// finished once an account exists.
    #[serde(default)]
    pub space: String,
}

/// Parse the payload a guest forwards, tolerating a bare reason string
/// from an older guest.
pub fn parse_request(payload: &str) -> Request {
    serde_json::from_str(payload).unwrap_or_else(|_| Request {
        reason: payload.to_owned(),
        space: String::new(),
    })
}

/// Remember what to share once registration finishes.
fn remember_space(space: &str) {
    if space.is_empty() {
        return;
    }
    if let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
    {
        let _ = host.set_attribute(PENDING_SHARE, space);
    }
}

/// The spot a finished registration should go on to share.
pub(crate) fn pending_share() -> Option<String> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
        .and_then(|host| host.get_attribute(PENDING_SHARE))
        .filter(|space| !space.is_empty())
}

/// Where the interrupted share's target is parked while the ceremony
/// runs.
const PENDING_SHARE: &str = "data-pending-share";

/// Re-word the dialog for the refusal that raised it, and remember what
/// the interrupted click was trying to share.
///
/// `needs-activation` is not a signup: the account exists and the link
/// is unopened, so the dialog says so instead of offering to create a
/// second one.
pub fn describe(payload: &str) {
    let request = parse_request(payload);
    remember_space(&request.space);
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if request.reason != tonk_worker_api::share::BLOCKED_NEEDS_ACTIVATION {
        return;
    }
    if let Some(lede) = document.query_selector(LEDE).ok().flatten() {
        lede.set_text_content(Some(
            "Your account is waiting on its email. Open the link we sent, then share again.",
        ));
    }
    if let Some(dialog) = document
        .query_selector("#tonk-register-dialog")
        .ok()
        .flatten()
    {
        let _ = dialog.set_attribute("label", "Confirm your email to share");
    }
}

/// Wire `selector`'s click, inside the gesture so a ceremony started
/// from it still counts as user-activated.
fn on_click(host: &Element, selector: &str, handler: impl Fn() + 'static) {
    let Some(button) = host.query_selector(selector).ok().flatten() else {
        return;
    };
    let listener = Closure::<dyn FnMut()>::new(handler);
    let _ = button.dyn_ref::<HtmlElement>().map(|button| {
        button.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref())
    });
    listener.forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog asks about addresses the browser would accept, and
    /// keeps half-typed ones out of the lookup.
    #[dialog_common::test]
    fn it_asks_only_about_plausible_addresses() {
        assert!(is_plausible("jsmith@example.com"));
        assert!(is_plausible("  jsmith@example.com  "), "trimmed first");

        assert!(!is_plausible(""), "nothing typed yet");
        assert!(!is_plausible("jsmith"), "still typing the local part");
        assert!(!is_plausible("jsmith@"), "no domain yet");
        assert!(!is_plausible("jsmith@example"), "no dot yet");
        assert!(!is_plausible("@example.com"), "no local part");
        assert!(!is_plausible("jsmith@.com"), "domain starts with a dot");
        assert!(!is_plausible("jsmith@example."), "domain ends with a dot");
    }

    /// The guest's ask carries why it could not share and what it was
    /// sharing, so the dialog can word the prompt and then finish the
    /// click that raised it.
    #[dialog_common::test]
    fn it_reads_the_reason_and_the_interrupted_share() {
        let request = parse_request(r#"{"reason":"needs-account","space":"did:key:z6Mk"}"#);
        assert_eq!(request.reason, "needs-account");
        assert_eq!(request.space, "did:key:z6Mk");
    }

    /// A payload that is not the structured shape is taken as a bare
    /// reason: an older guest still gets its prompt worded, it just has
    /// no share to resume.
    #[dialog_common::test]
    fn it_falls_back_to_a_bare_reason() {
        let request = parse_request("needs-activation");
        assert_eq!(request.reason, "needs-activation");
        assert!(request.space.is_empty(), "nothing to resume");
    }

    /// The resume claim asks for the attach AND the mint, and names no
    /// remote: the worker resolves that from the account that was just
    /// created, which is why registering was worth doing.
    #[dialog_common::test]
    fn it_resumes_the_share_by_asking_for_attach_and_mint() {
        let claim = enable_sync_claim("did:key:z6Mk", 1234.0).to_string();
        assert!(claim.contains("xyz.tonk.enable-sync/space"));
        assert!(claim.contains("did:key:z6Mk"));
        assert!(
            claim.contains("xyz.tonk.enable-sync/share"),
            "the mint is what produces the link the click wanted",
        );
        assert!(
            !claim.contains("enable-sync/remote"),
            "naming a remote here would re-derive what the account records",
        );
    }

    /// Both claims name the read-path the commands decode, and differ
    /// so each mints its own command entity.
    #[dialog_common::test]
    fn it_builds_claims_the_commands_decode() {
        let check = check_email_claim("jsmith@example.com").to_string();
        assert!(check.contains("dom.event.current-target.elements.email/value"));
        assert!(check.contains("jsmith@example.com"));

        let register = register_claim("jsmith@example.com").to_string();
        assert!(register.contains("dom.event.current-target.elements.email/value"));
        assert_ne!(
            check, register,
            "the descriptions differ, so the two commands are distinct transients",
        );
    }
}
