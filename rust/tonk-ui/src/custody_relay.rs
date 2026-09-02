//! Running a WebAuthn ceremony on the service worker's behalf.
//!
//! The worker has no `window`, so when an operation it is running needs
//! the account secret (to derive the recipient custodied seeds are
//! sealed to) on a device whose root record predates that key, it posts a
//! `webauthn` message to the document that asked for the operation. This
//! module answers — but never silently: a passkey prompt with no
//! surrounding context reads as a phishing attempt and teaches people to
//! dismiss it, and `credentials.get` wants a user gesture anyway. The
//! request raises a consent card naming what is being asked and why; the
//! card's button runs the assertion inside the click, saves the derived
//! key with the root (`POST /api/identity/root`, what the worker is
//! waiting on), and stays up to say what happened.

use std::cell::Cell;

use tonk_worker_api::{
    LINK_ACCOUNT, LinkAccountRequest, RootStatus, WEBAUTHN, WebAuthnKind, WebAuthnRequest,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{Element, MessageEvent};

use crate::user_error::{self, AccountAction};

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// One card at a time: a second request arriving while the card is
    /// up is already answered by the save the first one performs.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

const CARD_ID: &str = "tonk-custody-consent";

const CARD_HTML: &str = r#"
<div style="position:fixed;right:1rem;bottom:1rem;z-index:2147483647;max-width:22rem;
            background:#1c1c1e;color:#f2f2f4;border:1px solid #3a3a3d;border-radius:12px;
            padding:1rem 1.25rem;font:14px/1.45 system-ui,sans-serif;
            box-shadow:0 8px 30px rgba(0,0,0,.45)">
  <strong style="display:block;margin-bottom:.35rem">Passkey needed</strong>
  <p id="tonk-custody-text" style="margin:0 0 .8rem">
    Tonk is securing something to your account and needs your passkey to
    unlock the account&rsquo;s custody key on this device.
  </p>
  <div id="tonk-custody-actions" style="display:flex;gap:.5rem;justify-content:flex-end">
    <button id="tonk-custody-dismiss"
            style="background:none;border:none;color:#9a9aa0;cursor:pointer;font:inherit">
      Not now</button>
    <button id="tonk-custody-continue"
            style="background:#4a7dff;border:none;color:white;border-radius:8px;
                   padding:.4rem .9rem;cursor:pointer;font:inherit">
      Use passkey</button>
  </div>
</div>
"#;

/// Derive the account's encryption key through a passkey assertion and
/// save it with the root. `Ok(false)` when there was nothing to do: no
/// root on this device, or the key is already recorded.
pub(crate) async fn publish_encryption_key() -> Result<bool, String> {
    let RootStatus::Ready {
        credential_id,
        delegation_hex,
        encryption_key,
        ..
    } = crate::api::root_status()
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(false);
    };
    if encryption_key.is_some() {
        return Ok(false);
    }
    let endpoint = crate::account::proposed_remote()?;
    let published = crate::identity_bridge::publish_encryption_key(
        crate::identity_bridge::PublishEncryptionKeyInput {
            endpoint,
            credential_id: Some(credential_id.clone()),
        },
    )
    .await
    .map_err(|error| error.to_string())?;
    crate::api::save_root(
        credential_id,
        delegation_hex,
        None,
        Some(published.encryption_key),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(true)
}

fn remove_card() {
    if let Some(card) = card() {
        card.remove();
    }
    BUSY.with(|busy| busy.set(false));
}

fn card() -> Option<Element> {
    web_sys::window()?.document()?.get_element_by_id(CARD_ID)
}

fn set_card_text(text: &str) {
    if let Some(card) = card()
        && let Ok(Some(message)) = card.query_selector("#tonk-custody-text")
    {
        message.set_text_content(Some(text));
    }
    if let Some(card) = card()
        && let Ok(Some(actions)) = card.query_selector("#tonk-custody-actions")
    {
        actions.remove();
    }
}

fn remove_card_after(milliseconds: i32) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = Closure::once(remove_card);
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        closure.as_ref().unchecked_ref(),
        milliseconds,
    );
    closure.forget();
}

