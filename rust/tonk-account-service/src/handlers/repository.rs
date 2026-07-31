//! `POST /account/repository/establish`: establish one descriptor winner.

use worker::*;

use crate::auth::{authorize_root, required_string};
use crate::core::descriptor::establish_descriptor;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, read_body, with_cors_headers};
use crate::store::Store;

/// CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// Establish the immutable account repository descriptor if absent.
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match async {
        let body = read_body(&mut req).await?;
        let caller = authorize_root(&body, &["account", "repository", "establish"])
            .await
            .map_err(ceremony_error)?;
        let store = build_store(&ctx)?;
        let account = store
            .account_by_root(&caller.root_did)
            .await
            .map_err(|error| ceremony_error(error.into()))?
            .ok_or_else(|| {
                ceremony_error(crate::core::CeremonyError::Unauthorized(
                    "unknown account".to_string(),
                ))
            })?;
        let candidate =
            required_string(&caller.arguments, "repositoryDescriptor").map_err(ceremony_error)?;
        let established = establish_descriptor(&store, &account, &candidate)
            .await
            .map_err(ceremony_error)?;
        Response::from_json(&serde_json::json!({
            "descriptorHex": hex::encode(established.descriptor),
            "created": established.created,
        }))
        .map_err(|error| {
            ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
        })
    }
    .await
    {
        Ok(response) => response,
        Err(error) => error.to_response()?,
    };
    Ok(with_cors_headers(response))
}
