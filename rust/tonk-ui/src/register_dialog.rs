//! The registration dialog, raised over whatever the user was reading.
//!
//! Sharing a space needs an account, so the share control's refusal
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

use std::cell::{Cell, RefCell};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use web_sys::HtmlButtonElement;
use web_sys::{Element, HtmlDialogElement, HtmlElement};

use crate::user_error::{self, AccountAction};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_host::consumer::{self, Subscription};

thread_local! {
    /// The dialog is a singleton: a second refusal while it is up is
    /// already answered by the registration in progress.
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// The control that raised the singleton. Native modal focus is restored
    /// explicitly because the host is removed, rather than merely closed.
    static RETURN_FOCUS: RefCell<Option<ReturnFocus>> = const { RefCell::new(None) };
    /// Set synchronously at the event boundary, before any WebAuthn or network
    /// future can yield and admit a second click/Enter activation.
    static ACTION_PENDING: Cell<bool> = const { Cell::new(false) };
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
    /// The newest answer the dialog has been told about, held so a
    /// change to the typed address can be answered from what is already
    /// known rather than only from the next frame.
    ///
    /// Two things make the next frame an unreliable place to learn this.
    /// The answer row is a singleton, so asking about an address it
    /// already names is a write that changes nothing — and an
    /// established subscriber is sent a frame only when the poll
    /// reported a change, so the answer never arrives again. And the
    /// frame that DID carry it may have landed before anything was
    /// typed, when it was an answer about nothing on screen.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    static ANSWER: RefCell<Option<Answer>> = const { RefCell::new(None) };
}

enum ReturnFocus {
    Direct(HtmlElement),
    Guest(Option<Box<dyn FnOnce()>>),
}

/// The dialog's host id, and the parts the handlers address.
const DIALOG_ID: &str = "tonk-register";
const COMMITTED_EMAIL_ATTR: &str = "data-register-email";
/// Which ceremony ran: `signup` created the account, `login` reached an
/// existing one. Only signup's finish asks for a display name.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const CEREMONY_KIND_ATTR: &str = "data-register-ceremony";
const EMAIL_INPUT: &str = "#tonk-register-email";
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ACTION: &str = "#tonk-register-action";
const DISMISS: &str = "#tonk-register-dismiss";
const STATUS: &str = "#tonk-register-status";
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const EMAIL_ROW: &str = "#tonk-register-email-row";
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const NAME_ROW: &str = "#tonk-register-name-row";
/// The row standing while the emailed link is out. It is the SAME row
/// that becomes `email · verified`: activation confirms the address it
/// is already about, so a second row would leave the ceremony claiming
/// to await a confirmation that had arrived.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const CONFIRM_ROW: &str = "#tonk-register-confirm-row";

/// `wa-*` throughout, the same vocabulary the rest of the app uses. The
/// loader on this page auto-registers any `<wa-…>` it finds, so these
/// upgrade without anything imported here.
///
/// `autocomplete="username webauthn"` is what conditional mediation
/// binds to: the browser offers a discoverable passkey inside this
/// input's autofill. `wa-input` forwards the attribute to the inner
/// native input, which is where it has to land.
const DIALOG_HTML: &str = r##"
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
    <p id="tonk-register-status" aria-live="polite">Enter your email address. We’ll tell you whether to create a passkey or sign in.</p>
  </div>
  <button class="ghost" id="tonk-register-dismiss">
    <span aria-hidden="true">&#9666;</span> back to space</button>
</div>
"##;

/// Raise the dialog. A no-op while one is already up.
pub fn open() {
    open_with_return(None);
}

/// Raise the dialog for a sealed-guest request and invoke `restore` only after
/// the native modal has closed and its top-page host has been removed.
pub fn open_with_return_focus(restore: impl FnOnce() + 'static) {
    open_with_return(Some(Box::new(restore)));
}

fn open_with_return(guest_restore: Option<Box<dyn FnOnce()>>) {
    if OPEN.with(|open| open.replace(true)) {
        return;
    }
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        OPEN.with(|open| open.set(false));
        return;
    };
    let return_focus = match guest_restore {
        Some(restore) => Some(ReturnFocus::Guest(Some(restore))),
        None => document
            .active_element()
            .and_then(|element| element.dyn_into::<HtmlElement>().ok())
            .map(ReturnFocus::Direct),
    };
    RETURN_FOCUS.with(|held| *held.borrow_mut() = return_focus);
    let (Some(body), Ok(host)) = (document.body(), document.create_element("dialog")) else {
        OPEN.with(|open| open.set(false));
        RETURN_FOCUS.with(|held| *held.borrow_mut() = None);
        return;
    };
    host.set_id(DIALOG_ID);
    host.set_class_name("tonk-ceremony tonk-cluster");
    let _ = host.set_attribute("aria-labelledby", "tonk-register-head");
    let _ = host.set_attribute("aria-describedby", "tonk-register-status");
    host.set_inner_html(DIALOG_HTML);
    let _ = body.append_child(&host);

    on_click(&host, DISMISS, close);
    let cancel = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        close();
    });
    let _ = host.add_event_listener_with_callback("cancel", cancel.as_ref().unchecked_ref());
    cancel.forget();
    contain_tab_focus(&host);
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
        if let Some(dialog) = host.dyn_ref::<HtmlDialogElement>()
            && !dialog.open()
        {
            let _ = dialog.show_modal();
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
        .and_then(|action| action.dyn_into::<HtmlButtonElement>().ok())
        .is_some_and(|action| !action.has_attribute("hidden") && !action.disabled())
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

    // And anywhere else in the cluster. The address field is only the
    // FIRST place a step is taken from; once its row settles the focus
    // moves on, and Enter answered nothing for every step after it —
    // "copy share link" had to be clicked. The action row is the step
    // being offered wherever the cursor happens to be, so Enter runs it.
    let cluster = host.clone();
    let anywhere =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Enter" {
                return;
            }
            // A row taking input commits itself; its own handler decides
            // what Enter means there.
            let typing = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|element| element.matches("input").unwrap_or(false));
            if typing || !action_is_offered() {
                return;
            }
            event.prevent_default();
            submit();
        });
    let _ = cluster.add_event_listener_with_callback("keydown", anywhere.as_ref().unchecked_ref());
    anywhere.forget();
}

