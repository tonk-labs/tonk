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

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, HtmlElement};

thread_local! {
    /// The dialog is a singleton: a second refusal while it is up is
    /// already answered by the registration in progress.
    static OPEN: Cell<bool> = const { Cell::new(false) };
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
<wa-dialog id="tonk-register-dialog" label="Create an account to share" open
           style="--width: 26rem">
  <p id="tonk-register-lede" style="margin:0 0 1rem">
    Sharing a spot needs an account, so the people you share with have
    somewhere to sync from.
  </p>
  <form id="tonk-register-form">
    <wa-input id="tonk-register-email" name="email" type="email"
              label="Email" placeholder="you@example.com"
              autocomplete="username webauthn" required
              autofocus></wa-input>
    <p id="tonk-register-status" style="margin:.6rem 0 0;min-height:1.2em;
       font-size:.9em;color:var(--wa-color-text-quiet,#9a9aa0)"></p>
  </form>
  <wa-button id="tonk-register-dismiss" slot="footer" appearance="plain"
             variant="neutral">Not now</wa-button>
  <wa-button id="tonk-register-submit" slot="footer" variant="brand">
    Create account</wa-button>
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
}

/// Take the dialog down.
pub fn close() {
    OPEN.with(|open| open.set(false));
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
        return;
    };
    if !is_plausible(&email) {
        set_status("");
        return;
    }
    set_status("Checking…");
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = crate::api::transact_profile(check_email_claim(&email)).await {
            tonk_common::log!("register: could not ask about the address: {error}");
            set_status("");
        }
    });
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

/// Start the ceremony for the typed address.
fn submit() {
    let Some(email) = address().filter(|email| is_plausible(email)) else {
        set_status("Enter the address you want to use.");
        return;
    };
    set_status("Waiting for your passkey…");
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = crate::api::transact_profile(register_claim(&email)).await {
            tonk_common::log!("register: could not start registration: {error}");
            set_status("Registration could not start. Try again.");
        }
    });
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

/// Re-word the dialog for the refusal that raised it.
///
/// `needs-activation` is not a signup: the account exists and the link
/// is unopened, so the dialog says so instead of offering to create a
/// second one.
pub fn describe(reason: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if reason != tonk_worker_api::share::BLOCKED_NEEDS_ACTIVATION {
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
