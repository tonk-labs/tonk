//! The `window.tonkIdentity` ceremony hook.
//!
//! WebAuthn only exists on the window, so the ceremony surface installs
//! from the page main thread; the service worker never sees key
//! material. Installed as JS functions (rather than Rust-only API) so
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

use js_sys::{Object, Promise, Reflect};
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

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

fn string_property(input: &JsValue, name: &str) -> Result<String, JsValue> {
    Reflect::get(input, &name.into())?
        .as_string()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| JsValue::from_str(&format!("missing or invalid {name}")))
}

/// `createPasskey({ name?, displayName? })` → `{ credentialId }`.
///
/// Creates a custody passkey, evaluates its PRF, and hands the service
/// worker the two derivation handles. Nothing but the credential id
/// comes back to the caller: the handles go straight to the worker,
/// which is the only place that mints anything.
async fn create_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let name = optional_string_property(&input, "name");
    let display_name = optional_string_property(&input, "displayName");
    let custodian = crate::webcrypto_kek::Custodian::create(name, display_name)
        .perform(&crate::webcrypto_kek::Page)
        .await
        .map_err(js_error)?;
    let request = Reflect::get(&input, &"request".into()).unwrap_or(JsValue::UNDEFINED);
    mediate(custodian, request).await
}

/// `addPasskey({ name?, displayName?, request })` → `{ credentialId }`.
///
/// Two ceremonies in one call: assert the passkey that already holds
/// the account, then create the one being added. Both sets of handles
/// go to the worker, which is the only place the account secret is
/// opened and re-sealed.
async fn add_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let holder = crate::webcrypto_kek::Custodian::choose()
        .perform(&crate::webcrypto_kek::Page)
        .await
        .map_err(js_error)?;
    let name = optional_string_property(&input, "name");
    let display_name = optional_string_property(&input, "displayName");
    let added = crate::webcrypto_kek::Custodian::create(name, display_name)
        .perform(&crate::webcrypto_kek::Page)
        .await
        .map_err(js_error)?;
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
        let custodian = crate::webcrypto_kek::Custodian::from_credential(credential)
            .await
            .map_err(js_error)?;
        let request = Reflect::get(&input, &"request".into()).unwrap_or(JsValue::UNDEFINED);
        mediate(custodian, request).await
    })
}

/// Hand a custodian's two derivation handles to the service worker and
/// wait for it to finish.
///
/// The page's whole job. `structuredClone` — which is what
/// `postMessage` runs — carries a non-extractable `CryptoKey` intact,
/// so the worker receives derivation capability without either side
/// ever holding bytes.
///
/// A fresh `MessageChannel` per call carries the reply. The worker
/// drops the handles as soon as it is done, so the page must know when
/// that is; a port answers exactly one request and needs no correlation
/// id to do it.
async fn mediate(
    custodian: crate::webcrypto_kek::Custodian,
    request: JsValue,
) -> Result<JsValue, JsValue> {
    mediate_pair(custodian, None, request).await
}