/// The event this dialog fires on the top page once the ceremony has
/// given this browser an account.
///
/// The ceremony moved into the cluster, but the account panel under it
/// still renders from a status it read when it was connected — so
/// `/settings` went on offering to link an account that now exists, and
/// `/settings/link` went on refusing the approval it was told to
/// register for. The panel is not the one that learns this any more, so
/// it is told: the cluster and the panel share a document, which makes a
/// DOM event the whole channel. It carries nothing — the panel re-reads
/// the status, the same way it already re-derives itself on `popstate`,
/// rather than being handed a state to render.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) const ACCOUNT_CHANGED: &str = "tonk:account-changed";

/// Say that this browser's account changed.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn announce_account_change() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(event) = web_sys::Event::new(ACCOUNT_CHANGED) else {
        return;
    };
    let _ = window.dispatch_event(&event);
}

/// Take the dialog down.
pub fn close() {
    OPEN.with(|open| open.set(false));
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        finish_action();
        ANSWERS.with(|held| {
            if let Some(mut subscription) = held.borrow_mut().take() {
                subscription.cancel();
            }
        });
        DELEGATES.with(|held| held.borrow_mut().clear());
        // The answer belongs to the dialog that asked, so the next one
        // starts from nothing rather than from what this one was told.
        ANSWER.with(|held| *held.borrow_mut() = None);
    }
    if let Some(host) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
    {
        if let Some(dialog) = host.dyn_ref::<HtmlDialogElement>()
            && dialog.open()
        {
            dialog.close();
        }
        host.remove();
    }
    let Some(return_focus) = RETURN_FOCUS.with(|held| held.borrow_mut().take()) else {
        return;
    };
    let Some(window) = web_sys::window() else {
        restore_focus(return_focus);
        return;
    };
    // The native dialog performs its own close-focus settlement after the
    // `cancel` listener returns. Restore on the next task so that settlement
    // cannot overwrite either a top-page opener or a sealed guest's iframe.
    let restore = Closure::once(move || restore_focus(return_focus));
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(restore.as_ref().unchecked_ref(), 0);
    restore.forget();
}

fn restore_focus(return_focus: ReturnFocus) {
    match return_focus {
        ReturnFocus::Direct(opener)
            if opener.is_connected() && !opener.matches(":disabled").unwrap_or(false) =>
        {
            let _ = opener.focus();
        }
        ReturnFocus::Guest(Some(restore)) => restore(),
        _ => {}
    }
}

/// Chrome can move focus to `BODY` when Tab crosses the end of a native
/// modal. Guard only the two boundaries; ordinary movement and Escape stay
/// under the platform dialog.
fn contain_tab_focus(host: &Element) {
    let dialog = host.clone();
    let listener =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Tab" {
                return;
            }
            let focusables = registration_focusables(&dialog);
            let (Some(first), Some(last)) = (focusables.first(), focusables.last()) else {
                return;
            };
            let active = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element());
            let target = if event.shift_key()
                && active
                    .as_ref()
                    .is_some_and(|active| first.is_same_node(Some(active.as_ref())))
            {
                Some(last)
            } else if !event.shift_key()
                && active
                    .as_ref()
                    .is_some_and(|active| last.is_same_node(Some(active.as_ref())))
            {
                Some(first)
            } else {
                None
            };
            if let Some(target) = target {
                event.prevent_default();
                let _ = target.focus();
            }
        });
    let _ = host.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref());
    listener.forget();
}

fn registration_focusables(host: &Element) -> Vec<HtmlElement> {
    let Ok(candidates) = host.query_selector_all("button,input,select,textarea,a[href],[tabindex]")
    else {
        return Vec::new();
    };
    let mut focusables = Vec::new();
    for index in 0..candidates.length() {
        let Some(element) = candidates
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        if element.closest("[hidden]").ok().flatten().is_some()
            || element.matches(":disabled").unwrap_or(false)
            || element
                .get_attribute("tabindex")
                .and_then(|value| value.parse::<i32>().ok())
                .is_some_and(|tabindex| tabindex < 0)
            || (element.tag_name() == "INPUT"
                && element.get_attribute("type").as_deref() == Some("hidden"))
        {
            continue;
        }
        if let Ok(element) = element.dyn_into::<HtmlElement>() {
            focusables.push(element);
        }
    }
    focusables
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
        // Answer from what is already known, without waiting for the
        // lookup to come back: an address the dialog has an answer for
        // is answered now, and the lookup behind the debounce only
        // confirms it. Without this the dialog waits on a frame that a
        // repeat answer never produces.
        #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
        render_known_answer();
    });
    let _ = input.add_event_listener_with_callback("input", listener.as_ref().unchecked_ref());
    listener.forget();
}

/// Render the newest answer the dialog holds against what is typed now.
///
/// [`show_answer`] is what decides whether it is still about the typed
/// address, so an answer about something the user has edited away from
/// renders nothing.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn render_known_answer() {
    let held = ANSWER.with(|held| held.borrow().clone());
    if let Some(answer) = held {
        show_answer(&answer);
    }
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
            set_status(&user_error::diagnostic(
                AccountAction::CheckEmail,
                &error.to_string(),
            ));
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

