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
const ACTION: &str = "#tonk-register-action";
const DISMISS: &str = "#tonk-register-dismiss";
const STATUS: &str = "#tonk-register-status";
const EMAIL_ROW: &str = "#tonk-register-email-row";

/// `wa-*` throughout, the same vocabulary the rest of the app uses. The
/// loader on this page auto-registers any `<wa-…>` it finds, so these
/// upgrade without anything imported here.
///
/// `autocomplete="username webauthn"` is what conditional mediation
/// binds to: the browser offers a discoverable passkey inside this
/// input's autofill. `wa-input` forwards the attribute to the inner
/// native input, which is where it has to land.
const DIALOG_HTML: &str = r##"
<div id="tonk-register-dim" class="tonk-dim"></div>
<div id="tonk-register-cluster" class="tonk-cluster" role="dialog" aria-modal="true"
     aria-labelledby="tonk-register-head">
  <div class="ocol">
    <div class="ostack" id="tonk-register-stack">
      <div class="m-head mblk" id="tonk-register-head">link an account</div>
      <div class="orow mblk" id="tonk-register-email-row">
        <span class="k">email</span>
        <span class="v"><input class="ed" id="tonk-register-email" type="email"
              inputmode="email" enterkeyhint="go" autocomplete="username webauthn"
              aria-label="email" placeholder="you@example.com"><i class="cur"
              aria-hidden="true"></i></span>
      </div>
      <!-- Unfolds once the address is committed and the lookup answers:
           "create a passkey" for an address nobody has, "log in with your
           passkey" for one that is taken. Which of the two is the whole
           reason the address is checked before any ceremony runs. -->
      <button class="obtn pre" id="tonk-register-action" hidden></button>
    </div>
    <div class="oexp mblk">
      <p id="tonk-register-status" aria-live="polite"></p>
    </div>
    <button class="ghost" id="tonk-register-dismiss">
      <span aria-hidden="true">&#9666;</span> back to space</button>
  </div>
</div>
"##;

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
    host.set_class_name("tonk-ceremony");
    host.set_inner_html(DIALOG_HTML);
    let _ = body.append_child(&host);

    on_click(&host, DISMISS, close);
    on_click(&host, ACTION, submit);
    watch_address(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    commit_on_enter(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    focus_on_row_click(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    watch_answers(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    open_when_upgraded(&host);
}

/// Raise the cluster: dim the page, show the column, seat the cursor.
///
/// Deferred a task so the appended markup has been laid out — the dim
/// transitions from its resting opacity, and a class set in the same
/// frame as the insert would jump straight to the end state.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn open_when_upgraded(host: &Element) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let host = host.clone();
    let raise = Closure::<dyn FnMut()>::new(move || {
        if let Ok(Some(dim)) = host.query_selector("#tonk-register-dim") {
            // `add_1`, not `set_class_name`: the element already
            // carries `tonk-dim`, which is what styles it at all.
            let _ = dim.class_list().add_1("on");
        }
        focus_address(&host);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(raise.as_ref().unchecked_ref(), 0);
    raise.forget();
}

/// Seat the cursor in the address field.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn focus_address(host: &Element) {
    let Some(field) = host.query_selector(EMAIL_INPUT).ok().flatten() else {
        return;
    };
    if let Some(element) = field.dyn_ref::<HtmlElement>() {
        let _ = element.focus();
    }
}

/// Unfold a row into the stack: appended folded, then released a frame
/// later so the height transition has something to animate from.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn unfold(row: &Element) {
    row.remove_attribute("hidden").ok();
    row.class_list().add_1("pre").ok();
    let Some(window) = web_sys::window() else {
        return;
    };
    let row = row.clone();
    let release = Closure::<dyn FnMut()>::new(move || {
        let _ = row.class_list().remove_1("pre");
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        release.as_ref().unchecked_ref(),
        16,
    );
    release.forget();
}

/// Settle a row into its record: the noun stays, the value becomes ink,
/// and the block cursor goes.
///
/// A settled row is the step's receipt. It stays on screen so what you
/// have already answered is legible above what you are answering now,
/// which is the whole reason the ceremony is a stack rather than a
/// sequence of replaced screens.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn settle(row: &Element, noun: &str, value: &str) {
    row.set_inner_html("");
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    for (class, text) in [("k", noun), ("v", value)] {
        if let Ok(span) = document.create_element("span") {
            span.set_class_name(class);
            span.set_text_content(Some(text));
            let _ = row.append_child(&span);
        }
    }
}

/// Add a row to the stack, folded, ready to unfold.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn add_row(host: &Element, id: &str, noun: &str, value: &str) -> Option<Element> {
    let document = web_sys::window()?.document()?;
    let stack = host.query_selector("#tonk-register-stack").ok()??;
    let row = document.create_element("div").ok()?;
    row.set_id(id);
    row.set_class_name("orow mblk pre");
    settle(&row, noun, value);
    // Before the action row, so the button stays at the foot of the stack.
    let action = host.query_selector(ACTION).ok().flatten();
    match action {
        Some(action) => {
            let _ = stack.insert_before(&row, Some(&action));
        }
        None => {
            let _ = stack.append_child(&row);
        }
    }
    unfold(&row);
    Some(row)
}

/// Whether the answer has named a step the user can take.
///
/// The action row is revealed by [`show_answer`] once the address
/// lookup resolves, and hidden otherwise — so its visibility IS the
/// question "is there something to run yet".
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn action_is_offered() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(ACTION).ok().flatten())
        .is_some_and(|action| !action.has_attribute("hidden"))
}

