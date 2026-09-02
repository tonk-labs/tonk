//! Typed boundary to the top-document passkey ceremony API.

use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_wasm_bindgen::Serializer;
use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Where the custody assertion that recovers the account root resolves
/// its sealed secret: the page's own `/ucan/` endpoint.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignAccountPurgeInput {
    pub endpoint: String,
}

/// The root-signed `/void/customer/purge`, ready for the worker to
/// present, and the root that signed it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignedAccountPurge {
    pub invocation_hex: String,
    pub root_did: String,
}

/// Stable failures produced at the JavaScript identity boundary.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum IdentityBridgeError {
    #[error("identity ceremonies are unavailable")]
    Unavailable,
    #[error("identity ceremony {0} is unavailable")]
    MissingMethod(&'static str),
    #[error("identity ceremony {0} is not callable")]
    NotCallable(&'static str),
    #[error("identity ceremony input could not be prepared")]
    InvalidInput,
    #[error("identity ceremony did not return a promise")]
    NotPromise,
    #[error("identity ceremony failed: {0}")]
    Rejected(String),
    #[error("identity ceremony returned an invalid response")]
    MalformedOutput,
}

const MAX_REJECTION_REASON_CHARS: usize = 512;

/// Preserve the browser/provider reason without attempting to serialize an
/// arbitrary rejected object. wasm-bindgen ceremonies reject with strings;
/// browser APIs usually reject with an `Error` or `DOMException` carrying a
/// stable `name` and human-readable `message`.
fn rejection_reason(value: JsValue) -> String {
    fn bounded(value: String) -> String {
        value.chars().take(MAX_REJECTION_REASON_CHARS).collect()
    }

    if let Some(reason) = value.as_string() {
        return bounded(reason);
    }

    let property = |name: &str| {
        Reflect::get(&value, &name.into())
            .ok()
            .and_then(|value| value.as_string())
            .filter(|value| !value.trim().is_empty())
    };
    match (property("name"), property("message")) {
        (Some(name), Some(message)) => bounded(format!("{name}: {message}")),
        (Some(name), None) => bounded(name),
        (None, Some(message)) => bounded(message),
        (None, None) => "unknown browser error".to_string(),
    }
}

async fn call<I: Serialize, O: DeserializeOwned>(
    method: &'static str,
    input: I,
) -> Result<O, IdentityBridgeError> {
    let window = web_sys::window().ok_or(IdentityBridgeError::Unavailable)?;
    let identity = Reflect::get(&window, &"tonkIdentity".into())
        .map_err(|_| IdentityBridgeError::Unavailable)?;
    if identity.is_null() || identity.is_undefined() {
        return Err(IdentityBridgeError::Unavailable);
    }
    let value = Reflect::get(&identity, &method.into())
        .map_err(|_| IdentityBridgeError::MissingMethod(method))?;
    if value.is_null() || value.is_undefined() {
        return Err(IdentityBridgeError::MissingMethod(method));
    }
    let function: Function = value
        .dyn_into()
        .map_err(|_| IdentityBridgeError::NotCallable(method))?;
    let input = input
        .serialize(&Serializer::json_compatible())
        .map_err(|_| IdentityBridgeError::InvalidInput)?;
    let promise: Promise = function
        .call1(&identity, &input)
        .map_err(|error| IdentityBridgeError::Rejected(rejection_reason(error)))?
        .dyn_into()
        .map_err(|_| IdentityBridgeError::NotPromise)?;
    let output = JsFuture::from(promise)
        .await
        .map_err(|error| IdentityBridgeError::Rejected(rejection_reason(error)))?;
    serde_wasm_bindgen::from_value(output).map_err(|_| IdentityBridgeError::MalformedOutput)
}

/// Input for [`publish_encryption_key`]: the `/ucan/` endpoint the custody
/// cell resolves through.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishEncryptionKeyInput {
    pub endpoint: String,
    /// Hex credential id to pin the assertion to, from the root record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

/// The account's X25519 recipient, derived through one assertion.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishedEncryptionKey {
    pub encryption_key: String,
}

pub(crate) async fn publish_encryption_key(
    input: PublishEncryptionKeyInput,
) -> Result<PublishedEncryptionKey, IdentityBridgeError> {
    call("publishEncryptionKey", input).await
}

/// Input for [`authorize_device`].
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizeDeviceInput {
    /// The device the account should delegate to.
    pub device_did: String,
    /// The account repository's remote, so the descriptor names it.
    pub remote: String,
    /// The access service's `/ucan/` endpoint the custody cell
    /// resolves through.
    pub endpoint: String,
}

/// What the ceremony hands back for delivery to a waiting device.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizedDevice {
    /// The account root that issued the grant.
    pub root_did: String,
    /// Hex-encoded `account → device` delegation chain.
    pub delegation_hex: String,
    /// Exact signed account repository descriptor.
    pub descriptor_hex: String,
}

pub(crate) async fn authorize_device(
    input: AuthorizeDeviceInput,
) -> Result<AuthorizedDevice, IdentityBridgeError> {
    call("authorizeDevice", input).await
}

/// Recover the account root through its passkey and sign the purge
/// with it. The ceremony is the whole authority: the worker presents
/// what comes back and signs nothing itself.
pub(crate) async fn sign_account_purge(
    input: SignAccountPurgeInput,
) -> Result<SignedAccountPurge, IdentityBridgeError> {
    call("signAccountPurge", input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    fn install_method(name: &str, body: &str) {
        let window = web_sys::window().unwrap();
        let identity = js_sys::Object::new();
        let function = Function::new_with_args("input", body);
        Reflect::set(&identity, &name.into(), &function).unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &identity).unwrap();
    }

    fn purge_input() -> SignAccountPurgeInput {
        SignAccountPurgeInput {
            endpoint: "https://tonk.example/ucan/".into(),
        }
    }

    /// The ceremony hands back the signed purge and the root that
    /// signed it, camel-cased the way every ceremony output is.
    #[dialog_common::test]
    async fn it_decodes_the_signed_purge() {
        install_method(
            "signAccountPurge",
            "return Promise.resolve({ invocationHex: 'abcd', rootDid: 'did:key:zRoot' });",
        );
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap(),
            SignedAccountPurge {
                invocation_hex: "abcd".into(),
                root_did: "did:key:zRoot".into(),
            }
        );
    }

    #[dialog_common::test]
    async fn it_classifies_non_promises_rejections_and_malformed_outputs() {
        let window = web_sys::window().unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &JsValue::UNDEFINED).unwrap();
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::Unavailable
        );

        Reflect::set(&window, &"tonkIdentity".into(), &js_sys::Object::new()).unwrap();
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::MissingMethod("signAccountPurge")
        );

        let identity = js_sys::Object::new();
        Reflect::set(&identity, &"signAccountPurge".into(), &42.into()).unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &identity).unwrap();
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::NotCallable("signAccountPurge")
        );

        install_method("signAccountPurge", "return {};");
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::NotPromise
        );

        install_method(
            "signAccountPurge",
            "return Promise.reject(new DOMException('phone authenticator returned no PRF', 'NotSupportedError')); ",
        );
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::Rejected(
                "NotSupportedError: phone authenticator returned no PRF".into()
            )
        );

        install_method(
            "signAccountPurge",
            "return Promise.reject('provider unavailable'); ",
        );
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::Rejected("provider unavailable".into())
        );

        install_method("signAccountPurge", "return Promise.resolve({});");
        assert_eq!(
            sign_account_purge(purge_input()).await.unwrap_err(),
            IdentityBridgeError::MalformedOutput
        );
    }
}
