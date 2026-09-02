//! Passkey-gated commands, and how they report.
//!
//! The hub's settings page runs in a sealed guest with no `window` of
//! its own, so it cannot run WebAuthn. It asserts a command instead; the
//! handler asks the page for the passkey through the custody relay, the
//! page hands back derivation handles, and the worker does the work with
//! the account it opens from them. Progress goes on the profile overlay
//! as a [`CeremonyStatus`] row the page subscribes to.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_common::log;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_schema::{CeremonyStatus, ceremony, ceremony_state};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use crate::worker::TonkState;

/// Write where `ceremony` got to, replacing the last report.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn report(tonk: &TonkState, ceremony: &str, state: &str, detail: &str) {
    let Ok(this) = CeremonyStatus::ENTITY.parse::<dialog_artifacts::Entity>() else {
        return;
    };
    log!("{ceremony}: {state} {detail}");
    if let Err(error) = tonk
        .reactor
        .profile_repository()
        .branch(tonk_account::MAIN_BRANCH)
        .overlay()
        .assert(CeremonyStatus::new(this, ceremony, state, detail))
        .write()
        .perform(&tonk.operator)
        .await
    {
        log!("{ceremony}: the status was not published: {error}");
    }
}

/// Ask the page for the passkey, reporting the ask and its failure.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn ask_for_passkey(
    env: &crate::router::CommandEnv,
    ceremony: &str,
    intent: tonk_worker_api::CustodyIntent,
) {
    let Some(client) = env.client() else {
        let tonk = env.state().read().await;
        report(
            &tonk,
            ceremony,
            ceremony_state::REFUSED,
            "no page asked for this, so no passkey can be asked for",
        )
        .await;
        return;
    };
    {
        let tonk = env.state().read().await;
        report(&tonk, ceremony, ceremony_state::PENDING_CEREMONY, "").await;
    }
    if let Err(error) = super::navigate::request_webauthn_with(
        client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(intent),
    )
    .await
    {
        let tonk = env.state().read().await;
        report(
            &tonk,
            ceremony,
            ceremony_state::FAILED,
            &format!("the page could not be asked for the passkey: {error}"),
        )
        .await;
    }
}

/// `tonk:add-passkey`: seal the account under a second passkey.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct AddPasskeyHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl AddPasskeyHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::AddPasskey::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for AddPasskeyHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        use crate::reactor::Decode as _;
        facts
            .first()
            .map(|artifact| artifact.of.clone())
            .and_then(|entity| tonk_schema::command::AddPasskey::decode(entity, facts))
            .is_some()
    }

    fn run(
        &self,
        _facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let env = env.clone();
        Box::pin(async move {
            let addition = {
                let tonk = env.state().read().await;
                let Ok(root) = super::identity::local_root(&tonk).await else {
                    report(
                        &tonk,
                        ceremony::ADD_PASSKEY,
                        ceremony_state::REFUSED,
                        "no account is signed in on this profile",
                    )
                    .await;
                    return;
                };
                let Ok(origin) = super::customer::service_origin() else {
                    report(
                        &tonk,
                        ceremony::ADD_PASSKEY,
                        ceremony_state::REFUSED,
                        "the sync service is unknown",
                    )
                    .await;
                    return;
                };
                tonk_worker_api::PasskeyAddition {
                    account_did: root.root_did.to_string(),
                    endpoint: format!("{}ucan/", origin),
                }
            };
            ask_for_passkey(
                &env,
                ceremony::ADD_PASSKEY,
                tonk_worker_api::CustodyIntent::AddPasskey(addition),
            )
            .await;
        })
    }
}