fn on_click(card: &Element, selector: &str, callback: impl FnMut() + 'static) {
    let Ok(Some(button)) = card.query_selector(selector) else {
        return;
    };
    let closure = Closure::<dyn FnMut()>::new(callback);
    let _ = button.add_event_listener_with_callback("click", closure.as_ref().unchecked_ref());
    closure.forget();
}

/// Raise the consent card. The continue button runs the assertion inside
/// the click and reports the outcome on the card; dismissing leaves the
/// worker's wait to time out, failing the operation that asked.
fn show_consent() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        BUSY.with(|busy| busy.set(false));
        return;
    };
    let (Some(body), Ok(host)) = (document.body(), document.create_element("div")) else {
        BUSY.with(|busy| busy.set(false));
        return;
    };
    host.set_id(CARD_ID);
    host.set_inner_html(CARD_HTML);
    let _ = body.append_child(&host);

    on_click(&host, "#tonk-custody-dismiss", || {
        let mut attempt = crate::account_observability::WebAccountAttempt::start(
            AccountAction::FinishAccountBackup,
            tonk_analytics::account::Surface::CustodyConsent,
            tonk_analytics::account::Trigger::User,
            tonk_analytics::account::AccountState::Ready,
        );
        attempt.finish(
            tonk_analytics::account::Stage::PasskeyAssert,
            tonk_analytics::account::AccountOutcome::cancelled(),
        );
        remove_card();
    });
    on_click(&host, "#tonk-custody-continue", move || {
        set_card_text("Waiting for your passkey…");
        let mut attempt = crate::account_observability::WebAccountAttempt::start(
            AccountAction::FinishAccountBackup,
            tonk_analytics::account::Surface::CustodyConsent,
            tonk_analytics::account::Trigger::User,
            tonk_analytics::account::AccountState::Ready,
        );
        wasm_bindgen_futures::spawn_local(async move {
            match publish_encryption_key().await {
                Ok(true) => {
                    attempt.finish(
                        tonk_analytics::account::Stage::Complete,
                        tonk_analytics::account::AccountOutcome::success(),
                    );
                    tonk_common::log!("custody: encryption key published for the worker");
                    set_card_text("Account key saved on this device.");
                }
                Ok(false) => {
                    attempt.finish(
                        tonk_analytics::account::Stage::Complete,
                        tonk_analytics::account::AccountOutcome::success(),
                    );
                    set_card_text("Nothing was needed after all.")
                }
                Err(error) => {
                    tonk_common::log!("custody: encryption key not published: {error}");
                    set_card_text(&user_error::diagnostic(
                        AccountAction::FinishAccountBackup,
                        &error,
                    ));
                    let problem = user_error::problem_from_diagnostic(
                        AccountAction::FinishAccountBackup,
                        &error,
                    );
                    attempt.finish(
                        tonk_analytics::account::Stage::PasskeyAssert,
                        problem.outcome,
                    );
                }
            }
            remove_card_after(4000);
        });
    });
}

/// Install the service-worker message listener on the top document.
pub fn install() {
    if INSTALLED.with(|installed| installed.replace(true)) {
        return;
    }
    let Some(window) = web_sys::window() else {
        return;
    };
    let service_worker = window.navigator().service_worker();
    let listener = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // The worker decided a share needs an account and is asking for
        // one. It carries the space so the share can be finished once
        // the account exists.
        if let Ok(link) = serde_wasm_bindgen::from_value::<LinkAccountRequest>(event.data())
            && link.message_type == LINK_ACCOUNT
        {
            crate::register_dialog::open();
            crate::register_dialog::describe(
                &serde_json::json!({
                    "reason": tonk_worker_api::share::BLOCKED_NEEDS_ACCOUNT,
                    "space": link.space,
                })
                .to_string(),
            );
            return;
        }
        let Ok(message) = serde_wasm_bindgen::from_value::<WebAuthnRequest>(event.data()) else {
            return;
        };
        if message.message_type != WEBAUTHN {
            return;
        }
        // Exhaustive on purpose. A new ceremony kind must fail to
        // compile here rather than be dropped: `create-account` was
        // once filtered out by an `if request != ENCRYPTION_KEY_REQUEST
        // { return }`, so the worker asked the page to run a signup
        // ceremony, nothing listened, and the registration dialog still
        // reported success.
        match message.request {
            WebAuthnKind::EncryptionKey => {
                if BUSY.with(|busy| busy.replace(true)) {
                    return;
                }
                show_consent();
            }
            WebAuthnKind::CreateAccount => {
                // Handled by the registration dialog, which is the only
                // thing that knows which address the ceremony is for and
                // is already on screen when this arrives. BUSY is not
                // taken: that flag guards the consent card this module
                // owns, and the dialog runs its own ceremony.
                crate::register_dialog::run_signup_ceremony();
            }
            WebAuthnKind::Custody => {
                // One assertion, then the handles go to the worker,
                // which mints and enrolls. The page builds nothing and
                // holds nothing; it only supplies the gesture WebAuthn
                // insists on happening in a window.
                mediate_custody(tonk_worker_api::CustodyIntent::Enroll(
                    message.enrollment.unwrap_or_default(),
                ));
            }
        }
    });
    let _ = service_worker
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    listener.forget();
}