/// Clicking anywhere in a row seats the cursor in its editor.
///
/// The editor is a bare span with a block cursor for a seat, so without
/// this the only target is the glyph itself. In the study the row owns
/// the click for the same reason.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn focus_on_row_click(host: &Element) {
    let Some(row) = host.query_selector(EMAIL_ROW).ok().flatten() else {
        return;
    };
    let target = host.clone();
    let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        focus_address(&target);
    });
    let _ = row.add_event_listener_with_callback("click", listener.as_ref().unchecked_ref());
    listener.forget();
}

/// Enter in the address field runs the step the answer named.
///
/// The action row is revealed by the lookup, not by this — but the row
/// is a shortcut, not a gate: someone who has typed their address and
/// pressed Enter has said what they came to say, so the ceremony starts
/// without a second click. Enter before an answer has arrived does
/// nothing, because there is no step to run yet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn commit_on_enter(host: &Element) {
    let Some(field) = host.query_selector(EMAIL_INPUT).ok().flatten() else {
        return;
    };
    let listener =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Enter" {
                return;
            }
            // The field sits in no form, but Enter still submits on
            // some platforms; stop it reaching anything else.
            event.prevent_default();
            // Only once the lookup has named a step. The action row is
            // hidden until then, and starting a ceremony on Enter alone
            // means one fires the moment a half-typed address happens to
            // look plausible — before anyone has said which of create or
            // sign in they meant.
            if action_is_offered() {
                submit();
            }
        });
    let _ = field.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref());
    listener.forget();
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
    let _ = host.set_attribute("data-state", &answer.state);
    set_status(status_for(&answer.state));

    // The action row unfolds only once the lookup has named a step, and
    // says which one. Before that there is nothing to offer: an address
    // nobody has asked about could be either branch, and guessing wrong
    // runs a creation ceremony against an account that already exists.
    let Some(action) = host.query_selector(ACTION).ok().flatten() else {
        return;
    };
    match action_label(&answer.state) {
        Some(label) => {
            action.set_text_content(Some(label));
            unfold(&action);
        }
        None => {
            let _ = action.set_attribute("hidden", "");
        }
    }
}

/// What the action row offers for an answer, or `None` when it offers
/// nothing.
///
/// This is the routing the address lookup exists for: someone who
/// already has an account is signing in, not making a second one, and
/// sending them through a creation ceremony leaves an orphan passkey in
/// their authenticator.
pub(crate) fn action_label(state: &str) -> Option<&'static str> {
    use tonk_schema::email_state as answer;
    match state {
        answer::UNREGISTERED => Some("create a passkey"),
        answer::ACTIVE | answer::PENDING => Some("log in with your passkey"),
        // Checking, or an answer nothing can act on.
        _ => None,
    }
}

