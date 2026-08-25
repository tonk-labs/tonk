//! Running a WebAuthn ceremony on the service worker's behalf.
//!
//! The worker has no `window`, so when an operation it is running needs
//! the account secret (to derive the recipient custodied seeds are
//! sealed to) on a device whose root record predates that key, it posts a
//! `webauthn` message to the document that asked for the operation. This
//! module answers: one passkey assertion derives the key, and saving it
//! with the root (`POST /api/identity/root`) is what the worker is
//! waiting on. Nothing is replayed; the worker's operation resumes.

use std::cell::Cell;

use tonk_worker_api::{ENCRYPTION_KEY_REQUEST, RootStatus, WEBAUTHN, WebAuthnRequest};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::MessageEvent;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// One assertion at a time: a second request arriving while the
    /// prompt is up would stack a second prompt on the same answer.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

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
        crate::identity_bridge::PublishEncryptionKeyInput { endpoint },
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
        let Ok(message) = serde_wasm_bindgen::from_value::<WebAuthnRequest>(event.data()) else {
            return;
        };
        if message.message_type != WEBAUTHN || message.request != ENCRYPTION_KEY_REQUEST {
            return;
        }
        if BUSY.with(|busy| busy.replace(true)) {
            return;
        }
        wasm_bindgen_futures::spawn_local(async move {
            match publish_encryption_key().await {
                Ok(true) => tonk_common::log!("custody: encryption key published for the worker"),
                Ok(false) => {}
                Err(error) => tonk_common::log!("custody: encryption key not published: {error}"),
            }
            BUSY.with(|busy| busy.set(false));
        });
    });
    let _ = service_worker
        .add_event_listener_with_callback("message", listener.as_ref().unchecked_ref());
    listener.forget();
}
