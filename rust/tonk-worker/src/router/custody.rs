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
        if super::account_state::published_sealed_inbox(&tonk, &root.root_did)
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

/// A failed custody read, as the message to show and the reason behind it.
///
/// The handlers below answer `Result<_, String>` because they cross a
/// `postMessage` boundary that carries no Rust types. That string alone
/// lost WHY the service refused, and the page was left matching prose to
/// tell "confirm your email" from "check your connection" -- so a
/// reworded refusal silently became the wrong instruction. The code
/// travels beside the message and the page branches on it instead.
///
/// Prefixed rather than parallel-plumbed: the alternative is retyping
/// every handler and the dispatch between them to move one tag.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn refusal(context: &str, error: &tonk_identity::Error) -> String {
    match tonk_identity::custody::denial_of(error) {
        Some(denial) => format!("{}\u{1f}{denial}", denial.code()),
        None => format!("{context}: {error:#}"),
    }
}

/// The `(code, message)` a [`refusal`] carries, for the reply.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn split_refusal(error: &str) -> (Option<&str>, &str) {
    match error.split_once('\u{1f}') {
        Some((code, message)) => (Some(code), message),
        None => (None, error),
    }
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
            let (code, message) = split_refusal(&error);
            let _ = js_sys::Reflect::set(
                &reply,
                &JsValue::from_str("error"),
                &JsValue::from_str(message),
            );
            // Present only when the service said why. The page shows the
            // step that clears it rather than guessing from the sentence.
            if let Some(code) = code {
                let _ = js_sys::Reflect::set(
                    &reply,
                    &JsValue::from_str("code"),
                    &JsValue::from_str(code),
                );
            }
        }
    }
    if let Err(error) = port.post_message(&reply) {
        log!("custody: the reply did not post: {error:?}");
    }
}

/// Do what the handoff asked for, with the handles it carried.
///
/// What the page used to do and no longer can: generate the account
/// secret, seal it under the passkey's KEK, mint the custody space's
/// consent, and pre-sign the cell write. All of it here, so the secret
/// never exists outside the worker and every call site — including the
/// four with no ceremony at hand — reaches it the same way.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn perform(
    state: AppState,
    data: &wasm_bindgen::JsValue,
    custodian: tonk_identity::custodian::Custodian,
) -> Result<wasm_bindgen::JsValue, String> {
    use wasm_bindgen::JsValue;

    let request =
        js_sys::Reflect::get(data, &JsValue::from_str("request")).unwrap_or(JsValue::UNDEFINED);
    let intent: tonk_worker_api::CustodyIntent = serde_wasm_bindgen::from_value(request)
        .map_err(|error| format!("the handoff carried an unreadable request: {error}"))?;

    match intent {
        tonk_worker_api::CustodyIntent::Enroll(enrollment) => {
            // Reuse before minting. A browser that already sealed an
            // account under this passkey reaches here again on the
            // resend path, and a fresh secret would derive a DIFFERENT
            // account root — enrolling it would overwrite the recovery
            // everything since the first ceremony depends on. The
            // recorded envelope is present exactly when this browser
            // enrolled before, so its absence is what makes creation
            // safe.
            let account = match recorded_account(&state, &custodian).await? {
                Some(account) => account,
                None => open(&custodian).await?,
            };
            enroll(&state, &custodian, &account, enrollment.email, None).await
        }
        tonk_worker_api::CustodyIntent::CreateAccount(creation) => {
            create(&state, &custodian, creation).await
        }
        tonk_worker_api::CustodyIntent::Login(link) => login(&state, &custodian, link).await,
        tonk_worker_api::CustodyIntent::AddPasskey(addition) => {
            let holder = custodian_named(data, "holder")
                .ok_or_else(|| "the handoff carried no holder passkey".to_string())?;
            add_passkey(&state, &custodian, &holder, addition).await
        }
    }
}

