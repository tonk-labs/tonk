//! `POST /codes`: request an email verification code.

use serde::Deserialize;
use worker::*;

use crate::core::codes::{generate_code, request_code};
use crate::email::resend::Resend;
use crate::error::{ErrorCode, ServiceError};
use crate::handlers::{build_store, ceremony_error, with_cors_headers};

/// The `POST /codes` request body.
#[derive(Deserialize)]
struct CodeRequest {
    /// The email address to send a verification code to.
    email: String,
}

/// `OPTIONS /codes` → CORS preflight.
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Ok(with_cors_headers(Response::empty()?.with_status(204)))
}

/// `POST /codes` → request a verification code.
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
    let body: CodeRequest = req.json().await.map_err(|err| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("failed to parse request body: {err}"),
        )
    })?;

    let store = build_store(ctx)?;
    let sender = build_resend(ctx)?;

    let code = generate_code();
    let now = Date::now().as_millis() / 1000;
    request_code(&store, &sender, &body.email, &code, now)
        .await
        .map_err(ceremony_error)?;

    Response::from_json(&serde_json::json!({})).map_err(|err| {
        ServiceError::new(ErrorCode::InternalError, format!("response error: {err}"))
    })
}

/// Build a [`Resend`] sender from the worker environment's
/// `RESEND_API_KEY` secret and `EMAIL_FROM` variable.
fn build_resend(ctx: &RouteContext<()>) -> std::result::Result<Resend, ServiceError> {
    let api_key = ctx
        .secret("RESEND_API_KEY")
        .map_err(|err| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("missing RESEND_API_KEY: {err}"),
            )
        })?
        .to_string();
    let from = ctx
        .var("EMAIL_FROM")
        .map_err(|err| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("missing EMAIL_FROM: {err}"),
            )
        })?
        .to_string();
    Ok(Resend::new(api_key, from))
}
