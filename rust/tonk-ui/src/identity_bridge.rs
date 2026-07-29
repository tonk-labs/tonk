//! Typed boundary to the top-document passkey ceremony API.

use js_sys::{Function, Promise, Reflect};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_wasm_bindgen::Serializer;
use thiserror::Error;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// Input for creating or evaluating a root credential.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRootInput {
    pub device_did: String,
}

/// Evaluating an existing root takes the same device binding as creation.
pub(crate) type EvaluateRootInput = CreateRootInput;

/// Input for creating an account and its first device registration.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateAccountInput {
    pub email: String,
    pub code: String,
    pub device_did: String,
    pub device_name: String,
    pub root_did: String,
    pub credential_id: String,
    pub delegation_hex: String,
}

/// Input for linking the current browser as another account device.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LinkDeviceInput {
    pub device_did: String,
    pub device_name: String,
}

/// Input for completing a command-line device handoff.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteLinkInput {
    pub token_hash: String,
    pub device_did: String,
    pub device_name: String,
}

/// Input for signing a device-grant revocation.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SignRevocationInput {
    pub delegation_cid: String,
    pub path_hex: String,
}

/// Root ceremony output persisted by the service worker.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootOutput {
    pub root_did: String,
    pub device_did: String,
    pub credential_id: String,
    pub delegation_hex: String,
}

/// Account ceremony output sent to the account service.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CeremonyOutput {
    pub root_did: String,
    pub credential_id: String,
    pub delegation_hex: String,
    pub invocation_hex: String,
}

/// Root-signed revocation returned by the ceremony API.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevocationOutput {
    pub revocation_hex: String,
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
    #[error("identity ceremony was cancelled or failed")]
    Rejected,
    #[error("identity ceremony returned an invalid response")]
    MalformedOutput,
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
        .map_err(|_| IdentityBridgeError::Rejected)?
        .dyn_into()
        .map_err(|_| IdentityBridgeError::NotPromise)?;
    let output = JsFuture::from(promise)
        .await
        .map_err(|_| IdentityBridgeError::Rejected)?;
    serde_wasm_bindgen::from_value(output).map_err(|_| IdentityBridgeError::MalformedOutput)
}

pub(crate) async fn create_root(input: CreateRootInput) -> Result<RootOutput, IdentityBridgeError> {
    call("createRoot", input).await
}

pub(crate) async fn evaluate_root(
    input: EvaluateRootInput,
) -> Result<RootOutput, IdentityBridgeError> {
    call("evaluateRoot", input).await
}

pub(crate) async fn create_account(
    input: CreateAccountInput,
) -> Result<CeremonyOutput, IdentityBridgeError> {
    call("createAccount", input).await
}

pub(crate) async fn link_device(
    input: LinkDeviceInput,
) -> Result<CeremonyOutput, IdentityBridgeError> {
    call("linkDevice", input).await
}

pub(crate) async fn complete_link(
    input: CompleteLinkInput,
) -> Result<CeremonyOutput, IdentityBridgeError> {
    call("completeLink", input).await
}