/// Seal the account a first passkey holds under a second one.
///
/// Both custodians travel in the same handoff, so the secret is opened
/// and re-sealed inside this call and exists nowhere else. The account
/// is checked against the one the caller meant to extend: a mismatched
/// assertion would otherwise seal a different account under the new
/// passkey and silently strand it.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn add_passkey(
    state: &AppState,
    added: &tonk_identity::custodian::Custodian,
    holder: &tonk_identity::custodian::Custodian,
    addition: tonk_worker_api::PasskeyAddition,
) -> Result<wasm_bindgen::JsValue, String> {
    use dialog_varsig::Principal as _;

    let account = holder
        .account()
        .load(addition.endpoint.clone())
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| refusal("the custody cell did not open", &error))?
        .ok_or_else(|| "that passkey has published no account".to_string())?;

    let root = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    if root.did().to_string() != addition.account_did {
        return Err("the asserted passkey unlocks a different account".to_string());
    }

    // The same secret, sealed again: either passkey opens the account
    // from here on.
    let sealed = added
        .account()
        .adopt(account.into_secret())
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| format!("the account did not seal under the new passkey: {error:#}"))?;

    enroll(state, added, &sealed, None, None).await
}

/// Open the account this passkey holds and link this browser to it.
///
/// The account comes back from its custody cell, so a device that has
/// only ever seen the passkey gets the account: that is what publishing
/// the cell at enrollment made possible.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn login(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    link: tonk_worker_api::DeviceLink,
) -> Result<wasm_bindgen::JsValue, String> {
    use dialog_varsig::Principal as _;

    // The address the email lookup resolved, when one ran: the DID
    // document is the account's own word on where it syncs, and the
    // caller's endpoint is this browser's origin, which is only right
    // when both devices are on the same deployment.
    let endpoint = super::email_status::resolved_service().unwrap_or_else(|| link.endpoint.clone());
    let account = custodian
        .account()
        .load(endpoint.clone())
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| refusal("the custody cell did not open", &error))?
        .ok_or_else(|| {
            "this passkey has published no account yet; create one on the browser that holds it"
                .to_string()
        })?;

    let dialog_credentials::Signer::Ed25519(root) = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;

    let device = {
        let tonk = state.read().await;
        tonk.profile.signer().signer().clone()
    };
    let ceremony =
        tonk_identity::ceremony::link_device(root.clone(), device.did(), link.device_name.clone())
            .await
            .map_err(|error| format!("the device link did not sign: {error:#}"))?;

    {
        let tonk = state.read().await;
        let root_record = tonk_worker_api::SaveRootRequest {
            credential_id: custodian
                .credential_id()
                .map(hex::encode)
                .unwrap_or_default(),
            delegation_hex: ceremony.delegation_hex.clone(),
            passkey: None,
            encryption_key: Some(account.secret().did().to_string()),
        };
        crate::router::identity::persist_root(&tonk, root_record)
            .await
            .map_err(|error| format!("the account root was not recorded: {error}"))?;
    }

    // No request: the roster is DeviceLink facts on the account's own
    // branch, and the sweep describes this device's row from the root
    // that was just persisted. Posting the same link to a service was
    // keeping a second copy of a list that already syncs.
    //
    link_account(
        state,
        &link.provider,
        &root.did().to_string(),
        &custodian
            .credential_id()
            .map(hex::encode)
            .unwrap_or_default(),
        &ceremony.delegation_hex,
        &endpoint,
        false,
    )
    .await?;
    Ok(wasm_bindgen::JsValue::UNDEFINED)
}

/// Seal a fresh account secret under this custodian.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn open(
    custodian: &tonk_identity::custodian::Custodian,
) -> Result<tonk_identity::account::Account, String> {
    custodian
        .account()
        .create()
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| format!("the account did not seal under this passkey: {error:#}"))
}