/// Watch for the account to finish registering.
///
/// `AccountCustomer` is durable on profile main and carries the
/// provider the service named in its activation receipt — the remote
/// this account's spaces sync to. Activation commits it, so it
/// replicates and every session sees it, including this one.
///
/// That durability is the whole reason to read this rather than the
/// address-lookup row: the emailed link opens in a different tab with
/// its own worker session, and an overlay fact written there never
/// Whether this account has already activated.
///
/// Asked once, at the end of a sign-in, to decide whether the ceremony can
/// close or has to wait for the emailed link. The steady-state answer
/// still arrives as a fact through [`await_activation`]'s subscription —
/// this is only the initial read, for the device that just signed in and
/// has nothing on screen yet.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn account_is_activated() -> bool {
    // The registration fact carries the provider from enrollment; the
    // activation fact is what says the customer confirmed. Presence is the
    // whole signal, so an absent row means "still waiting".
    crate::api::customer_state()
        .await
        .ok()
        .and_then(|state| {
            state
                .get("status")
                .and_then(|value| value.as_str().map(str::to_owned))
        })
        .is_some_and(|status| status == "Active")
}

/// crosses.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn await_activation(host: &Element) {
    let mut delegates = Vec::new();
    for method in ["reset", "update"] {
        let is_delta = method == "update";
        let delegate =
            Closure::<dyn FnMut(JsValue, JsValue)>::new(move |payload: JsValue, _opts: JsValue| {
                if account_is_active(&payload, is_delta) {
                    finish_ceremony();
                }
            });
        if js_sys::Reflect::set(host.as_ref(), &method.into(), delegate.as_ref()).is_err() {
            activation_watch_failed("could not install the activation listener");
            return;
        }
        delegates.push(delegate);
    }
    DELEGATES.with(|held| held.borrow_mut().extend(delegates));

    let Ok(query) = js_sys::JSON::parse(&account_query_body()) else {
        activation_watch_failed("could not prepare the activation query");
        return;
    };
    match consumer::subscribe(host, &query, Some(&ACCOUNT_TAG.into())) {
        Ok(subscription) => ANSWERS.with(|held| *held.borrow_mut() = Some(subscription)),
        Err(error) => {
            activation_watch_failed(&format!("activation subscription failed: {error:?}"));
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn activation_watch_failed(detail: &str) {
    tonk_common::log!("register: {detail}");
    set_status(&user_error::diagnostic(
        AccountAction::WatchActivation,
        detail,
    ));
    set_action(RETURN_TO_SPACE, true);
}

/// Whether a failed ceremony is the ordinary wait for an emailed link.
///
/// The one question the outcome arm asks, as a function so a test can
/// ask it too: the arm itself only builds DOM rows.
///
/// A second device signing in before anyone opens the link is not a
/// failure -- it is the same wait signing up ends in, and it ends the
/// same way. Reported as an error it read "we couldn't finish logging
/// you in. check your connection and try again", which named the wrong
/// problem and offered the wrong remedy to someone whose account was one
/// click away.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn awaits_confirmation(denial: Option<&tonk_identity::custody::CustodyDenial>) -> bool {
    matches!(
        denial,
        Some(tonk_identity::custody::CustodyDenial::AwaitingActivation)
    )
}

/// The frame field whose presence means the account is served.
///
/// Named once because two things must agree on it: the subscription that
/// asks for it ([`account_query_body`]) and the reader that looks for it
/// ([`account_is_active`]). They drifted apart once — the query moved from
/// a `status` string to the activation fact and the reader kept comparing
/// `status == "Active"`, so every frame read as not-yet-active and the
/// ceremony waited forever on an account that had already activated.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
const ACTIVE_FIELD: &str = "activated_at";

/// Whether a frame carries an activated account.
///
/// Presence, not comparison: the row resolves only when the account has an
/// activation fact, so a frame arriving at all is the answer. There is no
/// status string to match against.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn account_is_active(payload: &JsValue, is_delta: bool) -> bool {
    let rows = if is_delta {
        js_sys::Reflect::get(payload, &"asserted".into()).unwrap_or(JsValue::UNDEFINED)
    } else {
        payload.clone()
    };
    js_sys::Array::from(&rows).iter().any(|row| {
        js_sys::Reflect::get(&row, &"fields".into())
            .ok()
            .and_then(|fields| js_sys::Reflect::get(&fields, &ACTIVE_FIELD.into()).ok())
            .is_some_and(|value| !value.is_undefined() && !value.is_null())
    })
}

/// The account's ACTIVATION row: when it activated, and the provider its
/// spaces sync to. Its presence is what makes an account served — there is
/// no status string to compare against.
#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
fn account_query_body() -> String {
    serde_json::json!({
        "predicate": { "with": {
            "activated_at": {
                "the": "xyz.tonk.account/activated-at", "as": "UnsignedInteger",
                "cardinality": "one"
            }
        } },
        "terms": {
            "this": { "?": { "name": "account" } },
            "activated_at": { "?": { "name": "activated_at" } },
        }
    })
    .to_string()
}

/// The subscription tag for the answer row.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ANSWER_TAG: &str = "tonk-register-answer";
/// Distinguishes the activation watch from the address lookup.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ACCOUNT_TAG: &str = "tonk-register-account";

/// The overlay row the worker writes each answer to.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ANSWER_SUBJECT: &str = "state:email-status";

