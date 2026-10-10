//! The `window.tonkIdentity` ceremony hook.
//!
//! WebAuthn only exists on the window, so the ceremony surface installs
//! from the page main thread. Its two transient PRF outputs cross to the
//! service worker as typed arrays and are imported there immediately.
//! Installed as JS functions (rather than Rust-only API) so
//! WebDriver-driven tests and future non-wasm callers (the CLI linking
//! handoff page) can invoke ceremonies directly.
//!
//! This installs as its own global, `window.tonkIdentity`, and
//! deliberately never touches `window.tonk`. The top page must not carry
//! a `window.tonk` object: tonk-host's page-effect forwarding treats the
//! bare presence of `window.tonk` as the signal that the current document
//! is a portal guest with a bridge to its parent, rather than the page
//! itself. Creating `window.tonk` here would make the top page look like
//! a guest to that check, and every page effect (navigate, set title,
//! open) would silently stop working.

use crate::handoff::wait_for_custody_reply;
use js_sys::{Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

const CUSTODY_HANDOFF_TIMEOUT_MS: i32 = 30_000;
#[cfg(test)]
const CUSTODY_HANDOFF_TIMEOUT: &str =
    "the service worker did not answer the custody handoff in time";

fn js_error(error: anyhow::Error) -> JsValue {
    // A ceremony refusal carries its DOM error name as a variant; hand
    // that back as a `name` property so a caller can tell a dismissed
    // prompt from a real failure without matching on prose.
    let name = error
        .downcast_ref::<crate::passkey::CeremonyError>()
        .map(|refusal| refusal.reason.as_str());
    let value = js_sys::Error::new(&format!("{error:#}"));
    if let Some(name) = name {
        value.set_name(name);
    }
    value.into()
}

/// An optional string property: absent, empty, or not a string all read as
/// `None`, so a caller with nothing to say can simply say nothing.
fn optional_string_property(input: &JsValue, name: &str) -> Option<String> {
    Reflect::get(input, &name.into())
        .ok()?
        .as_string()
        .filter(|value| !value.is_empty())
}

/// `createPasskey({ name?, displayName? })` → `{ credentialId }`.
///
/// Creates a custody passkey, evaluates its PRF, and hands the service
/// worker the two PRF outputs. Nothing but the credential id comes back
/// to the caller: the worker imports non-extractable derivation handles
/// immediately and remains the only place that mints anything.
async fn create_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let name = optional_string_property(&input, "name");
    let display_name = optional_string_property(&input, "displayName");
    let credential =
        crate::passkey::create_custody_passkey(name.as_deref(), display_name.as_deref())
            .await
            .map_err(js_error)?;
    let credential = credential.into_evaluated().await.map_err(js_error)?;
    let request = Reflect::get(&input, &"request".into()).unwrap_or(JsValue::UNDEFINED);
    mediate(credential, request).await
}

/// `addPasskey({ name?, displayName?, request })` → `{ credentialId }`.
///
/// Two ceremonies in one call: assert the passkey that already holds
/// the account, then create the one being added. Both pairs of PRF
/// outputs go to the worker, which is the only place the account secret
/// is opened and re-sealed.
async fn add_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let holder = crate::passkey::evaluate_custody_passkey(None)
        .await
        .map_err(js_error)?;
    let holder = holder.into_evaluated().await.map_err(js_error)?;
    let name = optional_string_property(&input, "name");
    let display_name = optional_string_property(&input, "displayName");
    let added = crate::passkey::create_custody_passkey(name.as_deref(), display_name.as_deref())
        .await
        .map_err(js_error)?;
    let added = added.into_evaluated().await.map_err(js_error)?;
    let request = Reflect::get(&input, &"request".into()).unwrap_or(JsValue::UNDEFINED);
    mediate_pair(added, Some(holder), request).await
}

