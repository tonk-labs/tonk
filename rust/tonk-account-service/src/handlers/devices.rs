//! `POST /devices/{list,register,revoke}` over UCAN-authorized invocations.

use serde::Serialize;
use worker::*;

use crate::auth::{
    authorize, authorize_root, optional_revocation, required_string, string_argument,
};
use crate::core::devices::{DeviceView, list_devices, register_device, revoke_device};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{
    build_revocations, build_store, ceremony_error, read_body, with_cors_headers,
};
use crate::store::Store;

/// A device row as serialized to API callers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceJson {
    did: String,
    name: String,
    status: String,
    delegation_cid: String,
    delegation_hex: String,
    created_at: u64,
}

impl From<DeviceView> for DeviceJson {
    fn from(view: DeviceView) -> Self {
        DeviceJson {
            did: view.did,
            name: view.name,
            status: view.status,
            delegation_cid: view.delegation_cid,
            delegation_hex: view.delegation_hex,
            created_at: view.created_at,
        }
    }
}

/// `OPTIONS /devices/*` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /devices/list` → list the devices registered under an account.
pub async fn handle_list(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_list_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_list_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "device", "list"])
        .await
        .map_err(ceremony_error)?;

    let views = list_devices(&store, &caller.account)
        .await
        .map_err(ceremony_error)?;
    let devices: Vec<DeviceJson> = views.into_iter().map(DeviceJson::from).collect();

    Response::from_json(&devices).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

/// `POST /devices/register` → register a new device under an account.
pub async fn handle_register(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_register_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

/// `POST /devices/link` → register this device from a root-key ceremony.
pub async fn handle_link(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_link_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_link_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body = read_body(req).await?;
    let caller = authorize_root(&body, &["account", "device", "link"])
        .await
        .map_err(ceremony_error)?;
    let store = build_store(ctx)?;
    let account = store
        .account_by_root(&caller.root_did)
        .await
        .map_err(|err| ceremony_error(err.into()))?
        .ok_or_else(|| {
            ceremony_error(crate::core::CeremonyError::Unauthorized(
                "unknown account".to_string(),
            ))
        })?;
    let device_did = required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?;
    let device_name = required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?;
    let delegation_hex =
        required_string(&caller.arguments, "delegation").map_err(ceremony_error)?;
    let now = Date::now().as_millis() / 1000;

    register_device(
        &store,
        &account,
        &device_did,
        &device_name,
        &delegation_hex,
        now,
    )
    .await
    .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

async fn handle_register_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "device", "register"])
        .await
        .map_err(ceremony_error)?;

    let device_did = string_argument(&caller, "did").map_err(ceremony_error)?;
    let device_name = string_argument(&caller, "name").map_err(ceremony_error)?;
    let delegation_hex = string_argument(&caller, "delegation").map_err(ceremony_error)?;
    let now = Date::now().as_millis() / 1000;

    register_device(
        &store,
        &caller.account,
        &device_did,
        &device_name,
        &delegation_hex,
        now,
    )
    .await
    .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

/// `POST /devices/revoke` → revoke a device under an account.
pub async fn handle_revoke(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_revoke_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_revoke_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let revocations = build_revocations(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "device", "revoke"])
        .await
        .map_err(ceremony_error)?;

    let device_did = string_argument(&caller, "did").map_err(ceremony_error)?;
    let revocation = optional_revocation(&caller)
        .map_err(ceremony_error)?
        .ok_or_else(|| {
            ServiceError::new(
                ErrorCode::InvalidArgument,
                "a signed revocation artifact is required",
            )
        })?;
    let outcome = revoke_device(
        &store,
        &revocations,
        &caller.account,
        &caller.device.device_did,
        &device_did,
        &revocation,
    )
    .await
    .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({
        "attestation": outcome.attestation.as_str(),
        "projection": outcome.projection.as_str(),
        "targetCid": outcome.target_cid,
        "artifactCid": outcome.artifact_cid,
        "stored": outcome.stored,
    }))
    .map_err(|err| ServiceError::new(ErrorCode::InternalError, format!("response error: {err}")))
}
