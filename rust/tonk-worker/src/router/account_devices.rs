//! Proxy the account service's device registry for the linked profile.

use std::collections::BTreeMap;

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::promise::Promised;
use serde::Deserialize;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_worker_api::{AccountDevice, RevokeDeviceRequest};

use super::AppState;
use super::account_backup::{account_service_url, post_for_bytes};
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// A device row as the account service serializes it. `delegationCid` is
/// deliberately not modeled: the UI has no use for it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceDevice {
    did: String,
    name: String,
    status: String,
    created_at: u64,
    delegation_cid: String,
}

/// Resolve the stored link and service URL, or explain what's missing.
async fn linked_service(
    state: &TonkState,
) -> Result<(dialog_ucan_core::DelegationChain, String), TonkWorkerError> {
    let link = super::account::account_link(state).await.ok_or_else(|| {
        TonkWorkerError::NotFound("this profile is not linked to an account".to_string())
    })?;
    let service = account_service_url().ok_or_else(|| {
        TonkWorkerError::NotFound("no account service is configured for this host".to_string())
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
    let endpoint = format!("{}/devices/list", service.trim_end_matches('/'));
    let bytes = post_for_bytes(&endpoint, body).await?;
    let rows: Vec<ServiceDevice> = serde_json::from_slice(&bytes)
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

/// Revoke another of the account's devices, then return the fresh list.
///
/// Revoking the requesting device is refused: cutting a device off is an
/// action taken *about* a lost or untrusted device from a surviving one.
/// The local analogue on this device is unlink (sign out).
#[wasm_compat]
pub async fn revoke(
    State(state): State<AppState>,
    Json(request): Json<RevokeDeviceRequest>,
) -> Result<Json<Vec<AccountDevice>>, TonkWorkerError> {
    let state = state.read().await;
    if request.did == state.profile.did().to_string() {
        return Err(TonkWorkerError::Conflict(
            "cannot revoke the device you are using; sign out instead".to_string(),
        ));
    }
    // Checked before the account link and service are resolved: this is
    // a property of the request itself, and a caller who sent no
    // revocation should hear that rather than whichever lookup happened
    // to fail first.
    if request.revocation.is_empty() {
        return Err(TonkWorkerError::Conflict(
            "revoking another device needs a passkey-signed revocation".to_string(),
        ));
    }
    let (link, service) = linked_service(&state).await?;
    let device = state.profile.signer().signer().clone();
    let arguments = [
        ("did".to_owned(), Promised::String(request.did)),
        (
            "revocation".to_owned(),
            Promised::String(request.revocation),
        ),
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
    let endpoint = format!("{}/devices/revoke", service.trim_end_matches('/'));
    let _ = post_for_bytes(&endpoint, body).await?;
    Ok(Json(fetch_devices(&state, &link, &service).await?))
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
    use crate::router::tests::test_state;

    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_refuses_to_list_devices_for_an_unlinked_profile() {
        let state = Arc::new(RwLock::new(test_state().await));
        assert!(matches!(
            list(State(state)).await,
            Err(TonkWorkerError::NotFound(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_to_revoke_the_requesting_device() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request =
            crate::router::account::tests_request_for(&[7u8; 32], device_did.clone()).await;
        {
            let tonk = state.read().await;
            crate::router::account::persist_link(&tonk, &request)
                .await
                .unwrap();
        }
        assert!(matches!(
            revoke(
                State(state),
                Json(RevokeDeviceRequest {
                    did: device_did.to_string(),
                    revocation: "beef".to_string(),
                })
            )
            .await,
            Err(TonkWorkerError::Conflict(_))
        ));
    }

    #[dialog_common::test]
    async fn it_refuses_to_revoke_without_a_signed_revocation() {
        let state = Arc::new(RwLock::new(test_state().await));
        let device_did = state.read().await.profile.did();
        let request =
            crate::router::account::tests_request_for(&[7u8; 32], device_did.clone()).await;
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
