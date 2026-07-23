//! Native CLI browser-handoff endpoints.

use serde::Deserialize;
use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::links::{complete_link, consume_link, create_link, resolve_link};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    token_hash: String,
    device_did: String,
    device_name: String,
}

#[derive(Deserialize)]
struct SecretRequest {
    secret: String,
}

/// CORS preflight for every `/links/*` endpoint.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// Create a pending CLI browser handoff.
pub async fn handle_create(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match async {
        let body: CreateRequest = req.json().await.map_err(json_error)?;
        let store = build_store(&ctx)?;
        create_link(
            &store,
            &body.token_hash,
            &body.device_did,
            &body.device_name,
            now(),
        )
        .await
        .map_err(ceremony_error)?;
        Response::from_json(&serde_json::json!({})).map_err(response_error)
    }
    .await
    {
        Ok(response) => response.with_status(201),
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

/// Resolve a pending handoff using its raw bearer secret.
pub async fn handle_resolve(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match async {
        let body: SecretRequest = req.json().await.map_err(json_error)?;
        let store = build_store(&ctx)?;
        let link = resolve_link(&store, &body.secret, now())
            .await
            .map_err(ceremony_error)?;
        Response::from_json(&link).map_err(response_error)
    }
    .await
    {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

/// Complete a handoff through a root-signed invocation.
pub async fn handle_complete(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match async {
        let body = read_body(&mut req).await?;
        let caller = authorize_root(&body, &["account", "link", "complete"])
            .await
            .map_err(ceremony_error)?;
        let store = build_store(&ctx)?;
        complete_link(
            &store,
            &caller.root_did,
            &required_string(&caller.arguments, "tokenHash").map_err(ceremony_error)?,
            &required_string(&caller.arguments, "deviceDid").map_err(ceremony_error)?,
            &required_string(&caller.arguments, "deviceName").map_err(ceremony_error)?,
            &required_string(&caller.arguments, "delegation").map_err(ceremony_error)?,
            now(),
        )
        .await
        .map_err(ceremony_error)?;
        Response::from_json(&serde_json::json!({})).map_err(response_error)
    }
    .await
    {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

/// Consume a completed delegation once, or return `202` while pending.
pub async fn handle_consume(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match async {
        let body: SecretRequest = req.json().await.map_err(json_error)?;
        let store = build_store(&ctx)?;
        match consume_link(&store, &body.secret, now())
            .await
            .map_err(ceremony_error)?
        {
            Some(delegation_hex) => {
                Response::from_json(&serde_json::json!({ "delegationHex": delegation_hex }))
                    .map_err(response_error)
            }
            None => Response::from_json(&serde_json::json!({ "pending": true }))
                .map(|response| response.with_status(202))
                .map_err(response_error),
        }
    }
    .await
    {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

fn now() -> u64 {
    Date::now().as_millis() / 1000
}

fn json_error(error: worker::Error) -> ServiceError {
    ServiceError::new(
        ErrorCode::InvalidArgument,
        format!("failed to parse request body: {error}"),
    )
}

fn response_error(error: worker::Error) -> ServiceError {
    ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
}
