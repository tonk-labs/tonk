//! Getting the account's encryption key onto a device that needs it.
//!
//! Custody seals to the account's X25519 recipient, which only a holder
//! of the account secret can derive. The onboarding account's secret is
//! local, so its recipient is derived on demand. A passkey account's
//! secret only exists inside a WebAuthn assertion on a page, so a linked
//! device whose root record predates the key asks the page that
//! originated the operation to run one (`request_webauthn`), and waits
//! for the page to save the key with the root (`POST /api/identity/root`)
//! before continuing. Nothing is replayed: the operation that needed the
//! key simply resumes once it is there.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::TonkWorkerError;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::router::{AppState, ClientId};
use dialog_varsig::Did;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
thread_local! {
    /// Operations waiting for the page to record an encryption key.
    static WAITERS: std::cell::RefCell<Vec<tokio::sync::oneshot::Sender<Did>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// How long an operation waits for the page's assertion before giving
/// up. Generous: the user has to touch a passkey.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
const ASSERTION_TIMEOUT: web_time::Duration = web_time::Duration::from_secs(120);

/// Wake every operation waiting for the key. Called by the root save
/// whenever a record carrying a recipient lands.
pub(crate) fn notify_encryption_key(recipient: &Did) {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    WAITERS.with(|waiters| {
        for waiter in waiters.borrow_mut().drain(..) {
            let _ = waiter.send(recipient.clone());
        }
    });
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    let _ = recipient;
}

/// Make sure a custody recipient is obtainable before an operation that
/// custodies a seed takes the state lock.
///
/// Returns at once when the device has no passkey root (the onboarding
/// recipient derives locally) or when the root record already carries the
/// key. Otherwise asks `client` for a passkey assertion and waits for the
/// key to be saved. Must be called WITHOUT the state lock held: the page
/// answers through `/api/identity/root`, which needs it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn ensure_recipient(
    state: &AppState,
    client: Option<&ClientId>,
) -> Result<(), TonkWorkerError> {
    let root = {
        let tonk = state.read().await;
        match super::identity::local_root(&tonk).await {
            Ok(root) => root,
            Err(TonkWorkerError::RootRequired) => return Ok(()),
            Err(error) => return Err(error),
        }
    };
    if root.encryption_key.is_some() {
        return Ok(());
    }
    {
        let tonk = state.read().await;
        if super::account_state::published_encryption_key(&tonk, &root.root_did)
            .await?
            .is_some()
        {
            return Ok(());
        }
    }
    let Some(client) = client else {
        return Err(TonkWorkerError::Conflict(
            "the account has not published its encryption key on this device, and no page \
             asked for this operation, so no passkey assertion can derive it"
                .to_string(),
        ));
    };
    request_and_wait(client).await
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn request_and_wait(client: &ClientId) -> Result<(), TonkWorkerError> {
    // Registered before the request goes out, so an answer that arrives
    // faster than this task resumes is not missed.
    let receiver = wait_for_key();
    super::navigate::request_webauthn(client, tonk_worker_api::WebAuthnKind::EncryptionKey).await?;
    await_key(receiver, ASSERTION_TIMEOUT).await.map(|_| ())
}

/// Register for the next recorded encryption key.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn wait_for_key() -> tokio::sync::oneshot::Receiver<Did> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    WAITERS.with(|waiters| waiters.borrow_mut().push(sender));
    receiver
}

/// Wait for a registered key, giving up after `timeout`.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn await_key(
    receiver: tokio::sync::oneshot::Receiver<Did>,
    timeout: web_time::Duration,
) -> Result<Did, TonkWorkerError> {
    use futures_util::future::{Either, select};

    let timeout = Box::pin(crate::r#async::sleep(timeout));
    match select(Box::pin(receiver), timeout).await {
        Either::Left((Ok(recipient), _)) => Ok(recipient),
        Either::Left((Err(_), _)) => Err(TonkWorkerError::Internal(
            "the encryption-key waiter was dropped".to_string(),
        )),
        Either::Right(_) => Err(TonkWorkerError::Conflict(
            "no passkey assertion answered in time; the account's encryption key is still \
             unpublished on this device"
                .to_string(),
        )),
    }
}

/// Ask the page to mediate a passkey so this worker can mint custody
/// material.
///
/// The worker has no `window`, so the assertion happens on the page
/// that asked for the enrollment. What comes back is two derivation
/// handles, not key material, and this enrollment travels with the
/// request so the handoff knows what it is for.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn request_mediation(client: &ClientId, email: Option<String>) {
    let enrollment = tonk_worker_api::Enrollment { email };
    if let Err(error) = super::navigate::request_webauthn_with(
        client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(enrollment),
    )
    .await
    {
        log!("custody: the page could not be asked to mediate: {error}");
    }
}