/// Run one custody assertion and hand the worker its derivation
/// handles.
///
/// The page's whole part in any custody work: `usePasskey` posts the
/// handles and resolves when the worker is done, so a failure here is
/// the intent's failure and is reported as one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn mediate_custody(intent: tonk_worker_api::CustodyIntent) {
    // Which ceremony the intent implies: a new account needs a new
    // passkey, everything else asserts one that exists.
    let method = match &intent {
        tonk_worker_api::CustodyIntent::CreateAccount(_) => "createPasskey",
        tonk_worker_api::CustodyIntent::AddPasskey(_) => "addPasskey",
        tonk_worker_api::CustodyIntent::Enroll(_) | tonk_worker_api::CustodyIntent::Login(_) => {
            "usePasskey"
        }
    };
    mediate_with(method, intent);
}

/// [`mediate_custody`], naming the ceremony explicitly.
pub(crate) fn mediate_with(method: &'static str, intent: tonk_worker_api::CustodyIntent) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = run(method, intent).await {
            report(&error.message);
        }
    });
}

/// A ceremony that did not complete: what to say, and the service's own
/// reason when it had one.
///
/// The reason is a variant rather than a sentence to match. Matching
/// prose made the service's wording load-bearing in the page: a reworded
/// refusal quietly downgraded "open the link in your email" to "check
/// your connection", and nothing failed to say so.
#[derive(Debug, Clone)]
pub(crate) struct CeremonyError {
    /// What went wrong, for a reader.
    pub message: String,
    /// Why the access service refused, when it is what refused.
    pub denial: Option<tonk_identity::custody::CustodyDenial>,
    /// Browser refusal, independently of a service denial.
    pub refusal: Option<tonk_identity::passkey::CeremonyRefusal>,
}

impl std::fmt::Display for CeremonyError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.message)
    }
}

/// [`mediate_custody`], awaited: for a caller that has to know whether
/// the work landed before it moves on.
pub(crate) async fn mediate_now(
    method: &'static str,
    intent: tonk_worker_api::CustodyIntent,
) -> Result<(), CeremonyError> {
    run(method, intent).await
}

/// A page ceremony that has already been invoked and now only needs its
/// asynchronous worker handoff to finish.
pub(crate) struct Mediation {
    promise: js_sys::Promise,
}

impl Mediation {
    pub(crate) async fn finish(self) -> Result<(), CeremonyError> {
        wasm_bindgen_futures::JsFuture::from(self.promise)
            .await
            .map(|_| ())
            .map_err(|error| CeremonyError::thrown(&error))
    }
}

/// Log a mediation failure, keeping a dismissed prompt quiet: declining
/// the passkey is a decision, not a fault.
fn report(error: &str) {
    if error.starts_with("NotAllowedError") {
        web_sys::console::debug_1(&"custody: the passkey prompt was dismissed".into());
    } else {
        web_sys::console::warn_1(&format!("custody: {error}").into());
    }
}

async fn run(
    method: &'static str,
    intent: tonk_worker_api::CustodyIntent,
) -> Result<(), CeremonyError> {
    begin(method, intent)?.finish().await
}