/// `usePasskey({ credentialId? })` → `{ credentialId }`.
///
/// One assertion against an existing passkey — a picker when no
/// `credentialId` is given — then the same handoff [`create_passkey`]
/// does.
fn use_passkey(input: JsValue) -> Promise {
    // Parse and open WebAuthn before returning to the click handler. Wrapping
    // this whole function in `future_to_promise` used to defer
    // `credentials.get()` until a later microtask, after mobile browsers had
    // cleared the tap's transient user activation.
    let credential_id = match optional_string_property(&input, "credentialId") {
        Some(encoded) => match hex::decode(&encoded) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                return Promise::reject(&JsValue::from_str(&format!(
                    "invalid credentialId: {error}"
                )));
            }
        },
        None => None,
    };
    let assertion = match crate::passkey::begin_evaluate_custody_passkey(credential_id.as_deref()) {
        Ok(assertion) => assertion,
        Err(error) => return Promise::reject(&js_error(error)),
    };
    future_to_promise(async move {
        let credential = assertion.finish().await.map_err(js_error)?;
        let credential = credential.into_evaluated().await.map_err(js_error)?;
        let request = Reflect::get(&input, &"request".into()).unwrap_or(JsValue::UNDEFINED);
        mediate(credential, request).await
    })
}

/// Hand a credential's two PRF outputs to the service worker and wait
/// for it to finish.
///
/// The page's whole job. Fresh fixed-length typed arrays survive service
/// worker messaging on every supported browser. The sender clears its
/// copies immediately after the synchronous post; the worker clears its
/// structured-clone copies after importing non-extractable HKDF handles.
///
/// A fresh `MessageChannel` per call carries the reply. The worker
/// drops the handles as soon as it is done, so the page must know when
/// that is; a port answers exactly one request and needs no correlation
/// id to do it.
async fn mediate(
    credential: crate::passkey::EvaluatedCustodyCredential,
    request: JsValue,
) -> Result<JsValue, JsValue> {
    mediate_pair(credential, None, request).await
}

/// Post a custody hand-off to the worker that holds the account. Where the
/// profile renders on an origin of its own that is the profile's worker,
/// reached through the profile's frame on this page, which installs
/// `tonkProfileWorker` to do it. Otherwise it is this page's own worker.
fn hand_to_custodian(message: &JsValue, transfer: &js_sys::Array) -> Result<(), JsValue> {
    let global = js_sys::global();
    if let Ok(relay) = Reflect::get(&global, &"tonkProfileWorker".into())
        .and_then(|relay| relay.dyn_into::<js_sys::Function>())
    {
        return relay.call2(&global, message, transfer).map(|_| ());
    }
    web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .navigator()
        .service_worker()
        .controller()
        .ok_or_else(|| JsValue::from_str("no service worker controls this page"))?
        .post_message_with_transferable(message, transfer)
}

/// [`mediate`], optionally carrying a second custodian: the passkey
/// that already holds the account, for work that must open it before
/// sealing under the first.
async fn mediate_pair(
    credential: crate::passkey::EvaluatedCustodyCredential,
    holder: Option<crate::passkey::EvaluatedCustodyCredential>,
    request: JsValue,
) -> Result<JsValue, JsValue> {
    let channel = web_sys::MessageChannel::new()?;
    let key = Uint8Array::from(&credential.evaluation.key[..]);
    let kek = Uint8Array::from(&credential.evaluation.kek[..]);

    let message = Object::new();
    Reflect::set(&message, &"type".into(), &"custody".into())?;
    // Old pages understand terminal replies only. Negotiate progress rather
    // than letting a newer worker accidentally resolve an older page early.
    let approval = Reflect::get(&request, &"kind".into())
        .ok()
        .and_then(|value| value.as_string())
        .as_deref()
        == Some("authorize-device");
    Reflect::set(&message, &"progress".into(), &JsValue::from_bool(approval))?;
    Reflect::set(
        &message,
        &"credentialId".into(),
        &hex::encode(&credential.id).into(),
    )?;
    Reflect::set(&message, &"key".into(), &key)?;
    Reflect::set(&message, &"kek".into(), &kek)?;
    Reflect::set(&message, &"request".into(), &request)?;
    let holder_arrays = if let Some(holder) = &holder {
        let holder_key = Uint8Array::from(&holder.evaluation.key[..]);
        let holder_kek = Uint8Array::from(&holder.evaluation.kek[..]);
        Reflect::set(
            &message,
            &"holderCredentialId".into(),
            &hex::encode(&holder.id).into(),
        )?;
        Reflect::set(&message, &"holderKey".into(), &holder_key)?;
        Reflect::set(&message, &"holderKek".into(), &holder_kek)?;
        Some((holder_key, holder_kek))
    } else {
        None
    };

    let transfer = js_sys::Array::new();
    transfer.push(&channel.port2());
    let posted = hand_to_custodian(&message, &transfer);

    // Structured clone has taken the receiver's copies before postMessage
    // returns. Clear every page-side typed array whether posting succeeded
    // or threw; the Zeroizing Rust arrays are cleared when their credentials
    // leave this function.
    key.fill(0, 0, key.length());
    kek.fill(0, 0, kek.length());
    if let Some((holder_key, holder_kek)) = &holder_arrays {
        holder_key.fill(0, 0, holder_key.length());
        holder_kek.fill(0, 0, holder_kek.length());
    }
    posted?;

    wasm_bindgen_futures::JsFuture::from(wait_for_custody_reply(
        channel.port1(),
        CUSTODY_HANDOFF_TIMEOUT_MS,
    ))
    .await
}