/// Is this SW-global message a custody handoff?
///
/// Read off the raw `JsValue`: the two handles it carries are
/// `CryptoKey`s, which do not survive a trip through JSON.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) fn is_custody_envelope(data: &wasm_bindgen::JsValue) -> bool {
    js_sys::Reflect::get(data, &wasm_bindgen::JsValue::from_str("type"))
        .ok()
        .and_then(|value| value.as_string())
        .is_some_and(|kind| kind == "custody")
}

/// Take the page's derivation handles, do the work that needs them, and
/// drop them.
///
/// The page mediates WebAuthn and nothing else: it holds no bytes, and
/// the handles it posts are non-extractable. This is the only place a
/// custodian exists, and it exists for the length of one request — the
/// reply goes back over the transferred port, and the custodian is
/// dropped when this returns.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn receive(state: AppState, data: wasm_bindgen::JsValue, ports: js_sys::Array) {
    use wasm_bindgen::{JsCast, JsValue};

    let Some(port) = ports.get(0).dyn_into::<web_sys::MessagePort>().ok() else {
        log!("custody: handoff arrived with no reply port; dropping");
        return;
    };

    let answer = match custodian_from(&data) {
        Ok(custodian) => perform(state, &data, custodian).await,
        Err(error) => Err(error),
    };

    let reply = js_sys::Object::new();
    match answer {
        Ok(value) => {
            let _ = js_sys::Reflect::set(&reply, &JsValue::from_str("ok"), &value);
        }
        Err(error) => {
            log!("custody: {error}");
            let _ = js_sys::Reflect::set(
                &reply,
                &JsValue::from_str("error"),
                &JsValue::from_str(&error),
            );
        }
    }
    if let Err(error) = port.post_message(&reply) {
        log!("custody: the reply did not post: {error:?}");
    }
}

/// Mint the custody material this enrollment needs, then enroll.
///
/// What the page used to do and no longer can: generate the account
/// secret, seal it under the passkey's KEK, mint the custody space's
/// consent, and pre-sign the cell write. All of it here, so the secret
/// never exists outside the worker and the four call sites with no
/// ceremony at hand can enroll like any other.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn perform(
    state: AppState,
    data: &wasm_bindgen::JsValue,
    custodian: tonk_identity::custodian::Custodian,
) -> Result<wasm_bindgen::JsValue, String> {
    use dialog_varsig::Principal as _;
    use wasm_bindgen::JsValue;

    let request =
        js_sys::Reflect::get(data, &JsValue::from_str("request")).unwrap_or(JsValue::UNDEFINED);
    let enrollment: tonk_worker_api::Enrollment = serde_wasm_bindgen::from_value(request)
        .map_err(|error| format!("the handoff carried an unreadable enrollment: {error}"))?;
    let email = enrollment.email.filter(|value| !value.trim().is_empty());

    let account = custodian
        .account()
        .create()
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| format!("the account did not seal under this passkey: {error:#}"))?;

    let custody = custodian
        .signer()
        .await
        .map_err(|error| format!("the custody signer did not derive: {error:#}"))?;
    let root = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;

    let sealed = account.envelope().encode();
    let consent = tonk_identity::custody::sign_custody_consent(custody.clone(), &root.did())
        .await
        .map_err(|error| format!("the custody consent did not sign: {error:#}"))?;
    let recovery =
        tonk_identity::custody::sign_deferred_publish_invocation(custody.clone(), &sealed)
            .await
            .map_err(|error| format!("the cell write did not sign: {error:#}"))?;

    let material = tonk_identity::request::CustodyMaterial {
        custody: &custody.did(),
        consent: &consent,
        recovery: &recovery,
        sealed: &sealed,
    };

    let origin = crate::router::customer::service_origin().map_err(|error| format!("{error}"))?;
    let tonk = state.read().await;
    let receipt = crate::router::customer::enroll_customer(&tonk, &origin, email, &material)
        .await
        .map_err(|error| format!("{error}"))?;

    let answer = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &answer,
        &JsValue::from_str("customer"),
        &JsValue::from_str(&receipt.customer.to_string()),
    );
    Ok(answer.into())
}

