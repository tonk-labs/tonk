//! `POST /accounts/recover`: flip an account onto a freshly created
//! passkey root under the authority of a surviving device, with proof of
//! control of the new root.

use serde::Deserialize;
use worker::*;

use crate::auth::{authorize, authorize_root, required_string};
use crate::core::recovery::recover_account;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, with_cors_headers};

#[derive(Deserialize)]
struct RecoverBody {
    recovery: String,
    confirmation: String,
}

/// `OPTIONS /accounts/recover` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /accounts/recover` → recover an account onto a new root under
/// surviving-device authority.
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body: RecoverBody = req
        .json()
        .await
        .map_err(|err| ServiceError::new(ErrorCode::InvalidArgument, format!("bad body: {err}")))?;
    let recovery_bytes = hex::decode(&body.recovery).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("bad recovery hex: {err}"),
        )
    })?;
    let confirmation_bytes = hex::decode(&body.confirmation).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("bad confirmation hex: {err}"),
        )
    })?;

    // `authorize` needs the store to resolve the device-signed
    // container's subject/issuer onto an account and device, so it must
    // be built before authorization here (unlike the root-signed-only
    // rotation handler).
    let store = build_store(ctx)?;

    let caller = authorize(&store, &recovery_bytes, &["account", "recover"])
        .await
        .map_err(ceremony_error)?;
    let confirm = authorize_root(&confirmation_bytes, &["account", "recover", "confirm"])
        .await
        .map_err(ceremony_error)?;

    // Each container must name the other's principal.
    let claimed_new_root =
        required_string(&caller.arguments, "newRootDid").map_err(ceremony_error)?;
    let claimed_old_root =
        required_string(&confirm.arguments, "oldRootDid").map_err(ceremony_error)?;
    if claimed_new_root != confirm.root_did || claimed_old_root != caller.account.root_did {
        return Err(ServiceError::new(
            ErrorCode::Forbidden,
            "recovery and confirmation containers do not name each other",
        ));
    }

    let new_credential_id =
        required_string(&caller.arguments, "newCredentialId").map_err(ceremony_error)?;
    let device_delegation_hex =
        required_string(&caller.arguments, "deviceDelegation").map_err(ceremony_error)?;

    recover_account(
        &store,
        &caller,
        &claimed_new_root,
        &new_credential_id,
        &device_delegation_hex,
    )
    .await
    .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}
