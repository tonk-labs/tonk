//! `POST /devices/{list,register,revoke}` over UCAN-authorized invocations.

use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::devices::link_device;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};
use crate::store::Store;

/// `OPTIONS /devices/*` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
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
    let descriptor = account.repository_descriptor.as_ref().ok_or_else(|| {
        ceremony_error(crate::core::CeremonyError::Conflict(
            tonk_account::UNESTABLISHED_ACCOUNT_CONFLICT.to_string(),
        ))
    })?;
    let descriptor_hex = hex::encode(descriptor);
    let now = Date::now().as_millis() / 1000;

    let attachment_id = link_device(
        &store,
        &account,
        &device_did,
        &device_name,
        &delegation_hex,
        now,
    )
    .await
    .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({
        "attachmentId": attachment_id,
        "descriptorHex": descriptor_hex,
    }))
    .map_err(|err| ServiceError::new(ErrorCode::InternalError, format!("response error: {err}")))
}