/// The query for the answer row: the two raw attributes
/// `account/check-email` writes, bound to the one entity it writes them
/// to.
///
/// Raw attribute URIs rather than a concept name, so a profile seeded
/// from an older `profile.yaml` cannot break the read.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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
                    ANSWER.with(|held| *held.borrow_mut() = Some(answer.clone()));
                    show_answer(&answer);
                }
            });
        if js_sys::Reflect::set(host.as_ref(), &method.into(), delegate.as_ref()).is_err() {
            registration_watch_failed("could not install the account-options listener");
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
            registration_watch_failed("could not prepare the account-options query");
            return;
        };
        // The dialog may already be gone by the time the gate opens.
        if !host.is_connected() {
            return;
        }
        match consumer::subscribe(&host, &query, Some(&ANSWER_TAG.into())) {
            Ok(subscription) => ANSWERS.with(|held| *held.borrow_mut() = Some(subscription)),
            Err(error) => {
                registration_watch_failed(&format!(
                    "account-options subscription failed: {error:?}"
                ));
            }
        }
    });
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn registration_watch_failed(detail: &str) {
    tonk_common::log!("register: {detail}");
    set_status(&user_error::diagnostic(
        AccountAction::LoadRegistration,
        detail,
    ));
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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

    // Activation is what turns "check your email" into the next step.
    // It arrives as a fact — `record_customer_status` refreshes this row
    // when the emailed link is opened — so the cluster advances without
    // polling and without the user touching anything.
    if answer.state == tonk_schema::email_state::ACTIVE
        && host
            .query_selector("#tonk-register-passkey-row")
            .ok()
            .flatten()
            .is_some()
    {
        finish_ceremony();
        return;
    }
    // A lookup replay can land while WebAuthn or the share handoff is still
    // pending. It may refresh the host's state, but it must not re-enable the
    // action the user already accepted.
    if ACTION_PENDING.with(Cell::get) {
        return;
    }
    set_status(status_for(&answer.state));

    // The action row unfolds only once the lookup has named a step, and
    // says which one. Before that there is nothing to offer: an address
    // nobody has asked about could be either branch, and guessing wrong
    // runs a creation ceremony against an account that already exists.
    match action_label(&answer.state) {
        Some(label) => set_action(label, true),
        None => {
            if let Ok(Some(action)) = host.query_selector(ACTION) {
                let _ = action.set_attribute("hidden", "");
            }
        }
    }
}

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
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

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
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
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn submit() {
    if !begin_action() {
        return;
    }
    // The action row means different things at different steps, and its
    // label is which one — the same word the person just read.
    let label = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector(ACTION).ok().flatten())
        .map(|action| action.text_content().unwrap_or_default())
        .unwrap_or_default();
    match label.trim() {
        COPY_LINK => copy_the_share_link(),
        RETURN_TO_SPACE => close(),
        "" => finish_action(),
        _ => run_signup_ceremony(),
    }
}

/// Mint the invite and copy it: the close, once an account exists.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const COPY_LINK: &str = "copy share link";

/// And the step after it. The ceremony ends where it interrupted
/// something, so it offers the way back rather than leaving the person
/// to find the dismiss themselves — which also keeps the whole flow
/// runnable on Enter, the way every step before it is.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const RETURN_TO_SPACE: &str = "return to space";

/// Mint the invite the share was interrupted for, and copy it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn copy_the_share_link() {
    let Some(space) = pending_share() else {
        finish_action();
        return;
    };
    set_action("copying link…", false);
    wasm_bindgen_futures::spawn_local(async move {
        let claim = enable_sync_claim(&space, js_sys::Date::now());
        if let Err(error) = crate::api::transact_profile(claim).await {
            tonk_common::log!("register: could not finish the share: {error}");
            set_status("Could not create the link. Share the space again.");
            set_action(COPY_LINK, true);
            return;
        }
        // The link arrives as a fact on the space's own branch, so this
        // waits for the row rather than for a response body.
        match await_invite_link(&space).await {
            Some(link) => match write_to_clipboard(&link).await {
                Ok(()) => {
                    set_status("You can use the copied link to invite someone into a space.");
                    set_action(RETURN_TO_SPACE, true);
                    focus_action();
                }
                Err(error) => {
                    tonk_common::log!("register: could not copy the invite link: {error}");
                    set_status(&user_error::diagnostic(AccountAction::CopyInvite, &error));
                    set_action(COPY_LINK, true);
                }
            },
            None => {
                set_status("The link is taking longer than expected. Share the space again.");
                set_action(COPY_LINK, true);
            }
        }
    });
}

/// Wait for the space's invite row to carry a url.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn await_invite_link(space: &str) -> Option<String> {
    for _ in 0..60 {
        if let Some(link) = read_invite_link(space).await {
            return Some(link);
        }
        wait_ms(500).await;
    }
    None
}

/// Read the invite url off the space's `tonk:invite` row.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn read_invite_link(space: &str) -> Option<String> {
    let body = serde_json::json!({
        "predicate": { "with": {
            "status": {
                "the": "xyz.tonk.invite/status", "as": "Entity", "cardinality": "one"
            },
            "url": {
                "the": "xyz.tonk.invite/url", "as": "Text",
                "cardinality": "one", "optional": true
            }
        } },
        "terms": {
            "this": space,
            "status": { "?": { "name": "status" } },
            "url": { "?": { "name": "url" } }
        }
    });
    let endpoint = format!(
        "{}/api/repository/{}/branch/main/query",
        crate::api::origin(),
        space
    );
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .ok()?;
    let rows: serde_json::Value = response.json().await.ok()?;
    rows.as_array()?
        .iter()
        .find_map(|row| row["fields"]["url"].as_str())
        .map(str::to_owned)
}

