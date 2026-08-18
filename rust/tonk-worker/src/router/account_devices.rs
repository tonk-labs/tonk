//! Proxy the account service's device registry for the linked profile.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::{DelegationChain, promise::Promised};
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;
use tonk_worker_api::{
    AccountDevice, AccountSummary, PasskeyMetadata, RevokeDeviceAcknowledgement,
    RevokeDeviceRequest,
};

use super::AppState;
use super::account_backup::account_service_url;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// A device row as the account service serializes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDevice {
    attachment_id: String,
    did: String,
    name: String,
    status: String,
    created_at: u64,
    delegation_cid: String,
    delegation_hex: Option<String>,
}

/// Resolve the stored link and service URL, or explain what's missing.
pub(super) async fn linked_service(
    state: &TonkState,
) -> Result<(dialog_ucan_core::DelegationChain, String), TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let service = account_service_url(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("no account service is attached to this profile".to_string())
    })?;
    Ok((link, service))
}

async fn fetch_devices(
    state: &TonkState,
    link: &dialog_ucan_core::DelegationChain,
    service: &str,
) -> Result<Vec<AccountDevice>, TonkWorkerError> {
    let device = state.profile.signer().signer().clone();
    let body = tonk_identity::request::build_device_invocation(
        device,
        link,
        vec!["account".into(), "device".into(), "list".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-list invocation: {e}")))?;
    let endpoint = url::Url::parse(&format!("{}/devices/list", service.trim_end_matches('/')))
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invalid account provider URL: {error}"))
        })?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let rows: Vec<ServiceDevice> = serde_json::from_slice(&response.body)
        .map_err(|e| TonkWorkerError::Internal(format!("parse device list: {e}")))?;
    let this_did = state.profile.did().to_string();
    Ok(rows
        .into_iter()
        .map(|row| AccountDevice {
            this_device: row.did == this_did,
            attachment_id: row.attachment_id,
            did: row.did,
            name: row.name,
            status: row.status,
            created_at: row.created_at,
            delegation_cid: row.delegation_cid,
            delegation_hex: row.delegation_hex,
        })
        .collect())
}

/// List the devices registered under this profile's account.
#[wasm_compat]
pub async fn list(
    State(state): State<AppState>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    let state = state.read().await;
    let (link, service) = linked_service(&state).await?;
    Ok(Json(fetch_devices(&state, &link, &service).await?))
}

/// `POST /api/account/devices/register` request: a device the approving
/// page just authorized, to be recorded in the account service's
/// registry under this profile's account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterDeviceRequest {
    /// The registered device's DID.
    pub did: String,
    /// Display name for the device registry.
    pub name: String,
    /// Hex-encoded `root → device` delegation the device will present.
    pub delegation_hex: String,
}

/// Register a freshly authorized device in the account service's
/// registry. The service only accepts registration from a device that is
/// already an active member, which this browser is — a device authorized
/// over a callback cannot register itself.
#[wasm_compat]
pub async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterDeviceRequest>,
) -> Result<Json<serde_json::Value>, TonkWorkerError> {
    let state = state.read().await;
    let (link, service) = linked_service(&state).await?;
    let device = state.profile.signer().signer().clone();
    let arguments = [
        ("did".to_owned(), Promised::String(request.did)),
        ("name".to_owned(), Promised::String(request.name)),
        (
            "delegation".to_owned(),
            Promised::String(request.delegation_hex),
        ),
    ]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "register".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-register invocation: {e}")))?;
    let endpoint = url::Url::parse(&format!(
        "{}/devices/register",
        service.trim_end_matches('/')
    ))
    .map_err(|error| TonkWorkerError::Internal(format!("invalid account provider URL: {error}")))?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let answer: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|e| TonkWorkerError::Internal(format!("parse registration answer: {e}")))?;
    Ok(Json(answer))
}

/// The account service's `POST /account/summary` response.
///
/// Deliberately its own type rather than [`AccountSummary`]: the provider hop
/// and the local hop no longer carry the same shape, and decoding the provider
/// straight into the local DTO is what would silently re-couple them.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderSummary {
    email: String,
    passkey: Option<PasskeyMetadata>,
}

