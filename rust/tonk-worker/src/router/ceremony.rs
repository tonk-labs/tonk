//! Passkey-gated commands, and how they report.
//!
//! The hub's settings page runs in a sealed guest with no `window` of
//! its own, so it cannot run WebAuthn. It asserts a command instead; the
//! handler asks the page for the passkey through the custody relay, the
//! page hands back derivation handles, and the worker does the work with
//! the account it opens from them. Progress goes on the profile overlay
//! as a [`CeremonyStatus`] row the page subscribes to.

use tonk_common::log;
use tonk_schema::{CeremonyStatus, ceremony, ceremony_state};

use crate::worker::TonkState;

/// Write where `ceremony` got to, replacing the last report.
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

/// Ask the page for the passkey, reporting the ask and its failure. On
/// a host with no page the ask fails and the failure is reported — the
/// ceremony refuses visibly rather than silently not existing.
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
    let credential_id = {
        let tonk = env.state().read().await;
        report(&tonk, ceremony, ceremony_state::PENDING_CEREMONY, "").await;
        // A CLI authorization may use any passkey belonging to the signed-in
        // account. Pinning it to this profile's original credential prevents
        // another enrolled provider (for example 1Password) from offering its
        // passkey. The authorization handler verifies the recovered root DID
        // against this profile before minting the device grant.
        if matches!(&intent, tonk_worker_api::CustodyIntent::AuthorizeDevice(_)) {
            None
        } else {
            super::identity::local_root(&tonk)
                .await
                .ok()
                .map(|root| root.credential_id)
                .filter(|id| !id.is_empty())
        }
    };
    if let Err(error) = super::navigate::request_webauthn_with(
        client,
        tonk_worker_api::WebAuthnKind::Custody,
        Some(intent),
        credential_id,
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

/// Run `tonk:add-passkey`: seal the account under a second passkey.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<tonk_schema::command::AddPasskey> for crate::router::CommandEnv {
    async fn execute(&self, _command: tonk_schema::command::AddPasskey) {
        let addition = {
            let tonk = self.state().read().await;
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
            self,
            ceremony::ADD_PASSKEY,
            tonk_worker_api::CustodyIntent::AddPasskey(addition),
        )
        .await;
    }
}

/// Decode optional account constraints without accepting malformed constraints.
pub(crate) struct AuthorizeDeviceRequest(tonk_worker_api::DeviceAuthorization);
impl crate::reactor::Decode for AuthorizeDeviceRequest {
    fn trigger_attributes() -> Vec<String> {
        <tonk_schema::command::AuthorizeDevice as crate::reactor::Decode>::trigger_attributes()
    }
    fn decode(
        _entity: dialog_artifacts::Entity,
        facts: &crate::reactor::EntityFacts,
    ) -> Option<Self> {
        decode_authorization(facts).map(Self)
    }
}
impl dialog_capability::Command for AuthorizeDeviceRequest {
    type Input = Self;
    type Output = ();
}

fn decode_authorization(
    facts: &crate::reactor::EntityFacts,
) -> Option<tonk_worker_api::DeviceAuthorization> {
    use crate::reactor::Decode as _;
    let entity = facts.first()?.of.clone();
    // An absent optional field is unbound in the transient decoder. Decode
    // the original login shape only when no constraint was supplied; an
    // invalid supplied constraint must never fall back to unrestricted login.
    let command = if facts
        .iter()
        .any(|artifact| artifact.the.to_string() == "xyz.tonk.authorize-device/expected-account")
    {
        tonk_schema::command::AuthorizeDevice::decode(entity, facts)?
    } else {
        let legacy = tonk_schema::command::legacy::AuthorizeDevice::decode(entity, facts)?;
        tonk_schema::command::AuthorizeDevice {
            this: legacy.this,
            audience: legacy.audience,
            callback: legacy.callback,
            name: legacy.name,
            expected_account: None,
        }
    };
    let callback = bs58::decode(&command.callback.0).into_vec().ok()?;
    Some(tonk_worker_api::DeviceAuthorization {
        audience: command.audience.0.to_string(),
        callback: String::from_utf8(callback).ok()?,
        name: command.name.0,
        expected_account: command
            .expected_account
            .map(|account| account.0.to_string()),
    })
}

/// Run `tonk:authorize-device`: delegate the account to a waiting
/// process. Unparseable audience/callback/name skip with a log; an
/// undeliverable callback is refused before anyone touches a passkey.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl dialog_capability::Provider<AuthorizeDeviceRequest> for crate::router::CommandEnv {
    async fn execute(&self, command: AuthorizeDeviceRequest) {
        let authorization = command.0;
        // Refuse a callback the grant could never be delivered to
        // before asking anyone to touch a passkey.
        if let Err(error) = tonk_worker_api::callback::delivery_url(&authorization.callback, &[]) {
            let tonk = self.state().read().await;
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
            self,
            ceremony::AUTHORIZE_DEVICE,
            tonk_worker_api::CustodyIntent::AuthorizeDevice(authorization),
        )
        .await;
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
    if let Some(expected) = &authorization.expected_account
        && root.did().as_str() != expected
    {
        return Err(format!(
            "this handoff requires account {expected}; the unlocked passkey belongs to {}",
            root.did()
        ));
    }
    let audience: dialog_varsig::Did = authorization
        .audience
        .parse()
        .map_err(|error| format!("the device DID is invalid: {error:?}"))?;
    let origin = super::customer::service_origin().map_err(|error| error.to_string())?;
    // The sync endpoint the grant's signed `meta` names: the provider
    // the account registered with, so every attach path hands out the
    // one recorded address. Only an unattached profile falls back to
    // the deployment's own endpoint.
    let remote = {
        let tonk = state.read().await;
        super::account::provider(&tonk)
            .await
            .unwrap_or_else(|| format!("{}ucan/", origin))
    };
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
        // The grant's signed meta is the authoritative address; this
        // field is the carrier for CLIs from before the meta rode the
        // delegation, whose schema requires it.
        "remote": remote,
        "credentialId": authorized.root_did,
        "attachmentId": attachment_id,
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

#[cfg(test)]
mod authorization_tests {
    use super::decode_authorization;
    use dialog_artifacts::{Artifact, Value};

    #[dialog_common::test]
    fn agent_handoff_authorization_preserves_optional_constraint() {
        let did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let mut facts = vec![
            ("audience", Value::Entity(did.parse().unwrap())),
            (
                "callback",
                Value::String(bs58::encode("http://127.0.0.1:1234").into_string()),
            ),
            ("name", Value::String("terminal".into())),
        ]
        .into_iter()
        .map(|(field, value)| Artifact {
            the: format!("xyz.tonk.authorize-device/{field}")
                .parse()
                .unwrap(),
            of: "urn:test:approval".parse().unwrap(),
            is: value,
            cause: None,
        })
        .collect::<Vec<_>>();
        assert_eq!(decode_authorization(&facts).unwrap().expected_account, None);
        facts.push(Artifact {
            the: "xyz.tonk.authorize-device/expected-account"
                .parse()
                .unwrap(),
            of: "urn:test:approval".parse().unwrap(),
            is: Value::Entity(did.parse().unwrap()),
            cause: None,
        });
        assert_eq!(
            decode_authorization(&facts)
                .unwrap()
                .expected_account
                .as_deref(),
            Some(did)
        );
        facts.last_mut().unwrap().is = Value::String("invalid account".into());
        assert!(
            decode_authorization(&facts).is_none(),
            "a malformed constraint must not fall back to ordinary login"
        );
    }
}
