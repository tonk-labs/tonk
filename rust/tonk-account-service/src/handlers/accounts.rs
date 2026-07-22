//! `POST /accounts`: create a new account and register its first device.

use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::accounts::{CreateAccount, create_account};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};

/// `OPTIONS /accounts` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /accounts` → create a new account.
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
    let body = read_body(req).await?;
    let caller = authorize_root(&body, &["account", "create"])
        .await
        .map_err(ceremony_error)?;
    let request = CreateAccount {
        email: required_string(&caller.arguments, "email").map_err(ceremony_error)?,
        code: required_string(&caller.arguments, "code").map_err(ceremony_error)?,
        credential_id: required_string(&caller.arguments, "credentialId")
            .map_err(ceremony_error)?,
        device_did: required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
        device_name: required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
        delegation_hex: required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
        root_did: caller.root_did,
    };
    let store = build_store(ctx)?;
    let now = Date::now().as_millis() / 1000;
    let account_id = create_account(&store, &request, now)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({ "accountId": account_id }))
        .map(|response| response.with_status(201))
        .map_err(|err| {
            ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
        })
}
