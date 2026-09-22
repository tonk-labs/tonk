//! Browser half of local-space adoption.
//!
//! These narrow routes sign consent with the already-linked browser device
//! and run the ordinary targeted-invite join path. They never expose an
//! account key or account-wide grant to the waiting CLI.

use axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use dialog_ucan_core::time::Timestamp;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_invite::local_space_link;

use super::AppState;
use crate::TonkWorkerError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApproveRequest {
    request: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApproveResponse {
    approval: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteRequest {
    request: String,
    approval: String,
    invite: String,
    provisioned: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionRequest {
    request: String,
    approval: String,
    consent: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionResponse {
    provisioned: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteResponse {
    completion: String,
}

fn invalid(error: impl std::fmt::Display) -> TonkWorkerError {
    TonkWorkerError::Forbidden(error.to_string())
}

async fn trusted_service() -> Result<local_space_link::TrustedService, TonkWorkerError> {
    let origin = super::customer::service_origin()?;
    let endpoint = origin.join(".well-known/tonk").map_err(|error| {
        TonkWorkerError::Internal(format!("deployment discovery URL is invalid: {error}"))
    })?;
    let response = super::http::get(&endpoint).await.map_err(|error| {
        TonkWorkerError::Internal(format!("deployment discovery failed: {error}"))
    })?;
    let config: tonk_worker_api::DeploymentConfig = serde_json::from_slice(&response.body)
        .map_err(|error| {
            TonkWorkerError::Internal(format!("deployment discovery was malformed: {error}"))
        })?;
    let service_did = config
        .service_did
        .ok_or_else(|| TonkWorkerError::Internal("deployment has no service identity".into()))?
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("service DID is invalid: {error:?}")))?;
    let remote = super::customer::ucan_endpoint(&origin)?;
    local_space_link::TrustedService::new(service_did, remote).map_err(invalid)
}

fn decode(value: &str, label: &str) -> Result<Vec<u8>, TonkWorkerError> {
    local_space_link::decode_transport(value)
        .map_err(|_| TonkWorkerError::Router(format!("{label} is not valid base58")))
}

fn encode(bytes: &[u8]) -> Result<String, TonkWorkerError> {
    local_space_link::encode_transport(bytes).map_err(invalid)
}

#[wasm_compat]
pub(crate) async fn approve(
    State(state): State<AppState>,
    Json(body): Json<ApproveRequest>,
) -> Result<Json<ApproveResponse>, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        &body.request,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;
    let tonk = state.read().await;
    let root = super::identity::local_root(&tonk).await?;
    let device = tonk.profile.signer().signer().clone();
    let approval = local_space_link::LocalSpaceLinkApproval::issue_from_device(
        &request,
        root.delegation,
        &device,
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    Ok(Json(ApproveResponse {
        approval: encode(&approval.to_bytes().map_err(invalid)?)?,
    }))
}

#[wasm_compat]
pub(crate) async fn complete(
    State(state): State<AppState>,
    Json(body): Json<CompleteRequest>,
) -> Result<Json<CompleteResponse>, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        &body.request,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;

    let tonk = state.write().await;
    let root = super::identity::local_root(&tonk).await?;
    let approval = local_space_link::LocalSpaceLinkApproval::from_bytes(&decode(
        &body.approval,
        "local-space link approval",
    )?)
    .map_err(invalid)?
    .validate(&request, Some(&root.root_did), Timestamp::now())
    .await
    .map_err(invalid)?;
    let provisioned = local_space_link::LocalSpaceLinkCompletion::from_bytes(&decode(
        &body.provisioned,
        "local-space provisioning receipt",
    )?)
    .map_err(invalid)?
    .validate(&approval, Timestamp::now())
    .await
    .map_err(invalid)?;
    if provisioned.publication != "provisioned" {
        return Err(TonkWorkerError::Forbidden(
            "local-space provisioning receipt has the wrong stage".into(),
        ));
    }
    let outcome =
        super::join::join_for_local_space_link(&tonk, &body.invite, &request.space).await?;
    let device = tonk.profile.signer().signer().clone();
    let completion = local_space_link::LocalSpaceLinkCompletion::issue_from_device(
        &approval,
        root.delegation,
        &device,
        format!("{}:{}", outcome.key, outcome.subject),
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    Ok(Json(CompleteResponse {
        completion: encode(&completion.to_bytes().map_err(invalid)?)?,
    }))
}

#[wasm_compat]
pub(crate) async fn provision(
    State(state): State<AppState>,
    Json(body): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, TonkWorkerError> {
    let service = trusted_service().await?;
    let request = local_space_link::LocalSpaceLinkRequest::from_bytes(&decode(
        &body.request,
        "local-space link request",
    )?)
    .map_err(invalid)?
    .validate(&service, Timestamp::now())
    .await
    .map_err(invalid)?;

    let tonk = state.write().await;
    let root = super::identity::local_root(&tonk).await?;
    let approval = local_space_link::LocalSpaceLinkApproval::from_bytes(&decode(
        &body.approval,
        "local-space link approval",
    )?)
    .map_err(invalid)?
    .validate(&request, Some(&root.root_did), Timestamp::now())
    .await
    .map_err(invalid)?;
    let consent_bytes = decode(&body.consent, "local-space link consent")?;
    let consent = tonk_account::prefix::validate_prefix(&consent_bytes, &approval.account)
        .await
        .map_err(invalid)?;
    if consent.subject != request.space || consent.chain.proofs().count() != 1 {
        return Err(TonkWorkerError::Forbidden(
            "local-space link consent is not direct authority for this space".into(),
        ));
    }
    super::customer::provision_consumer(&tonk, &request.space, &consent.chain, None).await?;
    super::join::save_local_space_root_authority(&tonk, &request.space, consent.chain).await?;
    let device = tonk.profile.signer().signer().clone();
    let provisioned = local_space_link::LocalSpaceLinkCompletion::issue_from_device(
        &approval,
        root.delegation,
        &device,
        "provisioned".into(),
        Timestamp::now(),
    )
    .await
    .map_err(invalid)?;
    Ok(Json(ProvisionResponse {
        provisioned: encode(&provisioned.to_bytes().map_err(invalid)?)?,
    }))
}