/// The narrator's line for an answer.
pub(crate) fn status_for(state: &str) -> &'static str {
    use tonk_schema::email_state as answer;
    match state {
        answer::CHECKING => "Checking…",
        answer::UNREGISTERED => {
            "A passkey replaces a password. Your device saves it to itself, a browser \
             profile, or a password manager — Tonk never keeps a copy."
        }
        answer::ACTIVE => "You already have an account. Log in to finish sharing.",
        answer::PENDING => "This address is enrolled. Log in, then confirm your email.",
        answer::SUSPENDED => "This account is suspended, so it cannot host a copy.",
        answer::INVALID => "That does not look like an email address.",
        answer::UNAVAILABLE => "Could not reach the service. Check your connection.",
        answer::PENDING_CEREMONY => "Setting up your account…",
        _ => "",
    }
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
    let mut claim = claim("Register an account for this address.", email);
    // The marker is what makes this a REGISTRATION and not a lookup.
    // Both carry `{this, email}`, and decode does not consider concept
    // identity — so without it every keystroke's `check-email` also
    // decoded as `account/register`, and a passkey prompt appeared
    // while the user was still typing.
    claim["claims"][0]["application"]["predicate"]["concept"]["with"]["marker"] = serde_json::json!({
        "the": "dom.event.current-target.dataset/register-account",
        "as": "Entity"
    });
    claim["claims"][0]["application"]["parameters"]["marker"] =
        serde_json::json!(tonk_schema::command::RegisterAccount::MARKER);
    claim
}

/// The two commands read one field from the same read-path. A distinct
/// description mints a distinct command entity, but that alone does not
/// keep them apart at decode — see [`register_claim`].
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

/// Run the signup ceremony for the typed address.
///
/// The page's half of `account/register`: the worker cannot create an
/// account (WebAuthn needs a `window` and a user gesture) so it asks,
/// and this answers.
///
/// The ceremony itself lives in `account.rs`, shared with `/account`'s
/// create button rather than copied — two passkey ceremonies would
/// drift, and the half that drifted would leave an orphan credential in
/// someone's authenticator.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn run_signup_ceremony() {
    let Some(email) = address().filter(|email| is_plausible(email)) else {
        set_status("Enter the address you want to use.");
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        match crate::account::run_account_ceremony(&email, set_status).await {
            Ok(()) => {
                // What happens next arrives as facts: `AccountCustomer`
                // appears at enrollment and gains a provider once the
                // emailed link is opened. The dialog is subscribed, so
                // it renders them rather than being told.
                set_status("Check your email to finish setting up your account.");
            }
            Err(error) => {
                tonk_common::log!("register: the ceremony did not complete: {error}");
                set_status(&error);
            }
        }
    });
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
    set_status("Your account is waiting on its email. Open the link we sent, then share again.");
    if let Some(head) = document
        .query_selector("#tonk-register-head")
        .ok()
        .flatten()
    {
        head.set_text_content(Some("confirm your email to share"));
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

    /// A lookup must not also decode as a registration.
    ///
    /// `CheckEmail` and `RegisterAccount` are both `{this, email}`, and
    /// decode does not consider concept identity — so before the marker,
    /// every keystroke's lookup ALSO fired the register handler, and a
    /// passkey prompt appeared while the user was still typing.
    #[dialog_common::test]
    fn it_marks_a_registration_so_a_lookup_cannot_be_mistaken_for_one() {
        let lookup = check_email_claim("someone@example.com");
        let register = register_claim("someone@example.com");

        let marker_of = |claim: &serde_json::Value| {
            claim["claims"][0]["application"]["parameters"]["marker"].clone()
        };
        assert!(
            marker_of(&lookup).is_null(),
            "a lookup carries no marker: {lookup}",
        );
        assert_eq!(
            marker_of(&register),
            serde_json::json!(tonk_schema::command::RegisterAccount::MARKER),
            "a registration is the only one that does",
        );

        // The declared field must be there too, or the marker never
        // becomes a fact the handler can match on.
        let declared =
            &register["claims"][0]["application"]["predicate"]["concept"]["with"]["marker"];
        assert_eq!(declared["as"], "Entity", "a `:`-bearing value is an entity");
    }

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