/// [`mediate`], optionally carrying a second custodian: the passkey
/// that already holds the account, for work that must open it before
/// sealing under the first.
async fn mediate_pair(
    custodian: crate::webcrypto_kek::Custodian,
    holder: Option<crate::webcrypto_kek::Custodian>,
    request: JsValue,
) -> Result<JsValue, JsValue> {
    let (key, kek) = custodian.handles();
    let channel = web_sys::MessageChannel::new()?;

    let message = Object::new();
    Reflect::set(&message, &"type".into(), &"custody".into())?;
    Reflect::set(
        &message,
        &"credentialId".into(),
        &hex::encode(&custodian.credential_id).into(),
    )?;
    Reflect::set(&message, &"key".into(), key)?;
    Reflect::set(&message, &"kek".into(), kek)?;
    Reflect::set(&message, &"request".into(), &request)?;
    if let Some(holder) = &holder {
        let (holder_key, holder_kek) = holder.handles();
        Reflect::set(
            &message,
            &"holderCredentialId".into(),
            &hex::encode(&holder.credential_id).into(),
        )?;
        Reflect::set(&message, &"holderKey".into(), holder_key)?;
        Reflect::set(&message, &"holderKek".into(), holder_kek)?;
    }

    let answer = js_sys::Promise::new(&mut |resolve, reject| {
        let port = channel.port1();
        let on_message = Closure::once_into_js(move |event: web_sys::MessageEvent| {
            let data = event.data();
            match Reflect::get(&data, &"error".into())
                .ok()
                .and_then(|e| e.as_string())
            {
                Some(error) => {
                    // An `Error`, not a bare string: the worker says WHY
                    // the service refused in a `code` beside the message,
                    // and a rejection that carried only the sentence left
                    // the page matching prose to tell "confirm your email"
                    // from "check your connection".
                    let failure = js_sys::Error::new(&error);
                    if let Some(code) = Reflect::get(&data, &"code".into())
                        .ok()
                        .and_then(|code| code.as_string())
                    {
                        let _ = Reflect::set(failure.as_ref(), &"code".into(), &code.into());
                    }
                    let _ = reject.call1(&JsValue::NULL, failure.as_ref());
                }
                None => {
                    let _ = resolve.call1(&JsValue::NULL, &data);
                }
            }
        });
        port.set_onmessage(Some(on_message.unchecked_ref()));
    });

    let worker = web_sys::window()
        .ok_or_else(|| JsValue::from_str("no window"))?
        .navigator()
        .service_worker()
        .controller()
        .ok_or_else(|| JsValue::from_str("no service worker controls this page"))?;
    let transfer = js_sys::Array::new();
    transfer.push(&channel.port2());
    worker.post_message_with_transferable(&message, &transfer)?;

    let result = wasm_bindgen_futures::JsFuture::from(answer).await?;
    channel.port1().set_onmessage(None);
    Ok(result)
}

/// `signRevocation({ delegationCid, pathHex, endpoint })` →
/// `{ revocationHex }`.
///
/// Parses the public witness before prompting, unlocks the account
/// through a custody assertion, and signs only when that root issued a
/// delegation in the target's path prefix.
async fn sign_revocation(input: JsValue) -> Result<JsValue, JsValue> {
    let delegation_cid = string_property(&input, "delegationCid")?;
    let target = delegation_cid
        .parse::<ipld_core::cid::Cid>()
        .map_err(|error| JsValue::from_str(&format!("invalid delegationCid: {error}")))?;
    if target.to_string() != delegation_cid {
        return Err(JsValue::from_str("delegationCid must be canonical"));
    }
    let path_hex = string_property(&input, "pathHex")?;
    let path_bytes = hex::decode(path_hex)
        .map_err(|error| JsValue::from_str(&format!("invalid pathHex: {error}")))?;
    let path = dialog_ucan_core::DelegationChain::try_from(path_bytes.as_slice())
        .map_err(|error| JsValue::from_str(&format!("invalid revocation path: {error}")))?;
    if path
        .proof_cids()
        .iter()
        .filter(|cid| **cid == target)
        .count()
        != 1
    {
        return Err(JsValue::from_str(
            "revocation path must contain delegationCid exactly once",
        ));
    }

    let endpoint = string_property(&input, "endpoint")?;
    let root = crate::ceremony::unlock_root(&endpoint)
        .await
        .map_err(js_error)?;
    let revocation_hex = crate::ceremony::sign_revocation(root, &path, &target)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(&result, &"revocationHex".into(), &revocation_hex.into())?;
    Ok(result.into())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyPasskeyInput {
    credential_id: String,
}

/// A user-verification assertion against the account's own passkey,
/// with nothing derived and nothing signed: destructive account
/// invocations sign with the device's delegated authority, and this
/// ceremony only proves a human holding the passkey is present.
async fn verify_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let input: VerifyPasskeyInput = serde_wasm_bindgen::from_value(input)
        .map_err(|_| JsValue::from_str("malformed passkey verification input"))?;
    let credential_id = hex::decode(&input.credential_id)
        .map_err(|_| JsValue::from_str("malformed passkey credential id"))?;
    crate::passkey::verify_custody_passkey(&credential_id)
        .await
        .map_err(js_error)?;
    // The bridge decodes this into `()`, which maps to `undefined`.
    Ok(JsValue::UNDEFINED)
}

