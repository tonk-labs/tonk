//! Typed boundary to the top-document passkey ceremony API.

use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_wasm_bindgen::Serializer;
use thiserror::Error;
use tonk_account::handoff::CompleteLinkCeremony;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Input for creating an account and its first custody passkey in one
/// ceremony: the secret is generated, sealed under the passkey's KEK,
/// and published as the custody cell before the creation request signs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateAccountInput {
    pub email: String,
    pub device_did: String,
    pub device_name: String,
    pub remote: String,
    /// The access service's `/ucan/` endpoint the custody cell
    /// publishes through.
    pub endpoint: String,
    /// Browser/OS label recorded with the created passkey.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_on: Option<String>,
    /// Access-service DID the ceremony mints account-signed deposits
    /// for, when the deployment names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_did: Option<String>,
}

/// Account-creation output: persistence and submission material, plus
/// the custody DID and consent for provisioning the custody space.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateAccountOutput {
    pub root_did: String,
    pub credential_id: String,
    pub delegation_hex: String,
    pub invocation_hex: String,
    #[serde(default)]
    pub passkey: Option<tonk_worker_api::PasskeyMetadata>,
    #[serde(default)]
    pub deposits_hex: Vec<String>,
    pub custody_did: String,
    pub consent_hex: String,
}

/// Input for enrolling a custody passkey for a locally held account.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrollCustodyInput {
    pub account_did: String,
    /// What the passkey manager should call the credential — the
    /// account address, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The access service's `/ucan/` endpoint the cell publishes through.
    pub endpoint: String,
}

/// A custody enrollment's outcome.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrollCustodyOutput {
    pub custody_did: String,
    pub credential_id: String,
    pub consent_hex: String,
}

/// Input for unlocking an account with a custody passkey on this browser.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnlockWithPasskeyInput {
    pub device_did: String,
    pub device_name: String,
    /// The access service's `/ucan/` endpoint the cell resolves through.
    pub endpoint: String,
    /// Access-service DID the ceremony mints account-signed deposits
    /// for, when the deployment names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_did: Option<String>,
}

/// Input for signing a device-grant revocation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignRevocationInput {
    pub delegation_cid: String,
    pub path_hex: String,
    /// The access service's `/ucan/` endpoint the custody cell
    /// resolves through.
    pub endpoint: String,
}

/// Account ceremony output sent to the account service.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CeremonyOutput {
    pub root_did: String,
    pub credential_id: String,
    pub delegation_hex: String,
    pub invocation_hex: String,
    /// Hex-encoded account-signed access-service deposits, when the
    /// input named the service.
    #[serde(default)]
    pub deposits_hex: Vec<String>,
}

/// Root-signed revocation returned by the ceremony API.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevocationOutput {
    pub revocation_hex: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeletionProofInput {
    pub kind: String,
    pub proof_hex: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrepareAccountDeletionInput {
    pub expected_root: String,
    pub confirmed_email: String,
    /// The access service's `/ucan/` endpoint the custody unlock
    /// resolves through.
    pub endpoint: String,
    pub proofs: Vec<DeletionProofInput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedAccountDeletion {
    pub space_invocations_hex: Vec<String>,
    pub customer_invocation_hex: String,
    pub account_invocation_hex: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrepareSpaceDeletionInput {
    pub expected_root: String,
    /// The access service's `/ucan/` endpoint the custody unlock
    /// resolves through.
    pub endpoint: String,
    pub proof: DeletionProofInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedSpaceDeletion {
    pub invocation_hex: String,
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

pub(crate) async fn create_account(
    input: CreateAccountInput,
) -> Result<CreateAccountOutput, IdentityBridgeError> {
    call("createAccount", input).await
}

pub(crate) async fn enroll_custody_passkey(
    input: EnrollCustodyInput,
) -> Result<EnrollCustodyOutput, IdentityBridgeError> {
    call("enrollCustodyPasskey", input).await
}

pub(crate) async fn unlock_with_passkey(
    input: UnlockWithPasskeyInput,
) -> Result<CeremonyOutput, IdentityBridgeError> {
    call("unlockWithPasskey", input).await
}

/// Input for completing a CLI handoff: the resolved link plus the
/// `/ucan/` endpoint the custody unlock resolves through.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteLinkInput {
    pub token_hash: String,
    pub device_did: String,
    pub device_name: String,
    pub endpoint: String,
}

pub(crate) async fn complete_link(
    input: CompleteLinkInput,
) -> Result<CompleteLinkCeremony, IdentityBridgeError> {
    call("completeLink", input).await
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

pub(crate) async fn sign_revocation(
    input: SignRevocationInput,
) -> Result<RevocationOutput, IdentityBridgeError> {
    call("signRevocation", input).await
}

pub(crate) async fn prepare_account_deletion(
    input: PrepareAccountDeletionInput,
) -> Result<PreparedAccountDeletion, IdentityBridgeError> {
    call("prepareAccountDeletion", input).await
}

pub(crate) async fn prepare_space_deletion(
    input: PrepareSpaceDeletionInput,
) -> Result<PreparedSpaceDeletion, IdentityBridgeError> {
    call("prepareSpaceDeletion", input).await
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

    /// Completing a CLI handoff only submits the root-signed invocation. The
    /// account service, not this browser response, supplies the durable passkey
    /// credential id when the CLI consumes the completed handoff.
    #[dialog_common::test]
    async fn it_accepts_the_actual_complete_link_output() {
        install_method(
            "completeLink",
            r#"
            return Promise.resolve({
                rootDid: "did:key:root", deviceDid: input.deviceDid,
                delegationHex: "delegation", invocationHex: "invocation"
            });
            "#,
        );

        let output = complete_link(CompleteLinkInput {
            token_hash: "token".into(),
            device_did: "did:key:device".into(),
            device_name: "CLI".into(),
            endpoint: "https://tonk.spot/ucan/".into(),
        })
        .await
        .expect("the real completeLink response is a valid bridge output");

        assert_eq!(output.invocation_hex, "invocation");
    }

    #[dialog_common::test]
    async fn it_classifies_non_promises_rejections_and_malformed_outputs() {
        let window = web_sys::window().unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &JsValue::UNDEFINED).unwrap();
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::Unavailable
        );

        Reflect::set(&window, &"tonkIdentity".into(), &js_sys::Object::new()).unwrap();
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::MissingMethod("unlockWithPasskey")
        );

        let identity = js_sys::Object::new();
        Reflect::set(&identity, &"unlockWithPasskey".into(), &42.into()).unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &identity).unwrap();
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::NotCallable("unlockWithPasskey")
        );

        install_method("unlockWithPasskey", "return {};");
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::NotPromise
        );

        install_method(
            "unlockWithPasskey",
            "return Promise.reject(new DOMException('phone authenticator returned no PRF', 'NotSupportedError')); ",
        );
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::Rejected(
                "NotSupportedError: phone authenticator returned no PRF".into()
            )
        );

        install_method(
            "unlockWithPasskey",
            "return Promise.reject('provider unavailable'); ",
        );
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::Rejected("provider unavailable".into())
        );

        install_method("unlockWithPasskey", "return Promise.resolve({});");
        assert_eq!(
            unlock_with_passkey(UnlockWithPasskeyInput {
                device_did: "device".into(),
                device_name: "Browser".into(),
                endpoint: "https://tonk.spot/ucan/".into(),
                service_did: None,
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::MalformedOutput
        );
    }
}
