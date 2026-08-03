//! `POST /chains/{put,list,get,spots}`: content-addressed delegation-chain
//! backup and semantic spot inventory over UCAN-authorized invocations.

use worker::*;

use tonk_account::backup::{ACCOUNT_SPOTS_CAPABILITY_HEADER, ACCOUNT_SPOTS_CAPABILITY_V1};

use crate::auth::{authorize, string_argument};
use crate::core::backup::{get_chain, list_account_spots, list_chains, put_chain_and_index_spot};
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

    let key = put_chain_and_index_spot(&chains, &caller.account, &bytes)
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
        Ok(response) => {
            let headers = response.headers().clone();
            let _ = headers.set(ACCOUNT_SPOTS_CAPABILITY_HEADER, ACCOUNT_SPOTS_CAPABILITY_V1);
            response.with_headers(headers)
        }
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

/// `POST /chains/spots` → list one semantic row per account spot.
pub async fn handle_spots(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_spots_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(err) => err.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_spots_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let store = build_store(ctx)?;
    let chains = build_chains(ctx)?;
    let body = read_body(req).await?;
    let caller = authorize(&store, &body, &["account", "chain", "spots"])
        .await
        .map_err(ceremony_error)?;
    let spots = list_account_spots(&chains, &caller.account)
        .await
        .map_err(ceremony_error)?;
    Response::from_json(&spots).map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
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
