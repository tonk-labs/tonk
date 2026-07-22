//! `POST /chains/{put,list,get}`: content-addressed delegation chain
//! backup, over UCAN-authorized invocation containers.

use worker::*;

use crate::auth::{authorize, string_argument};
use crate::core::backup::{get_chain, list_chains, put_chain};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_chains, build_store, ceremony_error, read_body, with_cors_headers};

/// `OPTIONS /chains/*` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /chains/put` → back up a delegation chain, returning its
/// content-addressed key.
pub async fn handle_put(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_put_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_put_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let chains = build_chains(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "chain", "put"])
        .await
        .map_err(ceremony_error)?;

    let chain_hex = string_argument(&caller, "chain").map_err(ceremony_error)?;
    let bytes = hex::decode(&chain_hex).map_err(|err| {
        ServiceError::new(ErrorCode::InvalidArgument, format!("bad chain hex: {err}"))
    })?;

    let key = put_chain(&chains, &caller.account, &bytes)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({ "key": key })).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

/// `POST /chains/list` → list the chain keys backed up under an
/// account.
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
    let chains = build_chains(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "chain", "list"])
        .await
        .map_err(ceremony_error)?;

    let keys = list_chains(&chains, &caller.account)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&keys).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

/// `POST /chains/get` → fetch the bytes backed up under a chain key.
pub async fn handle_get(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_get_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_get_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let chains = build_chains(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "chain", "get"])
        .await
        .map_err(ceremony_error)?;

    let key = string_argument(&caller, "key").map_err(ceremony_error)?;
    let bytes = get_chain(&chains, &caller.account, &key)
        .await
        .map_err(ceremony_error)?;

    Response::from_bytes(bytes)
        .map(|response| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "application/octet-stream");
            response.with_headers(headers)
        })
        .map_err(|err| {
            ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
        })
}