/// Prefer the portable account-space fact; fall back to what the provider
/// recorded at account creation.
///
/// The provider row is not a legacy-only path. Every account still writes it at
/// creation, and it answers three live cases: an account created before the
/// space fact existed, a device that never held the passkey and so cannot seed
/// it, and a fresh account read in the window between account creation and the
/// first sweep that seeds the space.
fn merge_summary(
    email: Option<String>,
    space: Option<PasskeyMetadata>,
    provider: Option<PasskeyMetadata>,
) -> AccountSummary {
    AccountSummary {
        email,
        passkey: space.or(provider),
    }
}

/// Verified account facts for the linked profile, preferring what the account
/// repository carries over what the provider recorded.
///
/// Shared by the HTTP route and the roster hooks that capture the
/// account email best-effort at link time.
pub(crate) async fn account_summary(state: &TonkState) -> Result<AccountSummary, TonkWorkerError> {
    let (link, service) = linked_service(state).await?;
    let space = super::account_state::passkey_facts(state).await;
    let body = tonk_identity::request::build_device_invocation(
        state.profile.signer().signer().clone(),
        &link,
        vec!["account".into(), "summary".into()],
        BTreeMap::new(),
    )
    .await
    .map_err(|error| {
        TonkWorkerError::Internal(format!("build account-summary invocation: {error}"))
    })?;
    let endpoint = url::Url::parse(&format!(
        "{}/account/summary",
        service.trim_end_matches('/')
    ))
    .map_err(|error| TonkWorkerError::Internal(format!("invalid account provider URL: {error}")))?;
    match super::http::post_cbor(&endpoint, &body).await {
        Ok(response) => {
            let provider: ProviderSummary =
                serde_json::from_slice(&response.body).map_err(|error| {
                    TonkWorkerError::Internal(format!("parse account summary: {error}"))
                })?;
            Ok(merge_summary(Some(provider.email), space, provider.passkey))
        }
        // The account repository already answered the passkey question, so an
        // unreachable provider costs the email and nothing else. With no space
        // fact there is nothing to serve, and the caller keeps the real error.
        Err(error) if space.is_some() => {
            log!("account summary falling back to account-space facts: {error}");
            Ok(merge_summary(None, space, None))
        }
        Err(error) => Err(error.into()),
    }
}

/// Return verified account facts authorized by this profile's active grant.
#[wasm_compat]
pub async fn summary(
    State(state): State<AppState>,
) -> Result<Json<AccountSummary>, TonkWorkerError> {
    let state = state.read().await;
    Ok(Json(account_summary(&state).await?))
}

async fn self_revocation(
    state: &TonkState,
    link: &DelegationChain,
) -> Result<String, TonkWorkerError> {
    let target = link.proof_cids()[0];
    Ok(hex::encode(
        tonk_identity::revocation::mint_self_revocation(
            state.profile.signer().signer().clone(),
            link,
            &target,
        )
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("build self-revocation: {error}")))?,
    ))
}

