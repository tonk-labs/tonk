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

    on_click(&host, "#tonk-custody-dismiss", remove_card);
    on_click(&host, "#tonk-custody-continue", move || {
        set_card_text("Waiting for your passkey…");
        wasm_bindgen_futures::spawn_local(async move {
            match publish_encryption_key().await {
                Ok(true) => {
                    tonk_common::log!("custody: encryption key published for the worker");
                    set_card_text("Account key saved on this device.");
                }
                Ok(false) => set_card_text("Nothing was needed after all."),
                Err(error) => {
                    tonk_common::log!("custody: encryption key not published: {error}");
                    set_card_text("The passkey check did not complete. Reload and try again.");
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
                mediate_custody(message.enrollment.unwrap_or_default());
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
/// `usePasskey` posts them and resolves when the worker is done, so a
/// failure here is the enrollment's failure and is reported as one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn mediate_custody(enrollment: tonk_worker_api::Enrollment) {
    use wasm_bindgen::{JsCast, JsValue};

    wasm_bindgen_futures::spawn_local(async move {
        let Some(identity) = web_sys::window()
            .and_then(|window| js_sys::Reflect::get(&window, &"tonkIdentity".into()).ok())
            .filter(|value| !value.is_undefined())
        else {
            web_sys::console::warn_1(&"custody: window.tonkIdentity is not installed".into());
            return;
        };
        let Ok(use_passkey) = js_sys::Reflect::get(&identity, &"usePasskey".into())
            .and_then(|value| value.dyn_into::<js_sys::Function>())
        else {
            web_sys::console::warn_1(&"custody: tonkIdentity.usePasskey is missing".into());
            return;
        };

        let input = js_sys::Object::new();
        match serde_wasm_bindgen::to_value(&enrollment) {
            Ok(request) => {
                let _ = js_sys::Reflect::set(&input, &"request".into(), &request);
            }
            Err(error) => {
                web_sys::console::warn_1(
                    &format!("custody: the enrollment did not serialize: {error}").into(),
                );
                return;
            }
        }

        let answer = match use_passkey.call1(&JsValue::NULL, &input) {
            Ok(value) => value,
            Err(error) => {
                web_sys::console::warn_1(&format!("custody: {error:?}").into());
                return;
            }
        };
        let Ok(promise) = answer.dyn_into::<js_sys::Promise>() else {
            return;
        };
        if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
            // A dismissed prompt is a decision, not a fault: someone
            // declined the passkey, and enrollment simply did not
            // happen. Anything else is worth a warning.
            let name = js_sys::Reflect::get(&error, &"name".into())
                .ok()
                .and_then(|value| value.as_string());
            if name.as_deref() == Some("NotAllowedError") {
                web_sys::console::debug_1(&"custody: the passkey prompt was dismissed".into());
            } else {
                web_sys::console::warn_1(&format!("custody: the handoff failed: {error:?}").into());
            }
        }
    });
}
