//! `POST /accounts/rotate`: flip an account onto a new root DID under
//! old-root authority, with new-root proof of control.

use serde::Deserialize;
use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::rotation::{RotateAccount, rotate_account};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, with_cors_headers};
use crate::store::Store;

#[derive(Deserialize)]
struct RotateBody {
    rotation: String,
    confirmation: String,
}

/// `OPTIONS /accounts/rotate` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /accounts/rotate` → rotate an account onto a new root.
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
    let body: RotateBody = req
        .json()
        .await
        .map_err(|err| ServiceError::new(ErrorCode::InvalidArgument, format!("bad body: {err}")))?;
    let rotation_bytes = hex::decode(&body.rotation).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("bad rotation hex: {err}"),
        )
    })?;
    let confirmation_bytes = hex::decode(&body.confirmation).map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("bad confirmation hex: {err}"),
        )
    })?;

    let old = authorize_root(&rotation_bytes, &["account", "rotate"])
        .await
        .map_err(ceremony_error)?;
    let new = authorize_root(&confirmation_bytes, &["account", "rotate", "confirm"])
        .await
        .map_err(ceremony_error)?;

    // Each container must name the other's principal.
    let claimed_new = required_string(&old.arguments, "newRootDid").map_err(ceremony_error)?;
    let claimed_old = required_string(&new.arguments, "oldRootDid").map_err(ceremony_error)?;
    if claimed_new != new.root_did || claimed_old != old.root_did {
        return Err(ServiceError::new(
            ErrorCode::Forbidden,
            "rotation and confirmation containers do not name each other",
        ));
    }

    let store = build_store(ctx)?;
    let account = store
        .account_by_root(&old.root_did)
        .await
        .map_err(|err| ceremony_error(err.into()))?
        .ok_or_else(|| ServiceError::new(ErrorCode::Unauthorized, "unknown account"))?;

    let request = RotateAccount {
        new_root_did: new.root_did,
        new_credential_id: required_string(&old.arguments, "newCredentialId")
            .map_err(ceremony_error)?,
        succession_hex: required_string(&old.arguments, "succession").map_err(ceremony_error)?,
        device_did: required_string(&old.arguments, "deviceDid").map_err(ceremony_error)?,
        device_delegation_hex: required_string(&old.arguments, "deviceDelegation")
            .map_err(ceremony_error)?,
    };
    rotate_account(&store, &account, &request)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}
