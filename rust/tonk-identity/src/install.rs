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

async fn create() -> Result<JsValue, JsValue> {
    // No account context here, so the credential stays unlabelled.
    let created = crate::passkey::create_passkey(None)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(
        &result,
        &"credentialId".into(),
        &hex::encode(&created.id).into(),
    )?;
    Reflect::set(
        &result,
        &"prfAtCreate".into(),
        &created.prf_output.is_some().into(),
    )?;
    Ok(result.into())
}

async fn derive_root_did() -> Result<JsValue, JsValue> {
    use dialog_varsig::Principal;
    let prf = crate::passkey::prf_output().await.map_err(js_error)?;
    let signer = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    let did = signer.did();
    Ok(JsValue::from_str(did.as_ref()))
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

/// `signRevocation({ delegationCid, pathHex })` → `{ revocationHex }`.
///
/// Parses the public witness before prompting, derives the root, and signs
/// only when that root issued a delegation in the target's path prefix.
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

    let prf = crate::passkey::prf_output().await.map_err(js_error)?;
    let root = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    let revocation_hex = crate::ceremony::sign_revocation(root, &path, &target)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(&result, &"revocationHex".into(), &revocation_hex.into())?;
    Ok(result.into())
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
    Ok(result.into())
}

async fn create_root(input: JsValue) -> Result<JsValue, JsValue> {
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    // `label` is what the passkey manager will show. The account ceremony
    // sends the address it just verified; a spot creating a root sends
    // nothing, because no account exists to name.
    let label = optional_string_property(&input, "label");
    root_result(
        crate::ceremony::create_root(device_did, label.as_deref())
            .await
            .map_err(js_error)?,
    )
}

