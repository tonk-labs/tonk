//! Publication and claiming of audience-keyed enrollment chains.
//!
//! Both routes are unauthenticated, for the same reason `/revocations` is:
//! the bytes are self-authenticating and neither holding nor handing back a
//! delegation grants anything to anyone who lacks the audience's key.

use worker::*;

use dialog_varsig::Did;

use dialog_varsig::Principal;

use crate::core::enrollment::issue_anchor_chain;
use crate::enrollments::{EnrollmentStore, VerifyError, verify};
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{
    build_anchor, build_enrollments, build_store, ceremony_error, read_body, with_cors_headers,
};
use crate::revocations::PutOutcome;
use crate::store::Store;

/// A chain deep enough to need more than this is not an enrollment.
pub(crate) const MAX_CHAIN_BYTES: usize = 8 * 1024;

pub(crate) fn verify_error(error: VerifyError) -> ServiceError {
    match error {
        VerifyError::Invalid(message) => ServiceError::new(ErrorCode::InvalidArgument, message),
        VerifyError::Unauthorized(message) => ServiceError::new(ErrorCode::Forbidden, message),
    }
}

/// `OPTIONS /enrollments*` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /enrollments` → publish a chain addressed to a credential.
pub async fn handle_publish(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_publish_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(error) => error.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_publish_inner(
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

    let bytes = req.bytes().await.map_err(|error| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to read request body: {error}"),
        )
    })?;
    if bytes.len() > MAX_CHAIN_BYTES {
        return Err(ServiceError::new(
            ErrorCode::InvalidArgument,
            "enrollment chain exceeds 8 KiB",
        ));
    }

    let verified = verify(&bytes).await.map_err(verify_error)?;

    // Not a judgement about who may enroll — the chain already settles that.
    // It keeps the bucket from being free storage for accounts that are not
    // ours, which is the only thing an open write endpoint invites.
    let store = build_store(ctx)?;
    if store
        .account_by_root(verified.account_root.as_ref())
        .await
        .map_err(|error| {
            console_error!("enrollment account lookup failed: {error:?}");
            ServiceError::new(ErrorCode::InternalError, "internal error")
        })?
        .is_none()
    {
        return Err(ServiceError::new(
            ErrorCode::NotFound,
            "this service does not host the account the chain runs from",
        ));
    }

    let enrollments = build_enrollments(ctx)?;
    let stored = enrollments.put(&verified, &bytes).await.map_err(|error| {
        console_error!("enrollment publication failed: {error}");
        ServiceError::new(ErrorCode::InternalError, "internal error")
    })? == PutOutcome::Stored;

    // A chain addressed to this service's own anchor is the proof it will
    // later extend, so it is additionally filed by the account it comes
    // from. Claiming by audience would return every account's at once.
    if let Ok(anchor) = build_anchor(ctx).await
        && verified.credential == anchor.did()
    {
        enrollments
            .put_anchor(&verified.account_root, &bytes)
            .await
            .map_err(|error| {
                console_error!("anchor proof filing failed: {error}");
                ServiceError::new(ErrorCode::InternalError, "internal error")
            })?;
    }

    Response::from_json(&serde_json::json!({
        "credential": verified.credential.to_string(),
        "accountRoot": verified.account_root.to_string(),
        "key": verified.key,
        "stored": stored,
    }))
    .map(|response| response.with_status(202))
    .map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}

/// `POST /enrollments/confirm` → mint an anchor chain behind a one-time code.
pub async fn handle_confirm(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_confirm_inner(&mut req, &ctx).await {
        Ok(response) => response,
        Err(error) => error.to_response()?,
    };
    Ok(with_cors_headers(response))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmRequest {
    email: String,
    code: String,
    credential: String,
}

async fn handle_confirm_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    let body = read_body(req).await?;
    let request: ConfirmRequest = serde_json::from_slice(&body).map_err(|error| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to parse request body: {error}"),
        )
    })?;
    let credential: Did = request.credential.parse().map_err(|error| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("invalid credential DID: {error}"),
        )
    })?;

    let issued = issue_anchor_chain(
        &build_store(ctx)?,
        &build_enrollments(ctx)?,
        &crate::handlers::codes::build_resend(ctx)?,
        build_anchor(ctx).await?,
        &request.email,
        &request.code,
        &credential,
        Date::now().as_millis() / 1000,
    )
    .await
    .map_err(ceremony_error)?;

    let chain_hex = issued.chain.to_bytes().map(hex::encode).map_err(|error| {
        ServiceError::new(
            ErrorCode::InternalError,
            format!("failed to serialize the chain: {error}"),
        )
    })?;
    Response::from_json(&serde_json::json!({
        "accountRoot": issued.account_root.to_string(),
        "credential": issued.credential.to_string(),
        "chain": chain_hex,
        "stored": issued.stored,
    }))
    .map(|response| response.with_status(201))
    .map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}

/// `GET /enrollments/:credential` → every chain addressed to that key.
pub async fn handle_claim(_req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let response = match handle_claim_inner(&ctx).await {
        Ok(response) => response,
        Err(error) => error.to_response()?,
    };
    Ok(with_cors_headers(response))
}

async fn handle_claim_inner(ctx: &RouteContext<()>) -> std::result::Result<Response, ServiceError> {
    let credential = ctx
        .param("credential")
        .ok_or_else(|| ServiceError::new(ErrorCode::InvalidArgument, "missing credential DID"))?;
    let credential: Did = credential.parse().map_err(|error| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("invalid credential DID: {error}"),
        )
    })?;

    let enrollments = build_enrollments(ctx)?;
    let chains = enrollments.claim(&credential).await.map_err(|error| {
        console_error!("enrollment claim failed: {error}");
        ServiceError::new(ErrorCode::InternalError, "internal error")
    })?;

    Response::from_json(&serde_json::json!({
        "chains": chains.iter().map(hex::encode).collect::<Vec<_>>(),
    }))
    .map_err(|error| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {error}"))
    })
}