/// The account this browser already sealed under `custodian`, when it
/// did: the recorded envelope, reopened. `None` means no enrollment ever
/// ran here, which is the one state where minting a fresh secret is
/// safe. An envelope that is present but will not open is an error, not
/// a `None` — falling through to creation there would enroll a second
/// account over the recovery of the first.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn recorded_account(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
) -> Result<Option<tonk_identity::account::Account>, String> {
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::Principal as _;
    use tonk_schema::prelude::DidExt as _;

    let custody = custodian
        .signer()
        .await
        .map_err(|error| format!("the custody signer did not derive: {error:#}"))?
        .did();

    let tonk = state.read().await;
    let Ok(branch) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .acquire(&tonk.operator)
        .await
    else {
        // No branch yet is a fresh browser: nothing recorded.
        return Ok(None);
    };
    let rows: Vec<tonk_schema::SecretMessage> = branch
        .handle()
        .query()
        .select(Query::<tonk_schema::SecretMessage> {
            this: Term::var("this"),
            to: Term::from(tonk_schema::domain::custody::To(custody.this())),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| format!("the recorded envelopes did not read: {error:?}"))?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };

    let envelope = tonk_identity::envelope::Envelope::decode(&row.message.0)
        .map_err(|error| format!("the recorded envelope did not decode: {error:#}"))?;
    let account = custodian
        .account()
        .import(envelope)
        .perform(&tonk_identity::account::Crypto)
        .await
        .map_err(|error| format!("the recorded envelope did not open: {error:#}"))?;
    Ok(Some(account))
}

/// Mint the custody set this account needs and register it as a
/// customer.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn enroll(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    account: &tonk_identity::account::Account,
    email: Option<String>,
    // Present only when this enrollment created the passkey: the label
    // comes from the browser that ran `credentials.create()`, and a
    // re-enrollment or an added passkey carries none.
    created_on: Option<String>,
) -> Result<wasm_bindgen::JsValue, String> {
    use wasm_bindgen::JsValue;

    let material = custody_material(custodian, account).await?;
    let origin = crate::router::customer::service_origin().map_err(|error| format!("{error}"))?;
    let email = email.filter(|value| !value.trim().is_empty());

    // The envelope, as a fact, before it is sent anywhere. The vault
    // copy bootstraps a brand-new browser, but it is written at
    // enrollment and served only once a customer activates — so a
    // device that enrolls and restarts before the emailed link is
    // clicked would otherwise have nothing to reopen its own account
    // with. Recorded first for the same reason enrollment writes the
    // cell before the customer row: what cannot be recovered must not
    // depend on the step after it succeeding.
    {
        let tonk = state.read().await;
        let passkey = created_on.map(|created_on| tonk_worker_api::PasskeyMetadata {
            created_at: (js_sys::Date::now() / 1000.0) as u64,
            created_on,
        });
        crate::router::customer::record_custody_cell(
            &tonk,
            &material.custody.to_string(),
            &hex::encode(&material.sealed),
            passkey,
            &custodian
                .credential_id()
                .map(hex::encode)
                .unwrap_or_default(),
            email.as_deref(),
        )
        .await
        .map_err(|error| format!("the custody cell was not recorded: {error}"))?;
    }

    let tonk = state.read().await;
    let receipt =
        crate::router::customer::enroll_customer(&tonk, &origin, email, &material.borrow())
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

/// The custody set every enrollment presents: the space's consent to
/// being provisioned, and the pre-signed write of its sealed cell.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn custody_material(
    custodian: &tonk_identity::custodian::Custodian,
    account: &tonk_identity::account::Account,
) -> Result<tonk_identity::request::OwnedCustodyMaterial, String> {
    use dialog_varsig::Principal as _;

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

    Ok(tonk_identity::request::OwnedCustodyMaterial {
        custody: custody.did(),
        consent,
        recovery,
        sealed,
    })
}