async fn evaluate_root(input: JsValue) -> Result<JsValue, JsValue> {
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    root_result(
        crate::ceremony::evaluate_root(device_did)
            .await
            .map_err(js_error)?,
    )
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

async fn create_account(input: JsValue) -> Result<JsValue, JsValue> {
    let email = string_property(&input, "email")?;
    let code = string_property(&input, "code")?;
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let device_name = string_property(&input, "deviceName")?;
    let expected_root = string_property(&input, "rootDid")?;
    let credential_id = string_property(&input, "credentialId")?;
    let delegation_hex = string_property(&input, "delegationHex")?;
    let evaluated = crate::passkey::evaluate_passkey().await.map_err(js_error)?;
    if hex::encode(evaluated.id) != credential_id {
        return Err(JsValue::from_str(
            "the evaluated passkey does not match credentialId",
        ));
    }
    let prf = evaluated
        .prf_output
        .ok_or_else(|| JsValue::from_str("the authenticator returned no PRF output"))?;
    let remote = string_property(&input, "remote")?;
    let root = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    use dialog_varsig::Principal as _;
    if root.did().to_string() != expected_root {
        return Err(JsValue::from_str(
            "the evaluated passkey does not match rootDid",
        ));
    }
    let result = ceremony_result(
        crate::ceremony::create_account(
            root,
            email,
            code,
            credential_id.clone(),
            device_did,
            device_name,
            delegation_hex,
            remote,
        )
        .await
        .map_err(js_error)?,
    )?;
    Reflect::set(&result, &"credentialId".into(), &credential_id.into())?;
    Ok(result)
}

async fn link_device(input: JsValue) -> Result<JsValue, JsValue> {
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let device_name = string_property(&input, "deviceName")?;
    let evaluated = crate::passkey::evaluate_passkey().await.map_err(js_error)?;
    let credential_id = hex::encode(evaluated.id);
    let prf = evaluated
        .prf_output
        .ok_or_else(|| JsValue::from_str("the authenticator returned no PRF output"))?;
    let root = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    let result = ceremony_result(
        crate::ceremony::link_device(root, device_did, device_name)
            .await
            .map_err(js_error)?,
    )?;
    Reflect::set(&result, &"credentialId".into(), &credential_id.into())?;
    Ok(result)
}

async fn establish_account_repository(input: JsValue) -> Result<JsValue, JsValue> {
    let remote = string_property(&input, "remote")?;
    let prf = crate::passkey::prf_output().await.map_err(js_error)?;
    let root = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    let ceremony = crate::ceremony::establish_account_repository(root, remote)
        .await
        .map_err(js_error)?;
    let result = Object::new();
    Reflect::set(&result, &"rootDid".into(), &ceremony.root_did.into())?;
    Reflect::set(
        &result,
        &"descriptorHex".into(),
        &ceremony.descriptor_hex.into(),
    )?;
    Reflect::set(
        &result,
        &"invocationHex".into(),
        &ceremony.invocation_hex.into(),
    )?;
    Ok(result.into())
}

async fn complete_link(input: JsValue) -> Result<JsValue, JsValue> {
    let token_hash = string_property(&input, "tokenHash")?;
    let device_did = string_property(&input, "deviceDid")?
        .parse()
        .map_err(|error| JsValue::from_str(&format!("invalid deviceDid: {error}")))?;
    let device_name = string_property(&input, "deviceName")?;
    let prf = crate::passkey::prf_output().await.map_err(js_error)?;
    let root = crate::derive::derive_root_signer(&prf)
        .await
        .map_err(js_error)?;
    let ceremony = crate::ceremony::complete_link(root, token_hash, device_did, device_name)
        .await
        .map_err(js_error)?;
    ceremony_result(ceremony)
}

/// Install `window.tonkIdentity` on the page. Idempotent; a no-op
/// outside a window context.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let identity = Object::new();

    let create_passkey = Closure::<dyn FnMut() -> Promise>::new(|| future_to_promise(create()));
    let _ = Reflect::set(
        &identity,
        &"createPasskey".into(),
        create_passkey.as_ref().unchecked_ref(),
    );
    create_passkey.forget();

    let create_root = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(create_root(input))
    });
    let _ = Reflect::set(
        &identity,
        &"createRoot".into(),
        create_root.as_ref().unchecked_ref(),
    );
    create_root.forget();

    let evaluate_root = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(evaluate_root(input))
    });
    let _ = Reflect::set(
        &identity,
        &"evaluateRoot".into(),
        evaluate_root.as_ref().unchecked_ref(),
    );
    evaluate_root.forget();

    let derive = Closure::<dyn FnMut() -> Promise>::new(|| future_to_promise(derive_root_did()));
    let _ = Reflect::set(
        &identity,
        &"deriveRootDid".into(),
        derive.as_ref().unchecked_ref(),
    );
    derive.forget();

    let create_account = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(create_account(input))
    });
    let _ = Reflect::set(
        &identity,
        &"createAccount".into(),
        create_account.as_ref().unchecked_ref(),
    );
    create_account.forget();

    let link_device = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(link_device(input))
    });
    let _ = Reflect::set(
        &identity,
        &"linkDevice".into(),
        link_device.as_ref().unchecked_ref(),
    );
    link_device.forget();

    let establish = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(establish_account_repository(input))
    });
    let _ = Reflect::set(
        &identity,
        &"establishAccountRepository".into(),
        establish.as_ref().unchecked_ref(),
    );
    establish.forget();

    let complete_link = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(complete_link(input))
    });
    let _ = Reflect::set(
        &identity,
        &"completeLink".into(),
        complete_link.as_ref().unchecked_ref(),
    );
    complete_link.forget();

    let sign_revocation = Closure::<dyn FnMut(JsValue) -> Promise>::new(|input: JsValue| {
        future_to_promise(sign_revocation(input))
    });
    let _ = Reflect::set(
        &identity,
        &"signRevocation".into(),
        sign_revocation.as_ref().unchecked_ref(),
    );
    sign_revocation.forget();

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
            "createRoot",
            "evaluateRoot",
            "deriveRootDid",
            "createAccount",
            "linkDevice",
            "establishAccountRepository",
            "completeLink",
        ] {
            let function = Reflect::get(&identity, &name.into()).unwrap();
            assert!(function.is_function(), "{name} must be a function");
        }
    }
}