/// `tonk:authorize-device`: delegate the account to a waiting process.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) struct AuthorizeDeviceHandler {
    attributes: Vec<String>,
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl AuthorizeDeviceHandler {
    pub(crate) fn new() -> Self {
        use crate::reactor::Decode as _;
        Self {
            attributes: tonk_schema::command::AuthorizeDevice::trigger_attributes(),
        }
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn decode_authorization(
    facts: &crate::reactor::EntityFacts,
) -> Option<tonk_worker_api::DeviceAuthorization> {
    use crate::reactor::Decode as _;
    let command = facts
        .first()
        .map(|artifact| artifact.of.clone())
        .and_then(|entity| tonk_schema::command::AuthorizeDevice::decode(entity, facts))?;
    let callback = bs58::decode(&command.callback.0).into_vec().ok()?;
    Some(tonk_worker_api::DeviceAuthorization {
        audience: command.audience.0.to_string(),
        callback: String::from_utf8(callback).ok()?,
        name: command.name.0,
    })
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
impl crate::reactor::CommandHandler<crate::router::CommandEnv> for AuthorizeDeviceHandler {
    fn trigger_attributes(&self) -> &[String] {
        &self.attributes
    }

    fn matches(&self, facts: &crate::reactor::EntityFacts) -> bool {
        decode_authorization(facts).is_some()
    }

    fn run(
        &self,
        facts: &crate::reactor::EntityFacts,
        env: &crate::router::CommandEnv,
    ) -> crate::reactor::RunFuture {
        let authorization = decode_authorization(facts);
        let env = env.clone();
        Box::pin(async move {
            let Some(authorization) = authorization else {
                log!("authorize-device: no/unparseable audience, callback, or name; skipping");
                return;
            };
            // Refuse a callback the grant could never be delivered to
            // before asking anyone to touch a passkey.
            if let Err(error) =
                tonk_worker_api::callback::delivery_url(&authorization.callback, &[])
            {
                let tonk = env.state().read().await;
                report(
                    &tonk,
                    ceremony::AUTHORIZE_DEVICE,
                    ceremony_state::REFUSED,
                    &error,
                )
                .await;
                return;
            }
            ask_for_passkey(
                &env,
                ceremony::AUTHORIZE_DEVICE,
                tonk_worker_api::CustodyIntent::AuthorizeDevice(authorization),
            )
            .await;
        })
    }
}

/// Mint the `account -> device` grant with the root the passkey
/// recovered, register the device in the account space, and answer
/// with the callback URL the page must go to. The page navigates: the
/// custody reply is what it is waiting on, and the status row carries
/// the same target for the guest that asked.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub(crate) async fn authorize_device(
    state: &crate::router::AppState,
    custodian: &tonk_identity::custodian::Custodian,
    authorization: tonk_worker_api::DeviceAuthorization,
) -> Result<String, String> {
    let outcome = authorize_device_inner(state, custodian, &authorization).await;
    let tonk = state.read().await;
    match &outcome {
        Ok(target) => {
            report(
                &tonk,
                ceremony::AUTHORIZE_DEVICE,
                ceremony_state::DONE,
                target,
            )
            .await
        }
        Err(error) => {
            report(
                &tonk,
                ceremony::AUTHORIZE_DEVICE,
                ceremony_state::FAILED,
                error,
            )
            .await
        }
    }
    outcome
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn authorize_device_inner(
    state: &crate::router::AppState,
    custodian: &tonk_identity::custodian::Custodian,
    authorization: &tonk_worker_api::DeviceAuthorization,
) -> Result<String, String> {
    use dialog_varsig::Principal as _;

    {
        let tonk = state.read().await;
        report(
            &tonk,
            ceremony::AUTHORIZE_DEVICE,
            ceremony_state::WORKING,
            "",
        )
        .await;
    }
    let account = super::custody::held_account(custodian).await?;
    let dialog_credentials::Signer::Ed25519(root) = account
        .signer()
        .await
        .map_err(|error| format!("the account signer did not derive: {error:#}"))?;
    let linked = {
        let tonk = state.read().await;
        super::identity::local_root(&tonk)
            .await
            .map_err(|error| format!("no account is signed in on this profile: {error}"))?
            .root_did
    };
    if root.did() != linked {
        return Err("this passkey belongs to a different account".into());
    }
    let audience: dialog_varsig::Did = authorization
        .audience
        .parse()
        .map_err(|error| format!("the device DID is invalid: {error:?}"))?;
    let origin = super::customer::service_origin().map_err(|error| error.to_string())?;
    let remote = format!("{}ucan/", origin);
    let authorized = tonk_identity::ceremony::authorize_device(root, audience, &remote)
        .await
        .map_err(|error| format!("the device grant did not sign: {error:#}"))?;
    // The service only accepts device registration from an active
    // member, which this browser is and the waiting device is not:
    // register it here, before the grant is delivered, so a device that
    // installs the grant is already listed and able to reach the service.
    let registered = super::account_devices::register(
        axum::extract::State(state.clone()),
        axum::Json(super::account_devices::RegisterDeviceRequest {
            did: authorization.audience.clone(),
            name: authorization.name.clone(),
            delegation_hex: authorized.delegation_hex.clone(),
        }),
    )
    .await
    .map_err(|error| format!("the device was not registered: {error}"))?;
    let attachment_id = registered
        .0
        .get("attachmentId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let payload = serde_json::json!({
        "delegationHex": authorized.delegation_hex,
        "remote": remote,
        "descriptorHex": authorized.descriptor_hex,
        "credentialId": authorized.root_did,
        "attachmentId": attachment_id,
        "serviceUrl": remote,
    })
    .to_string();
    let encoded = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(payload)
    };
    let redirect = format!("{origin}settings");
    tonk_worker_api::callback::delivery_url(
        &authorization.callback,
        &[("authorize", &encoded), ("redirect", &redirect)],
    )
}