/// Create an account this passkey holds, record it, and enroll it.
///
/// Everything the page's `createAccount` ceremony did, minus the part
/// that had to be there: the passkey assertion. The account secret is
/// generated here and never leaves, so the browser holds no key
/// material at any point in a signup.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn create(
    state: &AppState,
    custodian: &tonk_identity::custodian::Custodian,
    creation: tonk_worker_api::AccountCreation,
) -> Result<wasm_bindgen::JsValue, String> {
    use dialog_varsig::Principal as _;

    let account = open(custodian).await?;
    let root = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    // The account secret derives Ed25519, and the account request is
    // signed with the concrete key.
    let dialog_credentials::Signer::Ed25519(root) = root;

    let device = {
        let tonk = state.read().await;
        tonk.profile.signer().signer().clone()
    };
    let device_did = device.did();

    let ceremony = tonk_identity::ceremony::create_custody_request(
        root,
        tonk_identity::ceremony::AccountRequest {
            credential_id: custodian
                .credential_id()
                .map(hex::encode)
                .unwrap_or_default(),
            device_did,
            email: creation.email.clone(),
            device_name: creation.device_name,
            remote: creation.remote.clone(),
            created_on: creation.created_on.clone(),
            encryption_key: account.secret().did().to_string(),
        },
    )
    .await
    .map_err(|error| format!("the account request did not sign: {error:#}"))?;

    // The root record first: it is what every later custody operation
    // resolves the passkey through, and an account created without one
    // is unreachable from a second device.
    {
        let tonk = state.read().await;
        let root_record = tonk_worker_api::SaveRootRequest {
            credential_id: ceremony.root.credential_id.clone(),
            delegation_hex: ceremony.root.delegation_hex.clone(),
            passkey: ceremony.root.passkey.as_ref().map(|passkey| {
                tonk_worker_api::PasskeyMetadata {
                    created_at: passkey.created_at,
                    created_on: passkey.created_on.clone(),
                }
            }),
            encryption_key: ceremony.root.encryption_key.clone(),
        };
        crate::router::identity::persist_root(&tonk, root_record)
            .await
            .map_err(|error| format!("the account root was not recorded: {error}"))?;
    }

    // No account-service request: what it did was claim the address in
    // a table of its own, and enrollment claims the same address in the
    // customer row it already writes — one authority instead of two,
    // and the duplicate refused a second signup for an address the
    // access service had never heard of.
    //
    // The link: until it is written this profile has no account, and
    // everything downstream — enrollment included — reports one as
    // missing.
    link_account(
        state,
        &creation.provider,
        &ceremony.root.root_did,
        &ceremony.root.credential_id,
        &ceremony.root.delegation_hex,
        &creation.remote,
        true,
    )
    .await?;

    enroll(
        state,
        custodian,
        &account,
        Some(creation.email),
        creation.created_on,
    )
    .await
}

/// Record the account the service just accepted, so this profile has
/// one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[allow(clippy::too_many_arguments)]
async fn link_account(
    state: &AppState,
    provider: &str,
    root_did: &str,
    credential_id: &str,
    delegation_hex: &str,
    remote: &str,
    initialize_name: bool,
) -> Result<(), String> {
    let request = tonk_worker_api::AccountLinkRequest {
        provider: provider.to_string(),
        root_did: root_did.to_string(),
        credential_id: credential_id.to_string(),
        delegation_hex: delegation_hex.to_string(),
        remote: remote.to_string(),
        initialize_name,
    };
    let tonk = state.read().await;
    crate::router::account::persist_link(&tonk, &request)
        .await
        .map_err(|error| format!("the account link was not saved: {error}"))
}

/// Rebuild the custodian from the two posted handles.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn custodian_from(
    data: &wasm_bindgen::JsValue,
) -> Result<tonk_identity::custodian::Custodian, String> {
    custodian_named(data, "").ok_or_else(|| "the handoff carried no derivation handles".to_string())
}

