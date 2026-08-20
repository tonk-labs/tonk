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
    JsValue::from_str(&format!("{error:#}"))
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

fn root_result(ceremony: crate::ceremony::RootCeremony) -> Result<JsValue, JsValue> {
    let result = Object::new();
    Reflect::set(&result, &"rootDid".into(), &ceremony.root_did.into())?;
    Reflect::set(&result, &"deviceDid".into(), &ceremony.device_did.into())?;
    Reflect::set(
        &result,
        &"credentialId".into(),
        &ceremony.credential_id.into(),
    )?;
    Reflect::set(
        &result,
        &"delegationCid".into(),
        &ceremony.delegation_cid.into(),
    )?;
    Reflect::set(
        &result,
        &"delegationHex".into(),
        &ceremony.delegation_hex.into(),
    )?;
    if let Some(passkey) = ceremony.passkey {
        let metadata = Object::new();
        Reflect::set(
            &metadata,
            &"createdAt".into(),
            &JsValue::from_f64(passkey.created_at as f64),
        )?;
        Reflect::set(&metadata, &"createdOn".into(), &passkey.created_on.into())?;
        Reflect::set(&result, &"passkey".into(), &metadata)?;
    }
    Ok(result.into())
}

fn ceremony_result(ceremony: crate::ceremony::AccountCeremony) -> Result<JsValue, JsValue> {
    let result = Object::new();
    Reflect::set(&result, &"rootDid".into(), &ceremony.root_did.into())?;
    Reflect::set(&result, &"deviceDid".into(), &ceremony.device_did.into())?;
    Reflect::set(
        &result,
        &"delegationHex".into(),
        &ceremony.delegation_hex.into(),
    )?;
    if let Some(descriptor_hex) = ceremony.descriptor_hex {
        Reflect::set(&result, &"descriptorHex".into(), &descriptor_hex.into())?;
    }
    Reflect::set(
        &result,
        &"invocationHex".into(),
        &ceremony.invocation_hex.into(),
    )?;
    Ok(result.into())
}

/// Parse the optional access-service DID a ceremony mints deposits for.
fn service_did_property(input: &JsValue) -> Result<Option<dialog_varsig::Did>, JsValue> {
    optional_string_property(input, "serviceDid")
        .map(|value| {
            value
                .parse()
                .map_err(|error| JsValue::from_str(&format!("invalid serviceDid: {error}")))
        })
        .transpose()
}

/// Attach hex-encoded deposits to a ceremony result as `depositsHex`.
fn set_deposits(result: &JsValue, deposits: &[String]) -> Result<(), JsValue> {
    let array = js_sys::Array::new();
    for deposit in deposits {
        array.push(&JsValue::from_str(deposit));
    }
    Reflect::set(result, &"depositsHex".into(), &array)?;
    Ok(())
}

/// `createAccount({ email, deviceDid, deviceName, remote, endpoint,
/// createdOn?, serviceDid? })` → the account-creation artifacts plus
/// `custodyDid` and `consentHex` for provisioning the custody space.
/// One ceremony: secret, custody passkey, published cell, signed
/// creation request.
async fn create_account(input: JsValue) -> Result<JsValue, JsValue> {
    let email = string_property(&input, "email")?;
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let device_name = string_property(&input, "deviceName")?;
    let remote = string_property(&input, "remote")?;
    let created_on = optional_string_property(&input, "createdOn");
    let service = service_did_property(&input)?;
    let ceremony = crate::ceremony::create_custody_account(
        email,
        device_did,
        device_name,
        remote,
        created_on.as_deref(),
        service.as_ref(),
    )
    .await
    .map_err(js_error)?;
    let result = root_result(ceremony.root)?;
    Reflect::set(
        &result,
        &"invocationHex".into(),
        &ceremony.account.invocation_hex.into(),
    )?;
    if let Some(descriptor_hex) = ceremony.account.descriptor_hex {
        Reflect::set(&result, &"descriptorHex".into(), &descriptor_hex.into())?;
    }
    set_deposits(&result, &ceremony.deposits_hex)?;
    Reflect::set(&result, &"custodyDid".into(), &ceremony.custody_did.into())?;
    Reflect::set(&result, &"consentHex".into(), &ceremony.consent_hex.into())?;
    if let Some(sealed_hex) = ceremony.sealed_hex {
        Reflect::set(&result, &"sealedHex".into(), &sealed_hex.into())?;
    }
    Ok(result)
}