/// Rebuild the custodian from the two posted handles.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn custodian_from(
    data: &wasm_bindgen::JsValue,
) -> Result<tonk_identity::custodian::Custodian, String> {
    use wasm_bindgen::{JsCast, JsValue};

    let handle = |name: &str| -> Result<web_sys::CryptoKey, String> {
        js_sys::Reflect::get(data, &JsValue::from_str(name))
            .ok()
            .and_then(|value| value.dyn_into::<web_sys::CryptoKey>().ok())
            .ok_or_else(|| format!("the handoff carried no {name} handle"))
    };
    let credential_id = js_sys::Reflect::get(data, &JsValue::from_str("credentialId"))
        .ok()
        .and_then(|value| value.as_string())
        .ok_or_else(|| "the handoff carried no credential id".to_string())?;
    let credential_id = hex::decode(&credential_id)
        .map_err(|error| format!("the credential id is not hex: {error}"))?;

    Ok(tonk_identity::custodian::Custodian::Passkey(
        tonk_identity::webcrypto_kek::Custodian::new(credential_id, handle("key")?, handle("kek")?),
    ))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use crate::router::tests::{test_state, test_state_without_account, test_state_without_root};
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    /// An onboarding device derives its recipient locally: nothing to ask.
    #[dialog_common::test]
    async fn it_needs_no_assertion_without_a_passkey_root() {
        let state = Arc::new(RwLock::new(test_state_without_root().await));
        ensure_recipient(&state, None).await.unwrap();
    }

    /// The fixture's root carries the key a ceremony would have recorded.
    #[dialog_common::test]
    async fn it_needs_no_assertion_when_the_root_carries_the_key() {
        let state = Arc::new(RwLock::new(test_state().await));
        ensure_recipient(&state, None).await.unwrap();
    }

    /// The page answers by saving the key with the root; that save is
    /// what wakes the waiting operation.
    #[dialog_common::test]
    async fn it_wakes_the_waiter_when_the_root_save_records_the_key() {
        let state = test_state_without_account().await;
        let receiver = wait_for_key();
        let recipient = {
            use dialog_varsig::Principal as _;
            tonk_identity::envelope::AccountSecret::from_bytes(zeroize::Zeroizing::new([9u8; 32]))
                .secret()
                .did()
        };
        let root = super::super::identity::local_root(&state).await.unwrap();
        super::super::identity::persist_root(
            &state,
            tonk_worker_api::SaveRootRequest {
                credential_id: root.credential_id,
                delegation_hex: hex::encode(root.bytes),
                passkey: None,
                encryption_key: Some(recipient.to_string()),
            },
        )
        .await
        .unwrap();
        let woken = await_key(receiver, web_time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(woken, recipient);
    }

    /// Without a page to ask, a linked device lacking the key refuses
    /// rather than waiting on nothing.
    #[dialog_common::test]
    async fn it_refuses_without_a_client_to_ask() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        strip_recorded_key(&state).await;
        let error = ensure_recipient(&state, None).await.unwrap_err();
        assert!(matches!(error, TonkWorkerError::Conflict(_)), "{error}");
    }

    /// Overwrite the local root record without its recipient and drop the
    /// published fact, the shape of a device linked before the key
    /// existed.
    async fn strip_recorded_key(state: &AppState) {
        use tonk_schema::prelude::DidExt as _;
        let tonk = state.read().await;
        let root = super::super::identity::local_root(&tonk).await.unwrap();
        super::super::identity::forget_encryption_key(&tonk)
            .await
            .unwrap();
        let published =
            super::super::account_state::published_encryption_key(&tonk, &root.root_did)
                .await
                .unwrap()
                .expect("the fixture published one");
        tonk.reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .transaction()
            .retract(tonk_schema::AccountEncryptionKey::new(
                root.root_did.this(),
                published.this(),
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }
}