/// A custodian from one set of posted handles, by field prefix: `""`
/// for the primary, `"holder"` for the second one an addition carries.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn custodian_named(
    data: &wasm_bindgen::JsValue,
    prefix: &str,
) -> Option<tonk_identity::custodian::Custodian> {
    use wasm_bindgen::{JsCast, JsValue};

    // `key` / `kek` / `credentialId` for the primary set; `holderKey`
    // and friends for the second one an addition carries.
    let field = |name: &str| -> String {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}{}{}", &name[..1].to_uppercase(), &name[1..])
        }
    };
    let handle = |name: &str| -> Option<web_sys::CryptoKey> {
        js_sys::Reflect::get(data, &JsValue::from_str(&field(name)))
            .ok()
            .and_then(|value| value.dyn_into::<web_sys::CryptoKey>().ok())
    };
    let credential_id = js_sys::Reflect::get(data, &JsValue::from_str(&field("credentialId")))
        .ok()
        .and_then(|value| value.as_string())
        .and_then(|value| hex::decode(&value).ok())?;

    Some(tonk_identity::custodian::Custodian::Passkey(
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

    /// The handoff is recognised before anything parses it, because
    /// the handles it carries are `CryptoKey`s and reading the envelope
    /// through `serde_wasm_bindgen` would drop them silently.
    #[dialog_common::test]
    fn it_recognises_a_custody_handoff_on_the_raw_value() {
        use wasm_bindgen::JsValue;

        let envelope = js_sys::Object::new();
        js_sys::Reflect::set(&envelope, &"type".into(), &"custody".into()).unwrap();
        assert!(is_custody_envelope(&envelope.into()));

        let other = js_sys::Object::new();
        js_sys::Reflect::set(&other, &"type".into(), &"hello".into()).unwrap();
        assert!(!is_custody_envelope(&other.into()));

        assert!(!is_custody_envelope(&JsValue::UNDEFINED));
    }

    /// The two posted handles rebuild the custodian, and a handoff
    /// missing either is refused rather than half-built.
    #[dialog_common::test]
    async fn it_rebuilds_the_custodian_from_the_posted_handles() {
        let custodian =
            tonk_identity::webcrypto_kek::Custodian::adopt(vec![7], &[11u8; 32], &[22u8; 32])
                .await
                .expect("handles import");
        let (key, kek) = custodian.handles();

        let envelope = js_sys::Object::new();
        js_sys::Reflect::set(&envelope, &"type".into(), &"custody".into()).unwrap();
        js_sys::Reflect::set(&envelope, &"credentialId".into(), &"07".into()).unwrap();
        js_sys::Reflect::set(&envelope, &"key".into(), key).unwrap();
        js_sys::Reflect::set(&envelope, &"kek".into(), kek).unwrap();
        let envelope: wasm_bindgen::JsValue = envelope.into();

        let rebuilt = custodian_from(&envelope).expect("the handoff rebuilds a custodian");
        // The custody space it names is what a recovering device
        // resolves against, so a drift here strands the account.
        use dialog_varsig::Principal as _;
        assert_eq!(
            rebuilt.signer().await.unwrap().did(),
            custodian.signer().await.unwrap().did(),
        );

        let partial = js_sys::Object::new();
        js_sys::Reflect::set(&partial, &"credentialId".into(), &"07".into()).unwrap();
        js_sys::Reflect::set(&partial, &"key".into(), key).unwrap();
        assert!(
            custodian_from(&partial.into()).is_err(),
            "a handoff missing the KEK handle is refused"
        );
    }

    /// Minting the custody material records the envelope as a fact
    /// before anything is sent, so the account survives a restart that
    /// happens before activation.
    #[dialog_common::test]
    async fn it_records_the_envelope_as_a_fact_when_it_mints_one() {
        use dialog_query::{Output as _, Query, Term};

        let state = Arc::new(RwLock::new(test_state().await));
        let custodian = tonk_identity::custodian::Custodian::Passkey(
            tonk_identity::webcrypto_kek::Custodian::adopt(vec![3], &[13u8; 32], &[23u8; 32])
                .await
                .expect("handles import"),
        );
        let account = open(&custodian).await.expect("an account seals");
        let material = custody_material(&custodian, &account)
            .await
            .expect("the custody set mints");

        {
            let tonk = state.read().await;
            crate::router::customer::record_custody_cell(
                &tonk,
                &material.custody.to_string(),
                &hex::encode(&material.sealed),
                None,
                "",
                None,
            )
            .await
            .expect("the envelope records");
        }

        let tonk = state.read().await;
        let branch = tonk
            .reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("profile main acquires");
        let rows: Vec<tonk_schema::SecretMessage> = branch
            .handle()
            .query()
            .select(Query::<tonk_schema::SecretMessage> {
                this: Term::var("this"),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .expect("the branch answers");

        assert_eq!(rows.len(), 1, "one row per passkey");
        assert_eq!(
            rows[0].message.0, material.sealed,
            "the row carries the sealed envelope, not a reference to one"
        );
    }

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
        let published = super::super::account_state::published_sealed_inbox(&tonk, &root.root_did)
            .await
            .unwrap()
            .expect("the fixture published one");
        tonk.reactor
            .profile_repository()
            .branch(tonk_account::MAIN_BRANCH)
            .transaction()
            .retract(tonk_schema::AccountSealedInbox::new(
                root.root_did.this(),
                published.this(),
            ))
            .commit()
            .perform(&tonk.operator)
            .await
            .unwrap();
    }
}