/// `enrollCustodyPasskey({ accountDid, label?, endpoint })` →
/// `{ custodyDid, credentialId, consentHex, sealedHex? }`. Creates the
/// custody passkey and seals the secret under its KEK. The caller
/// provisions the custody DID with the consent; `sealedHex` is present
/// when the cell could not be published yet — the new custody DID is
/// nobody's consumer until that provisioning lands — and the caller
/// queues those bytes to publish afterwards.
async fn enroll_custody_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let account_did = string_property(&input, "accountDid")?;
    let label = optional_string_property(&input, "label");
    let endpoint = string_property(&input, "endpoint")?;
    let enrollment = crate::ceremony::enroll_custody(&account_did, label.as_deref(), &endpoint)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(
        &result,
        &"custodyDid".into(),
        &enrollment.custody_did.into(),
    )?;
    Reflect::set(
        &result,
        &"credentialId".into(),
        &enrollment.credential_id.into(),
    )?;
    Reflect::set(
        &result,
        &"consentHex".into(),
        &enrollment.consent_hex.into(),
    )?;
    if let Some(sealed_hex) = enrollment.sealed_hex {
        Reflect::set(&result, &"sealedHex".into(), &sealed_hex.into())?;
    }
    Ok(result.into())
}

/// `publishQueuedCustody({ custodyDid, sealedHex, endpoint })` → `{}`.
/// Publishes a custody cell that could not be written when its
/// ceremony ran, using a fresh assertion of the same passkey.
async fn publish_queued_custody(input: JsValue) -> Result<JsValue, JsValue> {
    let custody_did = string_property(&input, "custodyDid")?;
    let sealed_hex = string_property(&input, "sealedHex")?;
    let endpoint = string_property(&input, "endpoint")?;
    let sealed = hex::decode(&sealed_hex)
        .map_err(|error| JsValue::from_str(&format!("sealedHex is not hex: {error}")))?;
    crate::ceremony::publish_queued_custody(&custody_did, &sealed, &endpoint)
        .await
        .map_err(js_error)?;
    Ok(Object::new().into())
}

/// `unlockWithPasskey({ deviceDid, deviceName, endpoint, serviceDid? })`
/// → the `linkDevice` result shape. One assertion, one presigned GET,
/// and the unwrapped secret self-issues the device delegation.
async fn unlock_with_passkey(input: JsValue) -> Result<JsValue, JsValue> {
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let device_name = string_property(&input, "deviceName")?;
    let endpoint = string_property(&input, "endpoint")?;
    let service = service_did_property(&input)?;
    let unlock =
        crate::ceremony::unlock_account(device_did, device_name, &endpoint, service.as_ref())
            .await
            .map_err(js_error)?;
    let result = ceremony_result(unlock.account)?;
    Reflect::set(
        &result,
        &"credentialId".into(),
        &unlock.credential_id.into(),
    )?;
    set_deposits(&result, &unlock.deposits_hex)?;
    Ok(result)
}