/// Sleep, for a poll that has no fact to wait on.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn wait_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Put `text` on the clipboard.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn write_to_clipboard(text: &str) -> Result<(), String> {
    let Some(clipboard) = web_sys::window().map(|window| window.navigator().clipboard()) else {
        return Err("the clipboard is unavailable".to_owned());
    };
    wasm_bindgen_futures::JsFuture::from(clipboard.write_text(text))
        .await
        .map(|_| ())
        .map_err(|error| format!("clipboard write failed: {error:?}"))
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
        finish_action();
        return;
    };
    // Which ceremony is the answer's to choose, not this function's.
    // Running creation for an address that already has an account tries
    // to save a second root over the first and fails with
    // `409 a different account is already signed in on this profile` —
    // after the user has already been through a passkey prompt.
    let state = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
        .and_then(|host| host.get_attribute("data-state"))
        .unwrap_or_default();
    let existing = matches!(
        state.as_str(),
        tonk_schema::email_state::ACTIVE | tonk_schema::email_state::PENDING
    );

    // The address is answered; settle its row so it reads as a record
    // and the step in front of you is the only one taking input.
    if let Some(host) = host_element() {
        let _ = host.set_attribute(COMMITTED_EMAIL_ATTR, &email);
        // Which ceremony this is, for the finish: only REGISTRATION
        // ever asks for a display name. A login reaches an account that
        // already belongs to someone — asking again on every device
        // would have each answer overwrite the last.
        let _ = host.set_attribute(
            CEREMONY_KIND_ATTR,
            if existing { "login" } else { "signup" },
        );
    }
    settle_named_row(EMAIL_ROW, "email", &email);
    // While the platform holds the ceremony, the action row says so
    // rather than looking clickable. It blinks rather than spinning:
    // attention is earned by blinking, never by hue.
    set_action("waiting for your device", false);

    wasm_bindgen_futures::spawn_local(async move {
        let outcome = if existing {
            crate::account::run_login_ceremony(set_status).await
        } else {
            crate::account::run_account_ceremony(&email, set_status).await
        };
        match outcome {
            // The account exists and registered, but nobody has opened
            // the emailed link yet. That is not a failure to report: it
            // is the same wait signing up ends in, so it ends the same
            // way -- the row that says what is outstanding, and the
            // subscription that closes the ceremony when activation
            // lands from whichever device opens the link.
            //
            // Reported as an error once, and it read "we couldn't finish
            // logging you in. check your connection and try again" for
            // someone whose connection was fine and whose account was
            // one click away, with no way to tell from the screen that
            // waiting was all it needed.
            Err(error) if awaits_confirmation(error.denial.as_ref()) => {
                tonk_common::log!("register: the account awaits its email confirmation");
                hide_action();
                add_row(
                    &host_element().unwrap_or_else(|| unreachable!()),
                    "tonk-register-confirm-row",
                    "email",
                    "awaiting confirmation",
                );
                set_status("Open the confirmation link in your email to finish signing in.");
                // No subscription here: the login did not complete, so
                // this profile holds no account and no fact will ever
                // arrive on it. The service is the only party that knows
                // when the link is opened, so ask it — the address
                // lookup — until it says so, then hand the passkey step
                // back. One tap resumes: the ceremony needs a fresh
                // assertion anyway, since its derivation handles were
                // dropped with the failed handoff.
                poll_lookup_until_active(email.clone());
            }
            Err(error) => {
                tonk_common::log!("register: the ceremony did not complete: {error}");
                set_status(&user_error::ceremony(
                    if existing {
                        AccountAction::LogIn
                    } else {
                        AccountAction::CreateAccount
                    },
                    &error,
                ));
                // Back to something clickable: a control left mid-flight
                // refuses every later attempt.
                set_action(
                    if existing {
                        "log in with your passkey"
                    } else {
                        "create a passkey"
                    },
                    true,
                );
            }
            Ok(()) => {
                // The account exists as of this line — created, or
                // signed in to — which is the whole of what the panel
                // under the cluster has to be told. Said here rather
                // than on the way out: the ceremony can end with the
                // cluster still up, waiting on an emailed link, and a
                // panel that shows the account only once the dialog is
                // dismissed is a panel that disagrees with the page.
                announce_account_change();
                // The passkey is the step's record, named by the device
                // that holds it — "Chrome on macOS" is more use than a
                // credential id nobody can act on.
                hide_action();
                add_row(
                    &host_element().unwrap_or_else(|| unreachable!()),
                    "tonk-register-passkey-row",
                    "passkey",
                    &crate::device_name::current(),
                );
                // `existing` means an account exists for this address —
                // NOT that it is activated. Signing in on a second device
                // while the first has not opened the emailed link is the
                // ordinary case, and closing the ceremony there stranded
                // it: the account branch cannot hydrate until the customer
                // confirms, so the device sat behind a failure message
                // with nothing to act on.
                //
                // Both paths wait the same way. The row says what is
                // outstanding, and the subscription closes the ceremony
                // when activation lands — from the emailed link on any
                // device, since what it waits on is a fact that syncs.
                if existing && account_is_activated().await {
                    // Already activated: nothing to wait for.
                    finish_ceremony();
                } else {
                    // What happens next arrives as facts: the emailed
                    // link lands `AccountCustomer`, and the subscription
                    // renders it. Nothing here polls for it.
                    // A row of its own, so the step in front of you is
                    // visible as a row and not only as a sentence.
                    add_row(
                        &host_element().unwrap_or_else(|| unreachable!()),
                        "tonk-register-confirm-row",
                        "email",
                        "awaiting confirmation",
                    );
                    set_status(if existing {
                        "Open the confirmation link in your email to finish signing in."
                    } else {
                        "Click the confirmation link we sent to your email."
                    });
                    if let Some(host) = host_element() {
                        await_activation(&host);
                        // The activation signal is the account sweep's
                        // own pull being served, so drive the sweeps at
                        // the ceremony's cadence: confirmation should
                        // land here seconds after the link is opened,
                        // not whenever the background heartbeat next
                        // comes around.
                        nudge_sync_while_waiting();
                    }
                }
            }
        }
    });
}