/// Revoke a device, using device-signed self-revocation for the caller or a
/// passkey/root-signed artifact for another device.
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<RevokeDeviceAcknowledgement>, TonkWorkerError> {
    let state = state.read().await;
    let own = request.did == state.profile.did().to_string();
    let target_did = request.did.clone();
    let (link, service) = linked_service(&state).await?;
    let revocation = if own {
        self_revocation(&state, &link).await?
    } else {
        if request.revocation.is_empty() {
            return Err(TonkWorkerError::Conflict(
                "revoking another device needs a passkey-signed revocation".to_string(),
            ));
        }
        request.revocation
    };
    let device = state.profile.signer().signer().clone();
    let arguments = [
        (
            "attachmentId".to_owned(),
            Promised::String(request.attachment_id),
        ),
        ("did".to_owned(), Promised::String(request.did)),
        ("revocation".to_owned(), Promised::String(revocation)),
    ]
    .into_iter()
    .collect();
    let body = tonk_identity::request::build_device_invocation(
        device,
        &link,
        vec!["account".into(), "device".into(), "revoke".into()],
        arguments,
    )
    .await
    .map_err(|e| TonkWorkerError::Internal(format!("build device-revoke invocation: {e}")))?;
    let endpoint = url::Url::parse(&format!("{}/devices/revoke", service.trim_end_matches('/')))
        .map_err(|error| {
            TonkWorkerError::Internal(format!("invalid account provider URL: {error}"))
        })?;
    let response = super::http::post_cbor(&endpoint, &body).await?;
    let acknowledgement: RevokeDeviceAcknowledgement = serde_json::from_slice(&response.body)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("parse device-revoke acknowledgement: {error}"))
        })?;
    if acknowledgement.target_did != target_did {
        return Err(TonkWorkerError::Internal(
            "account provider acknowledged a different device".to_string(),
        ));
    }
    if acknowledgement.target_cid.is_empty() || !acknowledgement.published {
        return Err(TonkWorkerError::Internal(
            "account provider did not confirm canonical revocation publication".to_string(),
        ));
    }
    Ok(Json(acknowledgement))
}

#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use std::sync::Arc;

    use axum::Json;
    use axum::extract::State;
    use tokio::sync::RwLock;
    use tonk_worker_api::RevokeDeviceRequest;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;
    use crate::TonkWorkerError;
    use crate::router::tests::test_state_without_account;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_refuses_to_list_devices_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        assert!(matches!(
            list(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_a_summary_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        assert!(matches!(
            summary(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
    }

    #[dialog_common::test]
    fn it_prefers_the_account_space_passkey_fact_over_the_provider_row() {
        let space = PasskeyMetadata {
            created_at: 1_754_380_800,
            created_on: "Chrome on macOS".to_string(),
        };
        let provider = PasskeyMetadata {
            created_at: 1_600_000_000,
            created_on: "Safari on iOS".to_string(),
        };

        let merged = merge_summary(
            Some("person@example.com".to_string()),
            Some(space.clone()),
            Some(provider.clone()),
        );
        assert_eq!(merged.passkey, Some(space.clone()));

        let fallback = merge_summary(
            Some("person@example.com".to_string()),
            None,
            Some(provider.clone()),
        );
        assert_eq!(
            fallback.passkey,
            Some(provider),
            "an account created before the space fact existed still has the provider row"
        );

        let neither = merge_summary(Some("person@example.com".to_string()), None, None);
        assert_eq!(neither.passkey, None);

        // What an unreachable provider leaves: the portable fact, no address.
        let offline = merge_summary(None, Some(space), None);
        assert_eq!(offline.email, None);
        assert_eq!(offline.passkey.unwrap().created_on, "Chrome on macOS");
    }

    #[dialog_common::test]
    async fn it_self_revokes_without_a_passkey_artifact() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let tonk = state.read().await;
            crate::router::account::tests_matching_request(&tonk).await
        };
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request)
                .await
                .unwrap();
        }
        let tonk = state.read().await;
        let link = crate::router::account::account_link(&tonk).await.unwrap();
        let artifact = hex::decode(self_revocation(&tonk, &link).await.unwrap()).unwrap();
        let verified = tonk_identity::revocation::verify(&artifact).await.unwrap();
        assert_eq!(verified.target_cid, link.proof_cids()[0].to_string());
    }

    #[dialog_common::test]
    async fn it_refuses_to_revoke_without_a_signed_revocation() {
        let state = Arc::new(RwLock::new(test_state_without_account().await));
        let request = {
            let tonk = state.read().await;
            crate::router::account::tests_matching_request(&tonk).await
        };
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request)
                .await
                .unwrap();
        }
        assert!(
            matches!(
                revoke(
                    State(state),
                    Json(RevokeDeviceRequest {
                        attachment_id: "test-attachment".to_string(),
                        did: "did:key:zOtherDevice".to_string(),
                        revocation: String::new(),
                    })
                )
                .await,
                Err(TonkWorkerError::Conflict(_))
            ),
            "cutting off another device takes a passkey-signed revocation"
        );
    }
}