/// `publishEncryptionKey({ endpoint, credentialId? })` → `{ encryptionKey }`:
/// one assertion — pinned to `credentialId` (hex) when the root record
/// carries one — and the account's X25519 recipient. The page saves it
/// with the root so the worker can set up custody for what it creates.
async fn publish_encryption_key(input: JsValue) -> Result<JsValue, JsValue> {
    let endpoint = string_property(&input, "endpoint")?;
    let credential_id = credential_id_property(&input)?;
    let key = crate::ceremony::publish_encryption_key(&endpoint, credential_id.as_deref())
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(&result, &"encryptionKey".into(), &key.into())?;
    Ok(result.into())
}

/// The optional `credentialId` property, hex-decoded.
fn credential_id_property(input: &JsValue) -> Result<Option<Vec<u8>>, JsValue> {
    optional_string_property(input, "credentialId")
        .map(|hex| {
            hex::decode(&hex)
                .map_err(|error| JsValue::from_str(&format!("credentialId is not hex: {error}")))
        })
        .transpose()
}

/// `authorizeDevice({ deviceDid, remote, endpoint })` → `{ rootDid,
/// deviceDid, delegationHex, descriptorHex }`.
///
/// The callback authorization: unlock the account through a custody
/// assertion, mint the `account → device` powerline, and hand it back
/// with the account repository descriptor. Nothing is sent anywhere —
/// the caller delivers it.
async fn authorize_device(input: JsValue) -> Result<JsValue, JsValue> {
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let remote = string_property(&input, "remote")?;
    let endpoint = string_property(&input, "endpoint")?;
    let root = crate::ceremony::unlock_root(&endpoint)
        .await
        .map_err(js_error)?;
    let authorized = crate::ceremony::authorize_device(root, device_did, &remote)
        .await
        .map_err(js_error)?;

    let output = js_sys::Object::new();
    for (key, value) in [
        ("rootDid", authorized.root_did),
        ("deviceDid", authorized.device_did),
        ("delegationHex", authorized.delegation_hex),
        ("descriptorHex", authorized.descriptor_hex),
    ] {
        Reflect::set(&output, &key.into(), &value.into())?;
    }
    Ok(output.into())
}

/// Install `window.tonkIdentity` on the page. Idempotent; a no-op
/// outside a window context.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let identity = Object::new();

    let publish_encryption_key = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(publish_encryption_key(input))
    });
    let _ = Reflect::set(
        &identity,
        &"publishEncryptionKey".into(),
        publish_encryption_key.as_ref().unchecked_ref(),
    );
    publish_encryption_key.forget();

    let authorize_device = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(authorize_device(input))
    });
    let _ = Reflect::set(
        &identity,
        &"authorizeDevice".into(),
        authorize_device.as_ref().unchecked_ref(),
    );
    authorize_device.forget();

    let sign_revocation = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(sign_revocation(input))
    });
    let _ = Reflect::set(
        &identity,
        &"signRevocation".into(),
        sign_revocation.as_ref().unchecked_ref(),
    );
    sign_revocation.forget();

    let verify_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(verify_passkey(input))
    });
    let _ = Reflect::set(
        &identity,
        &"verifyPasskey".into(),
        verify_passkey.as_ref().unchecked_ref(),
    );
    verify_passkey.forget();

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
        for name in [
            "createPasskey",
            "usePasskey",
            "addPasskey",
            "authorizeDevice",
            "signRevocation",
            "verifyPasskey",
        ] {
            let function = Reflect::get(&identity, &name.into()).unwrap();
            assert!(function.is_function(), "{name} must be a function");
        }
    }
}