/// Ask the worker to drain sync every few seconds while the ceremony
/// waits on the emailed link.
///
/// The sweep that is finally served records the activation fact in the
/// same pass, and the subscription flips the ceremony — this loop only
/// controls how soon that sweep runs. Stops with the wait: a settled
/// row, a dismissed cluster.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn nudge_sync_while_waiting() {
    /// Fast enough that confirming feels answered, slow enough not to
    /// hammer a drain that also runs on its own heartbeat.
    const EVERY: i32 = 3_000;

    wasm_bindgen_futures::spawn_local(async move {
        loop {
            let Some(host) = host_element() else {
                return;
            };
            let still_waiting = host
                .query_selector(&format!("{CONFIRM_ROW} .v"))
                .ok()
                .flatten()
                .and_then(|value| value.text_content())
                .is_some_and(|value| value.trim() == "awaiting confirmation");
            if !still_waiting {
                return;
            }
            if let Err(error) = crate::api::kick_sync().await {
                tonk_common::log!("register: sync nudge did not run: {error}");
            }
            sleep(EVERY).await;
        }
    });
}

/// Ask the address lookup about `email` until it answers `active`, then
/// offer the passkey step again.
///
/// The waiting sign-in's driver. The registering browser waits on a fact
/// its own account sweep writes, but a browser whose login was refused
/// holds no account: nothing local will ever change, and only the
/// service knows when the emailed link is opened. Each round transacts
/// the same `account/check-email` the typing path uses, the worker asks
/// the service and rewrites the overlay answer, and the answer
/// subscription repaints `data-state` — which is what this loop reads.
///
/// Stops when the cluster goes away, when something else moved the
/// ceremony past waiting, and on the flip itself: the resume needs a
/// fresh passkey assertion (the refused ceremony's derivation handles
/// are gone), so the flip re-arms the action row rather than asserting
/// on its own — WebAuthn wants a gesture, and a surprise passkey prompt
/// from a background timer reads as an attack.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn poll_lookup_until_active(email: String) {
    /// Long enough not to hammer the service through a wait measured in
    /// however long it takes to reach an inbox, short enough that
    /// confirming on the other device feels answered rather than stuck.
    const EVERY: i32 = 4_000;

    wasm_bindgen_futures::spawn_local(async move {
        // Rounds the lookup has answered `active` while the worker's
        // parked login was still finishing. The tap is the FALLBACK —
        // the worker kept the assertion's handles and completes the
        // login itself — so the offer waits a few rounds for the silent
        // finish before asking for a gesture the flow may not need.
        let mut served_rounds = 0u32;
        loop {
            // The ceremony is gone (dismissed, or finished): nothing
            // left to resume.
            let Some(host) = host_element() else {
                return;
            };
            // Something else moved the ceremony past waiting: the row is
            // gone, or no longer reads as the wait this loop drives.
            let still_waiting = host
                .query_selector(&format!("{CONFIRM_ROW} .v"))
                .ok()
                .flatten()
                .and_then(|value| value.text_content())
                .is_some_and(|value| value.trim() == "awaiting confirmation");
            if !still_waiting {
                return;
            }
            // The worker finished the login it parked: the account is
            // linked, and nothing needs a second passkey tap. Finish
            // the ceremony the way a completed sign-in would.
            if matches!(
                crate::api::account_status().await,
                Ok(tonk_worker_api::AccountStatus::Registered { .. })
            ) {
                settle_named_row(CONFIRM_ROW, "email", "verified");
                if host
                    .query_selector("#tonk-register-passkey-row")
                    .ok()
                    .flatten()
                    .is_none()
                {
                    add_row(
                        &host,
                        "tonk-register-passkey-row",
                        "passkey",
                        &crate::device_name::current(),
                    );
                }
                finish_ceremony();
                return;
            }
            if host.get_attribute("data-state").as_deref() == Some(tonk_schema::email_state::ACTIVE)
            {
                served_rounds += 1;
                if served_rounds >= 3 {
                    // The silent finish did not land — a restarted
                    // worker dropped the handles — so the way back in
                    // is one tap and a fresh assertion.
                    settle_named_row(CONFIRM_ROW, "email", "verified");
                    set_status("Your email is confirmed. Log in with your passkey to continue.");
                    set_action("log in with your passkey", true);
                    focus_action();
                    return;
                }
            }
            if let Err(error) = crate::api::transact_profile(check_email_claim(&email)).await {
                // A missed round is the next round's problem: the wait
                // is already open-ended, and the loop is the only thing
                // that can end it.
                tonk_common::log!("register: activation lookup did not run: {error}");
            }
            sleep(EVERY).await;
        }
    });
}

/// Resolve after `millis`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn sleep(millis: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// The cluster host, when it is on screen.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn host_element() -> Option<Element> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(DIALOG_ID))
}

/// Turn a row that was taking input into its record.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn settle_named_row(selector: &str, noun: &str, value: &str) {
    let Some(row) = host_element().and_then(|host| host.query_selector(selector).ok().flatten())
    else {
        return;
    };
    settle(&row, noun, value);
}

/// Show the action row with `label`, enabled or not.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn set_action(label: &str, ready: bool) {
    let Some(action) = host_element().and_then(|host| host.query_selector(ACTION).ok().flatten())
    else {
        return;
    };
    action.set_text_content(Some(label));
    if ready {
        let _ = action.class_list().remove_1("wait");
        finish_action();
    } else {
        let _ = action.class_list().add_1("wait");
        ACTION_PENDING.with(|pending| pending.set(true));
        if let Some(button) = action.dyn_ref::<HtmlButtonElement>() {
            button.set_disabled(true);
            let _ = button.set_attribute("aria-busy", "true");
        }
    }
    unfold(&action);
}

/// Claim the currently offered action before its handler can yield.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn begin_action() -> bool {
    if ACTION_PENDING.with(|pending| pending.replace(true)) {
        return false;
    }
    let Some(action) = host_element()
        .and_then(|host| host.query_selector(ACTION).ok().flatten())
        .and_then(|action| action.dyn_into::<HtmlButtonElement>().ok())
    else {
        ACTION_PENDING.with(|pending| pending.set(false));
        return false;
    };
    action.set_disabled(true);
    let _ = action.set_attribute("aria-busy", "true");
    true
}

