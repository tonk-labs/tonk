//! Unauthenticated publication of self-certifying revocation artifacts.

use worker::*;

use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_revocations, with_cors_headers};
use crate::revocations::{PublishError, publish};
use tonk_identity::revocation::VerifyError;

pub(crate) const MAX_ARTIFACT_BYTES: usize = 64 * 1024;

pub(crate) fn publish_error(error: PublishError) -> ServiceError {
    match error {
        PublishError::Verification(VerifyError::Malformed(message)) => {
            ServiceError::new(ErrorCode::InvalidArgument, message)
        }
        PublishError::Verification(VerifyError::Unauthorized(message)) => {
            ServiceError::new(ErrorCode::Forbidden, message)
        }
        PublishError::Store(error) => {
            console_error!("revocation publication failed: {error}");
            ServiceError::new(ErrorCode::InternalError, "internal error")
        }
    }
}

/// `OPTIONS /revocations` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /revocations` → verify and publish immutable artifact bytes.
pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(error) => error.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let content_type = req
        .headers()
        .get("Content-Type")
        .map_err(|error| {
            ServiceError::new(
                ErrorCode::InvalidArgument,
                format!("failed to read Content-Type: {error}"),
            )
        })?
        .unwrap_or_default();
    if content_type.split(';').next() != Some("application/cbor") {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "Content-Type must be application/cbor",
        ));
    }
    if let Some(length) = req
        .headers()
        .get("Content-Length")
        .map_err(|error| {
            ServiceError::new(
                ErrorCode::InvalidArgument,
                format!("failed to read Content-Length: {error}"),
            )
        })?
        .and_then(|value| value.parse::<usize>().ok())
        && length > MAX_ARTIFACT_BYTES
    {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "revocation artifact exceeds 64 KiB",
        ));
    }

    let bytes = req.bytes().await.map_err(|error| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to read request body: {error}"),
        )
    })?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "revocation artifact exceeds 64 KiB",
        ));
    }

    let store = build_revocations(ctx)?;
    let outcome = publish(&store, &bytes).await.map_err(publish_error)?;
    Response::from_json(&serde_json::json!({
        "targetCid": outcome.verified.target_cid,
        "artifactCid": outcome.verified.artifact_cid,
        "stored": outcome.stored,
    }))
    .map(|response| response.with_status(202))
    .map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}