pub(crate) async fn sign_revocation(
    input: SignRevocationInput,
) -> Result<RevocationOutput, IdentityBridgeError> {
    call("signRevocation", input).await
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

    #[dialog_common::test]
    async fn it_passes_root_input_as_a_plain_camel_case_object() {
        install_method(
            "createRoot",
            r#"
            if (input instanceof Map) return Promise.reject(new Error("map"));
            if (!(Object.getPrototypeOf(input) === Object.prototype || Object.getPrototypeOf(input) === null))
                return Promise.reject(new Error("prototype"));
            if (input.deviceDid !== "did:key:device") return Promise.reject(new Error("property"));
            return Promise.resolve({
                rootDid: "did:key:root", deviceDid: input.deviceDid,
                credentialId: "credential", delegationHex: "delegation"
            });
            "#,
        );
        let output = create_root(CreateRootInput {
            device_did: "did:key:device".into(),
        })
        .await
        .unwrap();
        assert_eq!(output.device_did, "did:key:device");
    }

    #[dialog_common::test]
    async fn it_passes_every_ceremony_a_plain_camel_case_object() {
        let plain_object_guard = r#"
            if (input instanceof Map) return Promise.reject(new Error("map"));
            if (!(Object.getPrototypeOf(input) === Object.prototype || Object.getPrototypeOf(input) === null))
                return Promise.reject(new Error("prototype"));
        "#;

        install_method(
            "evaluateRoot",
            &format!(
                r#"{plain_object_guard}
                if (input.deviceDid !== "did:key:device") return Promise.reject(new Error("property"));
                return Promise.resolve({{
                    rootDid: "did:key:root", deviceDid: input.deviceDid,
                    credentialId: "credential", delegationHex: "delegation"
                }});"#
            ),
        );
        assert_eq!(
            evaluate_root(EvaluateRootInput {
                device_did: "did:key:device".into(),
            })
            .await
            .unwrap()
            .root_did,
            "did:key:root"
        );

        install_method(
            "createAccount",
            &format!(
                r#"{plain_object_guard}
                if (input.email !== "person@example.test" || input.code !== "123456"
                    || input.deviceDid !== "did:key:device" || input.deviceName !== "Browser"
                    || input.rootDid !== "did:key:root" || input.credentialId !== "credential"
                    || input.delegationHex !== "delegation")
                    return Promise.reject(new Error("property"));
                return Promise.resolve({{
                    rootDid: input.rootDid, credentialId: input.credentialId,
                    delegationHex: input.delegationHex, invocationHex: "invocation"
                }});"#
            ),
        );
        assert_eq!(
            create_account(CreateAccountInput {
                email: "person@example.test".into(),
                code: "123456".into(),
                device_did: "did:key:device".into(),
                device_name: "Browser".into(),
                root_did: "did:key:root".into(),
                credential_id: "credential".into(),
                delegation_hex: "delegation".into(),
            })
            .await
            .unwrap()
            .invocation_hex,
            "invocation"
        );

        install_method(
            "linkDevice",
            &format!(
                r#"{plain_object_guard}
                if (input.deviceDid !== "did:key:device" || input.deviceName !== "Browser")
                    return Promise.reject(new Error("property"));
                return Promise.resolve({{
                    rootDid: "did:key:root", credentialId: "credential",
                    delegationHex: "delegation", invocationHex: "invocation"
                }});"#
            ),
        );
        link_device(LinkDeviceInput {
            device_did: "did:key:device".into(),
            device_name: "Browser".into(),
        })
        .await
        .unwrap();

        install_method(
            "completeLink",
            &format!(
                r#"{plain_object_guard}
                if (input.tokenHash !== "token" || input.deviceDid !== "did:key:device"
                    || input.deviceName !== "CLI")
                    return Promise.reject(new Error("property"));
                return Promise.resolve({{
                    rootDid: "did:key:root", credentialId: "credential",
                    delegationHex: "delegation", invocationHex: "invocation"
                }});"#
            ),
        );
        complete_link(CompleteLinkInput {
            token_hash: "token".into(),
            device_did: "did:key:device".into(),
            device_name: "CLI".into(),
        })
        .await
        .unwrap();

        install_method(
            "signRevocation",
            &format!(
                r#"{plain_object_guard}
                if (input.delegationCid !== "bafygrant" || input.pathHex !== "path")
                    return Promise.reject(new Error("property"));
                return Promise.resolve({{ revocationHex: "revocation" }});"#
            ),
        );
        assert_eq!(
            sign_revocation(SignRevocationInput {
                delegation_cid: "bafygrant".into(),
                path_hex: "path".into(),
            })
            .await
            .unwrap()
            .revocation_hex,
            "revocation"
        );
    }

    #[dialog_common::test]
    async fn it_classifies_non_promises_rejections_and_malformed_outputs() {
        let window = web_sys::window().unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &JsValue::UNDEFINED).unwrap();
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::Unavailable
        );

        Reflect::set(&window, &"tonkIdentity".into(), &js_sys::Object::new()).unwrap();
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::MissingMethod("createRoot")
        );

        let identity = js_sys::Object::new();
        Reflect::set(&identity, &"createRoot".into(), &42.into()).unwrap();
        Reflect::set(&window, &"tonkIdentity".into(), &identity).unwrap();
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::NotCallable("createRoot")
        );

        install_method("createRoot", "return {};");
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::NotPromise
        );

        install_method("createRoot", "return Promise.reject(new Error('no')); ");
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::Rejected
        );

        install_method("createRoot", "return Promise.resolve({});");
        assert_eq!(
            create_root(CreateRootInput {
                device_did: "device".into()
            })
            .await
            .unwrap_err(),
            IdentityBridgeError::MalformedOutput
        );
    }
}
