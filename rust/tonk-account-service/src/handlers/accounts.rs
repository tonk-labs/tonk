//! `POST /accounts`: create a new account and register its first device.

use worker::*;

use serde::Deserialize;

use crate::auth::{
    authorize, authorize_root, authorize_setup_device, optional_passkey_metadata, required_string,
};
use crate::core::accounts::{CreateAccount, account_setup_status, create_account};
use crate::core::deletion::delete_account;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};
use crate::store::Store;

/// `OPTIONS /accounts` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /account/delete` → permanently remove account-service state.
pub async fn handle_delete(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_delete_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_delete_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body = read_body(req).await?;
    // Device-authorized: the account's chain delegated to this device
    // IS the deletion authority (possession is policy). The typed
    // email confirmation and the passkey user-verification gesture are
    // client-side safeguards; the argument check here is what binds
    // the reviewed email to the signed request.
    let store = build_store(ctx)?;
    let caller = authorize(&store, &body, &["account", "delete"])
        .await
        .map_err(ceremony_error)?;
    let confirmed_email =
        required_string(&caller.arguments, "confirmedEmail").map_err(ceremony_error)?;
    let receipt = delete_account(&store, &caller.account.root_did, &confirmed_email)
        .await
        .map_err(ceremony_error)?;
    Response::from_json(&receipt).map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}

/// `POST /account/summary` → verified email and passkey creation facts.
pub async fn handle_summary(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_summary_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_summary_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "summary"])
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({
        "email": caller.account.email,
        "passkey": caller.account.passkey_created_at.zip(caller.account.passkey_created_on)
            .map(|(created_at, created_on)| serde_json::json!({
                "createdAt": created_at,
                "createdOn": created_on,
            })),
    }))
    .map_err(|err| ServiceError::new(ErrorCode::InternalError, format!("response error: {err}")))
}

/// `POST /accounts` → create a new account.
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

/// `POST /accounts/setup-status` → proof-bound account-creation state.
pub async fn handle_setup_status(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_setup_status_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_setup_status_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body = read_body(req).await?;
    let caller = authorize_setup_device(&body, &["account", "setup", "status"])
        .await
        .map_err(ceremony_error)?;
    let expected =
        required_string(&caller.arguments, "createFingerprint").map_err(ceremony_error)?;
    let store = build_store(ctx)?;
    let status = account_setup_status(
        &store,
        &caller.root_did,
        &caller.device_did,
        &caller.delegation_cid,
        &expected,
    )
    .await
    .map_err(ceremony_error)?;
    Response::from_json(&status).map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body = read_body(req).await?;
    let caller = authorize_root(&body, &["account", "create"])
        .await
        .map_err(ceremony_error)?;
    let now = Date::now().as_millis() / 1000;
    let passkey = optional_passkey_metadata(&caller.arguments, now).map_err(ceremony_error)?;
    let request = CreateAccount {
        email: required_string(&caller.arguments, "email").map_err(ceremony_error)?,
        credential_id: required_string(&caller.arguments, "credentialId")
            .map_err(ceremony_error)?,
        device_did: required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
        device_name: required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
        delegation_hex: required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
        repository_descriptor_hex: required_string(&caller.arguments, "repositoryDescriptor")
            .map_err(ceremony_error)?,
        root_did: caller.root_did,
        passkey,
    };
    let store = build_store(ctx)?;
    let outcome = create_account(&store, &request, now)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({
        "accountId": outcome.account_id,
        "descriptorHex": hex::encode(outcome.descriptor),
        "createFingerprint": outcome.create_fingerprint,
        "reused": outcome.reused,
    }))
    .map(|response| response.with_status(if outcome.reused { 200 } else { 201 }))
    .map_err(|err| ServiceError::new(ErrorCode::InternalError, format!("response error: {err}")))
}
