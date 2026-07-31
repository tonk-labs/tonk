//! Proxy the account service's device registry for the linked profile.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::{DelegationChain, promise::Promised};
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{AccountDevice, RevokeDeviceAcknowledgement, RevokeDeviceRequest};

use super::AppState;
use super::account_backup::account_service_url;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// A device row as the account service serializes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDevice {
    did: String,
    name: String,
    status: String,
    created_at: u64,
    delegation_cid: String,
    delegation_hex: Option<String>,
}

/// Resolve the stored link and service URL, or explain what's missing.
async fn linked_service(
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