/// Install `window.tonkIdentity` on the page. Idempotent; a no-op
/// outside a window context.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let identity = Object::new();

    let create_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(create_passkey(input))
    });
    let _ = Reflect::set(
        &identity,
        &"createPasskey".into(),
        create_passkey.as_ref().unchecked_ref(),
    );
    create_passkey.forget();

    let use_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(use_passkey);
    let _ = Reflect::set(
        &identity,
        &"usePasskey".into(),
        use_passkey.as_ref().unchecked_ref(),
    );
    use_passkey.forget();

    let add_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(add_passkey(input))
    });
    let _ = Reflect::set(
        &identity,
        &"addPasskey".into(),
        add_passkey.as_ref().unchecked_ref(),
    );
    add_passkey.forget();

    let _ = Reflect::set(&window, &"tonkIdentity".into(), &identity.into());
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Reflect;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_installs_ceremony_functions_on_window_tonk_identity() {
        install();
        let window = web_sys::window().unwrap();
        let identity = Reflect::get(&window, &"tonkIdentity".into()).unwrap();
        for name in ["createPasskey", "usePasskey", "addPasskey"] {
            let function = Reflect::get(&identity, &name.into()).unwrap();
            assert!(function.is_function(), "{name} must be a function");
        }
        // The page asks for passkeys and hands what they yield to the
        // worker. Nothing here opens the account or signs with it.
        for name in ["authorizeDevice", "signRevocation", "publishEncryptionKey"] {
            assert!(
                Reflect::get(&identity, &name.into())
                    .unwrap()
                    .is_undefined(),
                "{name} would sign with the account on the page"
            );
        }
    }

    #[dialog_common::test]
    async fn it_times_out_when_the_worker_does_not_reply() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let error =
            wasm_bindgen_futures::JsFuture::from(wait_for_custody_reply(channel.port1(), 10))
                .await
                .expect_err("an unanswered custody handoff must reject");
        let message = Reflect::get(&error, &"message".into())
            .unwrap()
            .as_string()
            .unwrap();
        assert_eq!(message, CUSTODY_HANDOFF_TIMEOUT);
        assert!(channel.port1().onmessage().is_none());
    }

    async fn delay(ms: i32) {
        let promise = Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    #[dialog_common::test]
    async fn it_waits_for_slow_work_while_the_worker_remains_responsive() {
        let channel = web_sys::MessageChannel::new().unwrap();
        let reply = wait_for_custody_reply(channel.port1(), 100);
        let worker = channel.port2();
        let progress = Object::new();
        Reflect::set(&progress, &"pending".into(), &JsValue::TRUE).unwrap();
        worker.post_message(&progress).unwrap();
        let completing = future_to_promise(async move {
            // Longer than the original deadline, with live worker progress.
            for _ in 0..8 {
                delay(25).await;
                worker.post_message(&progress).unwrap();
            }
            let result = Object::new();
            Reflect::set(&result, &"ok".into(), &JsValue::from_str("completed")).unwrap();
            worker.post_message(&result).unwrap();
            Ok(JsValue::UNDEFINED)
        });
        let result = wasm_bindgen_futures::JsFuture::from(reply).await;
        wasm_bindgen_futures::JsFuture::from(completing)
            .await
            .unwrap();
        let result = result.expect("responsive work must retain its completion listener");
        assert_eq!(
            Reflect::get(&result, &"ok".into()).unwrap().as_string(),
            Some("completed".into()),
            "progress is not completion"
        );
        assert!(channel.port1().onmessage().is_none());
    }
}