/// Offer the next attempt after a retryable outcome.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn finish_action() {
    ACTION_PENDING.with(|pending| pending.set(false));
    if let Some(action) = host_element()
        .and_then(|host| host.query_selector(ACTION).ok().flatten())
        .and_then(|action| action.dyn_into::<HtmlButtonElement>().ok())
    {
        action.set_disabled(false);
        let _ = action.remove_attribute("aria-busy");
    }
}

/// Put the cursor on the offered step.
///
/// Enter runs whatever the action row offers, but only if something in
/// the cluster has focus — after a row settles, the field that had it is
/// gone. Seating focus on the step itself is what lets the whole
/// ceremony be taken with Enter, which is how every step before the
/// close already worked.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn focus_action() {
    let Some(action) = host_element().and_then(|host| host.query_selector(ACTION).ok().flatten())
    else {
        return;
    };
    if let Some(element) = action.dyn_ref::<HtmlElement>() {
        let _ = element.focus();
    }
}

/// Fold the action row away.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn hide_action() {
    if let Some(action) = host_element().and_then(|host| host.query_selector(ACTION).ok().flatten())
    {
        let _ = action.set_attribute("hidden", "");
    }
}

/// The account is ready: ask for a name, then close out.
///
/// Called once activation lands — from the ceremony directly when
/// signing in (already activated), or from the `EmailStatus`
/// subscription when the emailed link is opened.
///
/// The name is asked for once per ACCOUNT, not once per device: signing
/// in reaches an account that was already named when it was created, so
/// the existing name is shown as a record rather than asked for again —
/// retyping it would overwrite what every other device already shows.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn finish_ceremony() {
    let Some(host) = host_element() else {
        return;
    };
    // Already past this point — a second activation frame must not
    // unfold a second name row.
    if host.query_selector(NAME_ROW).ok().flatten().is_some() {
        return;
    }
    // The row that was awaiting the link is the one that resolves; the
    // address row above it keeps saying which address. Where no
    // confirmation row stands — signing in with an existing passkey
    // never raises one — there is nothing to settle.
    settle_named_row(CONFIRM_ROW, "email", "verified");

    // Only REGISTRATION asks. A login reaches an account that already
    // belongs to someone: it may be named already, or its naming may
    // still be waiting in the signup ceremony open on the device that
    // registered — either way, asking here would have every device's
    // answer overwrite the last one's.
    let signing_in = host.get_attribute(CEREMONY_KIND_ATTR).as_deref() == Some("login");
    wasm_bindgen_futures::spawn_local(async move {
        // The summary's display name is the CHOSEN one — the
        // `AccountDisplayName` fact, absent until the registering
        // ceremony answers the question — not the roster's, which falls
        // back to a petname and so cannot tell a named account from a
        // fresh one. Best-effort: an unreadable summary only means the
        // record row is not shown.
        let named = crate::api::account_summary()
            .await
            .ok()
            .and_then(|summary| summary.display_name)
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        let Some(host) = host_element() else {
            return;
        };
        // A second frame can race the summary read past the guard above.
        if host.query_selector(NAME_ROW).ok().flatten().is_some() {
            return;
        }
        match named {
            Some(name) => {
                add_row(
                    &host,
                    NAME_ROW.trim_start_matches('#'),
                    "display name",
                    &name,
                );
                conclude("Your account is ready.");
            }
            None if signing_in => conclude("You're signed in."),
            None => ask_for_name(&host),
        }
    });
}

/// Unfold the display-name input and focus it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn ask_for_name(host: &Element) {
    set_status("What should we call you?");

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(stack) = host.query_selector("#tonk-register-stack").ok().flatten() else {
        return;
    };
    let Ok(row) = document.create_element("div") else {
        return;
    };
    row.set_id(NAME_ROW.trim_start_matches('#'));
    row.set_class_name("orow mblk pre");
    row.set_inner_html(
        r##"<span class="k">display name</span>
            <span class="v"><input class="ed" id="tonk-register-name" type="text"
                  enterkeyhint="go" aria-label="display name"><i class="cur"
                  aria-hidden="true"></i></span>"##,
    );
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
    commit_name_on_enter(host);
    if let Some(field) = host
        .query_selector("#tonk-register-name")
        .ok()
        .flatten()
        .and_then(|field| field.dyn_into::<HtmlElement>().ok())
    {
        let _ = field.focus();
    }
}

/// The name row commits on Enter, and the close follows.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn commit_name_on_enter(host: &Element) {
    let Some(field) = host.query_selector("#tonk-register-name").ok().flatten() else {
        return;
    };
    let listener =
        Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Enter" {
                return;
            }
            event.prevent_default();
            let name = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| {
                    document
                        .query_selector("#tonk-register-name")
                        .ok()
                        .flatten()
                })
                .and_then(|field| js_sys::Reflect::get(field.as_ref(), &"value".into()).ok())
                .and_then(|value| value.as_string())
                .unwrap_or_default()
                .trim()
                .to_owned();
            if name.is_empty() {
                return;
            }
            settle_named_row(NAME_ROW, "display name", &name);
            offer_the_link(&name);
        });
    let _ = field.add_event_listener_with_callback("keydown", listener.as_ref().unchecked_ref());
    listener.forget();
}

/// The close: the thing the share was for.
///
/// Only when a share raised this. Opened on its own there is nothing to
/// go back to, so the cluster simply says the account is ready.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn offer_the_link(name: &str) {
    wasm_bindgen_futures::spawn_local({
        let name = name.to_owned();
        async move {
            let status = match crate::api::transact_profile(profile_rename_claim(&name)).await {
                Ok(()) => "Your account is ready.".to_owned(),
                Err(error) => {
                    tonk_common::log!("register: could not record the display name: {error}");
                    user_error::diagnostic(
                        AccountAction::SaveInitialDisplayName,
                        &error.to_string(),
                    )
                }
            };
            conclude(&status);
        }
    });
}