fn parse_complete_link_input(
    input: JsValue,
) -> Result<tonk_account::handoff::ResolvedLink, JsValue> {
    let input = serde_wasm_bindgen::from_value::<tonk_account::handoff::ResolvedLink>(input)
        .map_err(|_| JsValue::from_str("malformed completeLink input"))?;
    if input.token_hash.is_empty() {
        return Err(JsValue::from_str("missing or invalid tokenHash"));
    }
    if input.device_name.is_empty() {
        return Err(JsValue::from_str("missing or invalid deviceName"));
    }
    if input.device_did.is_empty() {
        return Err(JsValue::from_str("missing or invalid deviceDid"));
    }
    input
        .device_did
        .parse::<dialog_varsig::Did>()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    Ok(input)
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

async fn complete_link(input: JsValue) -> Result<JsValue, JsValue> {
    let endpoint = string_property(&input, "endpoint")?;
    let input = parse_complete_link_input(input)?;
    let device_did = input
        .device_did
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let root = crate::ceremony::unlock_root(&endpoint)
        .await
        .map_err(js_error)?;
    let ceremony =
        crate::ceremony::complete_link(root, input.token_hash, device_did, input.device_name)
            .await
            .map_err(js_error)?;
    serde_wasm_bindgen::to_value(&tonk_account::handoff::CompleteLinkCeremony {
        invocation_hex: ceremony.invocation_hex,
    })
    .map_err(|_| JsValue::from_str("failed to serialize completeLink output"))
}

/// Install `window.tonkIdentity` on the page. Idempotent; a no-op
/// outside a window context.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let identity = Object::new();

    let create_account = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(create_account(input))
    });
    let _ = Reflect::set(
        &identity,
        &"createAccount".into(),
        create_account.as_ref().unchecked_ref(),
    );
    create_account.forget();

    let enroll_custody_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(enroll_custody_passkey(input))
    });
    let _ = Reflect::set(
        &identity,
        &"enrollCustodyPasskey".into(),
        enroll_custody_passkey.as_ref().unchecked_ref(),
    );
    enroll_custody_passkey.forget();

    let publish_queued = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(publish_queued_custody(input))
    });
    let _ = Reflect::set(
        &identity,
        &"publishQueuedCustody".into(),
        publish_queued.as_ref().unchecked_ref(),
    );
    publish_queued.forget();

    let unlock_with_passkey = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(unlock_with_passkey(input))
    });
    let _ = Reflect::set(
        &identity,
        &"unlockWithPasskey".into(),
        unlock_with_passkey.as_ref().unchecked_ref(),
    );
    unlock_with_passkey.forget();

    let complete_link = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(complete_link(input))
    });
    let _ = Reflect::set(
        &identity,
        &"completeLink".into(),
        complete_link.as_ref().unchecked_ref(),
    );
    complete_link.forget();

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

    let _ = Reflect::set(&window, &"tonkIdentity".into(), &identity.into());
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use super::*;
    use js_sys::Reflect;
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_browser);

    fn complete_link_input(
        token_hash: JsValue,
        device_did: JsValue,
        device_name: JsValue,
    ) -> JsValue {
        let input = Object::new();
        Reflect::set(&input, &"tokenHash".into(), &token_hash).unwrap();
        Reflect::set(&input, &"deviceDid".into(), &device_did).unwrap();
        Reflect::set(&input, &"deviceName".into(), &device_name).unwrap();
        input.into()
    }

    fn parse_error(input: JsValue) -> String {
        parse_complete_link_input(input)
            .unwrap_err()
            .as_string()
            .expect("parser errors are stable strings")
    }

    #[dialog_common::test]
    async fn it_rejects_invalid_complete_link_input_before_the_ceremony() {
        use dialog_varsig::Principal;

        assert_eq!(
            parse_error(Object::new().into()),
            "malformed completeLink input"
        );
        assert_eq!(
            parse_error(complete_link_input(
                "".into(),
                "did:key:unused".into(),
                "terminal".into(),
            )),
            "missing or invalid tokenHash"
        );
        assert_eq!(
            parse_error(complete_link_input(
                "hash".into(),
                "did:key:unused".into(),
                "".into(),
            )),
            "missing or invalid deviceName"
        );
        assert_eq!(
            parse_error(complete_link_input(
                "hash".into(),
                "".into(),
                "terminal".into(),
            )),
            "missing or invalid deviceDid"
        );
        assert!(
            parse_error(complete_link_input(
                "hash".into(),
                "not a DID".into(),
                "terminal".into(),
            ))
            .starts_with("invalid deviceDid: ")
        );

        let device = dialog_credentials::Ed25519Signer::import(&[8u8; 32])
            .await
            .unwrap();
        let valid = tonk_account::handoff::ResolvedLink {
            token_hash: "hash".to_string(),
            device_did: device.did().to_string(),
            device_name: "terminal".to_string(),
        };
        assert_eq!(
            parse_complete_link_input(serde_wasm_bindgen::to_value(&valid).unwrap()).unwrap(),
            valid
        );
    }

    #[dialog_common::test]
    fn it_installs_ceremony_functions_on_window_tonk_identity() {
        install();
        let window = web_sys::window().unwrap();
        let identity = Reflect::get(&window, &"tonkIdentity".into()).unwrap();
        for name in [
            "createAccount",
            "enrollCustodyPasskey",
            "unlockWithPasskey",
            "completeLink",
            "authorizeDevice",
            "signRevocation",
            "verifyPasskey",
        ] {
            let function = Reflect::get(&identity, &name.into()).unwrap();
            assert!(function.is_function(), "{name} must be a function");
        }
    }
}