/// Invoke the window ceremony now and return its in-flight promise.
///
/// Keeping invocation separate from completion lets a click handler cross the
/// Rust/JavaScript bridge before returning, which is required for mobile
/// WebAuthn's transient user activation.
pub(crate) fn begin(
    method: &'static str,
    intent: tonk_worker_api::CustodyIntent,
) -> Result<Mediation, CeremonyError> {
    use wasm_bindgen::{JsCast, JsValue};

    let identity = web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"tonkIdentity".into()).ok())
        .filter(|value| !value.is_undefined())
        .ok_or_else(|| CeremonyError::said("window.tonkIdentity is not installed"))?;
    let ceremony = js_sys::Reflect::get(&identity, &method.into())
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
        .ok_or_else(|| CeremonyError::said(format!("tonkIdentity.{method} is missing")))?;

    // The address, lifted out of the intent and onto the ceremony input.
    // A passkey manager shows `user.name`, and without one the ceremony
    // falls back to a random hex string — so an entry that should read
    // "someone@example.com" reads "9e5a87ca4a602850…" and a person cannot
    // tell their own passkeys apart. Only creation has an address to give:
    // asserting an existing passkey names no user entity at all.
    let email = match &intent {
        tonk_worker_api::CustodyIntent::CreateAccount(creation) => Some(creation.email.clone()),
        tonk_worker_api::CustodyIntent::Enroll(enrollment) => enrollment.email.clone(),
        tonk_worker_api::CustodyIntent::AddPasskey(_)
        | tonk_worker_api::CustodyIntent::Login(_) => None,
    };

    let request = serde_wasm_bindgen::to_value(&intent)
        .map_err(|error| CeremonyError::said(format!("the request did not serialize: {error}")))?;
    let input = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&input, &"request".into(), &request);
    if let Some(email) = email.filter(|email| !email.trim().is_empty()) {
        let _ = js_sys::Reflect::set(&input, &"name".into(), &email.as_str().into());
        let _ = js_sys::Reflect::set(&input, &"displayName".into(), &email.as_str().into());
    }

    let answer = ceremony
        .call1(&JsValue::NULL, &input)
        .map_err(|error| CeremonyError::thrown(&error))?;
    let promise = answer.dyn_into::<js_sys::Promise>().map_err(|_| {
        CeremonyError::said(format!("tonkIdentity.{method} did not return a promise"))
    })?;
    Ok(Mediation { promise })
}

/// A failure that never reached the service carries no reason from it.
impl<T: Into<String>> From<T> for CeremonyError {
    fn from(message: T) -> Self {
        Self::said(message)
    }
}

impl CeremonyError {
    /// A failure this page found itself, with no service involved.
    pub(crate) fn said(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            denial: None,
            refusal: None,
        }
    }

    /// A thrown value, with the refusal reason it carries.
    ///
    /// The worker sets `code` on what it rejects with when the access
    /// service is what refused; anything else -- a dismissed prompt, an
    /// unsupported authenticator -- carries none and stays a message.
    fn thrown(error: &wasm_bindgen::JsValue) -> Self {
        let denial = js_sys::Reflect::get(error, &"code".into())
            .ok()
            .and_then(|code| code.as_string())
            .and_then(|code| {
                let message = js_sys::Reflect::get(error, &"message".into())
                    .ok()
                    .and_then(|value| value.as_string())
                    .unwrap_or_default();
                tonk_identity::custody::CustodyDenial::from_code(&code, &message)
            });
        let refusal = js_sys::Reflect::get(error, &"name".into())
            .ok()
            .and_then(|name| name.as_string())
            .map(|name| tonk_identity::passkey::CeremonyRefusal::from_name(&name));
        Self {
            message: describe(error),
            denial,
            refusal,
        }
    }
}

/// A thrown value as text, keeping the DOM error name in front so a
/// caller can tell a dismissed prompt from a real failure.
fn describe(error: &wasm_bindgen::JsValue) -> String {
    let name = js_sys::Reflect::get(error, &"name".into())
        .ok()
        .and_then(|value| value.as_string());
    let message = js_sys::Reflect::get(error, &"message".into())
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| format!("{error:?}"));
    match name {
        Some(name) => format!("{name}: {message}"),
        None => message,
    }
}