/// Close the finished ceremony out: the interrupted share's link when
/// one is pending, the way back to the space otherwise.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn conclude(status: &str) {
    match pending_share() {
        Some(_) => {
            set_status(status);
            set_action(COPY_LINK, true);
        }
        None => {
            // Opened on its own there is no link to hand over, but
            // there is still a way out to offer: hiding the action left
            // the ceremony finished and standing, with only the back
            // arrow to leave by.
            set_status(status);
            set_action(RETURN_TO_SPACE, true);
            focus_action();
        }
    }
}

/// The `profile/rename` claim, in the shape the seeded descriptor
/// decodes.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn profile_rename_claim(name: &str) -> serde_json::Value {
    serde_json::json!({
        "claims": [{
            "op": "assert",
            "application": {
                "predicate": {
                    "kind": "transient",
                    "concept": {
                        "description": "Rename the signed-in member (set their display name).",
                        "with": {
                            "name":   { "the": "dom.event.current-target/value", "as": "Text" },
                            "marker": { "the": "dom.event.current-target.dataset/rename", "as": "Entity" }
                        }
                    }
                },
                "parameters": { "name": name, "marker": "tonk:profile" }
            }
        }]
    })
}

#[cfg(any(test, all(target_arch = "wasm32", target_os = "unknown")))]
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
                        "description": "Attach a sync remote to a space, and share it.",
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
    let document = web_sys::window()?.document()?;
    let value = match document.query_selector(EMAIL_INPUT).ok().flatten() {
        Some(input) => js_sys::Reflect::get(input.as_ref(), &"value".into())
            .ok()?
            .as_string(),
        None => document
            .get_element_by_id(DIALOG_ID)
            .and_then(|host| host.get_attribute(COMMITTED_EMAIL_ATTR)),
    };
    value
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
    /// The space the interrupted click was sharing, so it can be
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

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
/// The space a finished registration should go on to share.
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

    use tonk_identity::custody::CustodyDenial;

    /// A second device that signs in before the link is opened waits,
    /// rather than being told it failed.
    ///
    /// The service answers this refusal for the ordinary case of signing
    /// in on a phone while the laptop's confirmation email sits unopened.
    /// The dialog reported it as a failure -- "check your connection and
    /// try again" -- for someone whose connection was fine and whose
    /// account was one click from ready, with nothing on screen to say
    /// that waiting was the whole remedy.
    #[dialog_common::test]
    fn it_waits_when_the_account_needs_its_email_confirmed() {
        assert!(super::awaits_confirmation(Some(
            &CustodyDenial::AwaitingActivation
        )));
    }

    /// Every other refusal is still a failure to report.
    ///
    /// Showing "open the link in your email" for a suspension or an
    /// unprovisioned space would park someone on a wait that no email
    /// ends.
    #[dialog_common::test]
    fn it_reports_refusals_no_email_would_clear() {
        assert!(!super::awaits_confirmation(Some(
            &CustodyDenial::Suspended("unpaid".to_owned())
        )));
        assert!(!super::awaits_confirmation(Some(
            &CustodyDenial::NotProvisioned("nobody pays".to_owned())
        )));
        assert!(!super::awaits_confirmation(Some(&CustodyDenial::Other(
            "something else".to_owned()
        ))));
        // A dismissed passkey prompt never reached the service.
        assert!(!super::awaits_confirmation(None));
    }

    /// The subscription asks for the field the reader looks for.
    ///
    /// These are two halves of one contract and nothing but this test holds
    /// them together. They came apart once: the query moved from a `status`
    /// string to the activation fact, the reader kept comparing `status ==
    /// "Active"`, and because a field that is not requested is simply absent
    /// from every frame, `account_is_active` answered false forever. The
    /// account activated, the service said so, and the ceremony sat on
    /// "awaiting confirmation" until the tab was reloaded.
    ///
    /// Nothing failed loudly, which is why it reached a browser: a
    /// subscription that resolves and a reader that finds nothing look
    /// identical from the outside.
    #[dialog_common::test]
    fn it_reads_the_field_its_subscription_asks_for() {
        let body: serde_json::Value =
            serde_json::from_str(&account_query_body()).expect("the query body is JSON");

        assert!(
            body["predicate"]["with"][ACTIVE_FIELD].is_object(),
            "the subscription must request `{ACTIVE_FIELD}`, which is what \
             `account_is_active` reads; got {}",
            body["predicate"]["with"],
        );
        assert!(
            body["terms"][ACTIVE_FIELD].is_object(),
            "`{ACTIVE_FIELD}` must be bound in `terms` or it never reaches a frame",
        );
    }

    /// The button's label and the ceremony it runs must agree.
    ///
    /// They are chosen from the same answer, so a mismatch means
    /// someone is offered "log in" and put through creation — which
    /// fails, after a passkey prompt, with `409 a different account is
    /// already signed in on this profile`.
    #[dialog_common::test]
    fn it_offers_the_ceremony_it_will_actually_run() {
        use tonk_schema::email_state as answer;

        for state in [answer::ACTIVE, answer::PENDING] {
            assert_eq!(
                action_label(state),
                Some("log in with your passkey"),
                "{state} has an account already",
            );
        }
        assert_eq!(
            action_label(answer::UNREGISTERED),
            Some("create a passkey"),
            "nobody has this address",
        );
        // Nothing to offer where no ceremony would help.
        for state in [
            answer::CHECKING,
            answer::SUSPENDED,
            answer::INVALID,
            answer::UNAVAILABLE,
        ] {
            assert_eq!(action_label(state), None, "{state} offers no step");
        }
    }

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
        assert!(claim.contains("Attach a sync remote to a space, and share it."));
        assert!(!claim.contains("spot"));
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
