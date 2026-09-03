//! Typed boundary to the top-document passkey ceremony API.

use js_sys::{Function, Promise, Reflect};
use serde::{Serialize, de::DeserializeOwned};
use serde_wasm_bindgen::Serializer;
use thiserror::Error;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

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
#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishedEncryptionKey {
    pub encryption_key: String,
}

pub(crate) async fn publish_encryption_key(
    input: PublishEncryptionKeyInput,
) -> Result<PublishedEncryptionKey, IdentityBridgeError> {
    call("publishEncryptionKey", input).await
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

    fn key_input() -> PublishEncryptionKeyInput {
        PublishEncryptionKeyInput {
            endpoint: "https://tonk.example/ucan/".into(),
            credential_id: None,
        }
    }

    /// The ceremony hands back the account's recipient, camel-cased the
    /// way every ceremony output is.
    #[dialog_common::test]
    async fn it_decodes_the_published_key() {
        install_method(
            "publishEncryptionKey",
            "return Promise.resolve({ encryptionKey: 'did:key:z6LSkey' });",
        );
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap(),
            PublishedEncryptionKey {
                encryption_key: "did:key:z6LSkey".into(),
            }
        );
    }

    #[dialog_common::test]
    async fn it_classifies_non_promises_rejections_and_malformed_outputs() {
        let window = web_sys::window().unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &JsValue::UNDEFINED).unwrap();
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::Unavailable
        );

        Reflect::set(&window, &"tonkIdentity".into(), &js_sys::Object::new()).unwrap();
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::MissingMethod("publishEncryptionKey")
        );

        let identity = js_sys::Object::new();
        Reflect::set(&identity, &"publishEncryptionKey".into(), &42.into()).unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &identity).unwrap();
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::NotCallable("publishEncryptionKey")
        );

        install_method("publishEncryptionKey", "return {};");
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::NotPromise
        );

        install_method(
            "publishEncryptionKey",
            "return Promise.reject(new DOMException('phone authenticator returned no PRF', 'NotSupportedError')); ",
        );
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::Rejected(
                "NotSupportedError: phone authenticator returned no PRF".into()
            )
        );

        install_method(
            "publishEncryptionKey",
            "return Promise.reject('provider unavailable'); ",
        );
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::Rejected("provider unavailable".into())
        );

        install_method("publishEncryptionKey", "return Promise.resolve({});");
        assert_eq!(
            publish_encryption_key(key_input()).await.unwrap_err(),
            IdentityBridgeError::MalformedOutput
        );
    }
}
