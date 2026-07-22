//! `POST /accounts`: create a new account and register its first device.

use serde::Deserialize;
use worker::*;

use crate::core::accounts::{CreateAccount, create_account};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, with_cors_headers};

/// The `POST /accounts` request body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAccountRequest {
    email: String,
    code: String,
    root_did: String,
    credential_id: String,
    device_did: String,
    device_name: String,
    delegation_hex: String,
}

impl From<CreateAccountRequest> for CreateAccount {
    fn from(req: CreateAccountRequest) -> Self {
        CreateAccount {
            email: req.email,
            code: req.code,
            root_did: req.root_did,
            credential_id: req.credential_id,
            device_did: req.device_did,
            device_name: req.device_name,
            delegation_hex: req.delegation_hex,
        }
    }
}

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
    let body: CreateAccountRequest = req.json().await.map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to parse request body: {err}"),
        )
    })?;

    let store = build_store(ctx)?;
    let now = Date::now().as_millis() / 1000;
    let account_id = create_account(&store, &body.into(), now)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({ "accountId": account_id }))
        .map(|response| response.with_status(201))
        .map_err(|err| {
            ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
        })
}
