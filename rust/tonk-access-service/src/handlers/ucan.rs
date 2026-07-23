//! UCAN authorization handler.
//!
//! Handles POST /ucan/ requests by:
//! 1. Reading the CBOR-encoded UCAN container from the request body
//! 2. Passing it to UcanAuthorizer for verification and authorization
//! 3. Returning the serialized AuthorizedRequest as CBOR

use crate::error::{ErrorCode, ServiceError};
use dialog_remote_s3::{Address, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;
use worker::*;

/// Add CORS headers to a response for WASM compatibility.
fn with_cors_headers(response: Response) -> Response {
    let headers = response.headers().clone();
    let _ = headers.set("Access-Control-Allow-Origin", "*");
    let _ = headers.set("Access-Control-Allow-Methods", "POST, OPTIONS");
    let _ = headers.set("Access-Control-Allow-Headers", "Content-Type");
    let _ = headers.set("Access-Control-Expose-Headers", "Content-Type");
    response.with_headers(headers)
}

/// OPTIONS /ucan/ → Handle CORS preflight
pub async fn handle_options(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let response = Response::empty()?.with_status(204);
    Ok(with_cors_headers(response))
}

/// POST /ucan/ → Authorize UCAN invocation and return presigned S3 request
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
    // 1. Read the request body as bytes
    let body_bytes = req.bytes().await.map_err(|e| {
        ServiceError::new(
            ErrorCode::InvalidArgument,
            format!("Failed to read request body: {}", e),
        )
    })?;

    // 2. Create the UcanAuthorizer from environment config
    let authorizer = create_authorizer(ctx)?;

    // 3. Authorize the UCAN container
    let authorized_request = authorizer
        .authorize(&body_bytes)
        .await
        .map_err(map_access_error)?;

    // 3b. Screen the presented credentials against revoked devices.
    // Runs only after cryptographic authorization succeeded, and fails
    // open: registry trouble must never take sync down.
    #[cfg(target_arch = "wasm32")]
    screen_revoked(&body_bytes, ctx).await?;

    // 4. Serialize the response as CBOR
    let cbor_bytes = serde_ipld_dagcbor::to_vec(&authorized_request).map_err(|e| {
        ServiceError::new(
            ErrorCode::InternalError,
            format!("Failed to serialize response: {}", e),
        )
    })?;

    // 5. Return CBOR response
    Response::from_bytes(cbor_bytes)
        .map(|r| {
            let headers = Headers::new();
            let _ = headers.set("Content-Type", "application/cbor");
            r.with_headers(headers)
        })
        .map_err(|e| ServiceError::new(ErrorCode::InternalError, format!("Response error: {}", e)))
}

#[cfg(target_arch = "wasm32")]
async fn screen_revoked(
    body_bytes: &[u8],
    ctx: &RouteContext<()>,
) -> std::result::Result<(), ServiceError> {
    use crate::revocation::{self, RevocationVerdict, d1::D1RevocationRegistry};

    let presented = match revocation::collect_presented(body_bytes) {
        Ok(presented) => presented,
        Err(err) => {
            // The authorizer already accepted this container; a parse
            // failure here is a shape drift to surface, not a reason to
            // block the request.
            console_error!("revocation screen skipped, container unparseable: {err}");
            return Ok(());
        }
    };
    let registry = match ctx.d1("ACCOUNTS_DB") {
        Ok(db) => D1RevocationRegistry::new(db),
        Err(err) => {
            console_error!("revocation screen skipped, no ACCOUNTS_DB binding: {err}");
            return Ok(());
        }
    };
    let now_ms = Date::now().as_millis();
    match revocation::assess(&registry, &presented, now_ms).await {
        RevocationVerdict::Allowed => Ok(()),
        RevocationVerdict::AllowedFailOpen(reason) => {
            console_error!("revocation screen failed open: {reason}");
            Ok(())
        }
        RevocationVerdict::Revoked => {
            worker::console_log!("presign rejected: revoked credential present");
            Err(ServiceError::new(
                ErrorCode::DeviceRevoked,
                "a credential in the presented chain has been revoked",
            ))
        }
    }
}

/// Create UcanAuthorizer from environment configuration.
fn create_authorizer(ctx: &RouteContext<()>) -> std::result::Result<UcanAuthorizer, ServiceError> {
    // Get R2 configuration from environment
    let account_id = ctx
        .var("R2_ACCOUNT_ID")
        .map_err(|e| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("Missing R2_ACCOUNT_ID: {}", e),
            )
        })?
        .to_string();

    let access_key_id = ctx
        .secret("R2_ACCESS_KEY_ID")
        .map_err(|e| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("Missing R2_ACCESS_KEY_ID: {}", e),
            )
        })?
        .to_string();

    let secret_access_key = ctx
        .secret("R2_SECRET_ACCESS_KEY")
        .map_err(|e| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("Missing R2_SECRET_ACCESS_KEY: {}", e),
            )
        })?
        .to_string();

    let bucket = ctx
        .var("R2_BUCKET_NAME")
        .map_err(|e| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("Missing R2_BUCKET_NAME: {}", e),
            )
        })?
        .to_string();

    // Build R2 endpoint URL
    let endpoint = format!("https://{}.r2.cloudflarestorage.com", account_id);

    // Create S3 credentials for R2 (using "auto" region as R2 requires)
    let address = Address::builder(&endpoint)
        .region("auto")
        .bucket(&bucket)
        .build()
        .map_err(|e| {
            ServiceError::new(
                ErrorCode::InternalError,
                format!("Failed to create address: {}", e),
            )
        })?;

    let credential = S3Credential::new(access_key_id, secret_access_key);

    Ok(UcanAuthorizer::new(address, Some(credential)))
}

/// Map S3Error to ServiceError with appropriate error codes.
fn map_access_error(err: dialog_remote_s3::S3Error) -> ServiceError {
    let msg = err.to_string();
    if msg.contains("expired") {
        ServiceError::new(ErrorCode::InvocationExpired, msg)
    } else if msg.contains("signature") {
        ServiceError::new(ErrorCode::SignatureInvalid, msg)
    } else if msg.contains("audience") {
        ServiceError::new(ErrorCode::AudienceMismatch, msg)
    } else if msg.contains("subject") {
        ServiceError::new(ErrorCode::SubjectNotAllowed, msg)
    } else if msg.contains("command") || msg.contains("Command") {
        ServiceError::new(ErrorCode::CommandMismatch, msg)
    } else if msg.contains("delegation") || msg.contains("chain") {
        ServiceError::new(ErrorCode::ChainInvalid, msg)
    } else {
        ServiceError::new(ErrorCode::InvalidArgument, msg)
    }
}
