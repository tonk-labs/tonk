//! Blob GET/PUT handlers with 307 redirects.
//!
//! These handlers implement the router pattern:
//! - GET /{space_did}/index/{digest} → 307 redirect to presigned GET URL
//! - PUT /{space_did}/index/{digest} → 307 redirect to presigned PUT URL
//!
//! Authorization is provided via headers:
//! - Authorization: Bearer <base64-dag-cbor-ucan>
//! - X-UCAN-Proofs: <base64>,<base64>,... (comma-separated delegation proofs)

use crate::{
    error::{ErrorCode, ServiceError},
    r2::{Method, R2Config, presign},
    ucan::{VerificationError, verify_invocation},
};
use base64::Engine;
use worker::*;

/// GET /{space_did}/index/{digest} → 307 redirect to presigned GET URL
pub async fn handle_get(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    match handle_get_inner(&req, &ctx).await {
        Ok(response) => Ok(response),
        Err(err) => err.to_response(),
    }
}

/// PUT /{space_did}/index/{digest} → 307 redirect to presigned PUT URL
pub async fn handle_put(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    match handle_put_inner(&req, &ctx).await {
        Ok(response) => Ok(response),
        Err(err) => err.to_response(),
    }
}

async fn handle_get_inner(
    req: &Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    // 1. Extract path params
    let space_did = ctx
        .param("space_did")
        .ok_or_else(|| ServiceError::invalid_argument("Missing space_did in path"))?;
    let digest = ctx
        .param("digest")
        .ok_or_else(|| ServiceError::invalid_argument("Missing digest in path"))?;

    // 2. Parse and verify UCAN from headers
    let (ucan_bytes, proof_bytes) = parse_auth_headers(req)?;
    let verified = verify_invocation(&ucan_bytes, &proof_bytes)
        .await
        .map_err(map_verification_error)?;

    // 3. Validate command is /http/get
    if verified.command != vec!["http", "get"] {
        return Err(ServiceError::new(
            ErrorCode::CommandMismatch,
            format!(
                "Expected command 'http/get', got '{}'",
                verified.command.join("/")
            ),
        ));
    }

    // 4. Validate space_did matches verified subject
    if space_did != &verified.subject {
        return Err(ServiceError::invalid_argument(format!(
            "URL space ({}) does not match UCAN subject ({})",
            space_did, verified.subject
        )));
    }

    // 5. Generate presigned URL and return 307
    let r2_config = get_r2_config(ctx)?;
    let key = format!("{}/{}", space_did, digest);
    let presigned = presign::presign_url(&r2_config, Method::Get, &key, 3600)
        .map_err(|e| ServiceError::internal(format!("Presign failed: {}", e)))?;

    redirect_307(&presigned.url)
}

async fn handle_put_inner(
    req: &Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    // 1. Extract path params
    let space_did = ctx
        .param("space_did")
        .ok_or_else(|| ServiceError::invalid_argument("Missing space_did in path"))?;
    let digest = ctx
        .param("digest")
        .ok_or_else(|| ServiceError::invalid_argument("Missing digest in path"))?;

    // 2. Parse and verify UCAN from headers
    let (ucan_bytes, proof_bytes) = parse_auth_headers(req)?;
    let verified = verify_invocation(&ucan_bytes, &proof_bytes)
        .await
        .map_err(map_verification_error)?;

    // 3. Validate command is /http/put
    if verified.command != vec!["http", "put"] {
        return Err(ServiceError::new(
            ErrorCode::CommandMismatch,
            format!(
                "Expected command 'http/put', got '{}'",
                verified.command.join("/")
            ),
        ));
    }

    // 4. Validate space_did matches verified subject
    if space_did != &verified.subject {
        return Err(ServiceError::invalid_argument(format!(
            "URL space ({}) does not match UCAN subject ({})",
            space_did, verified.subject
        )));
    }

    // 5. Generate presigned URL and return 307
    // No existence check - always redirect, let R2 handle idempotency
    let r2_config = get_r2_config(ctx)?;
    let key = format!("{}/{}", space_did, digest);
    let presigned = presign::presign_url(&r2_config, Method::Put, &key, 3600)
        .map_err(|e| ServiceError::internal(format!("Presign failed: {}", e)))?;

    redirect_307(&presigned.url)
}

/// Parse Authorization and X-UCAN-Proofs headers.
fn parse_auth_headers(req: &Request) -> std::result::Result<(Vec<u8>, Vec<Vec<u8>>), ServiceError> {
    // Parse Authorization: Bearer <base64>
    let auth_header = req
        .headers()
        .get("Authorization")
        .map_err(|e| ServiceError::internal(format!("Header read error: {}", e)))?
        .ok_or_else(|| {
            ServiceError::new(ErrorCode::SignatureInvalid, "Missing Authorization header")
        })?;

    let ucan_b64 = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        ServiceError::invalid_argument("Authorization header must use Bearer scheme")
    })?;

    let ucan_bytes = base64::engine::general_purpose::STANDARD
        .decode(ucan_b64)
        .map_err(|e| {
            ServiceError::invalid_base64(format!("Invalid base64 in Authorization: {}", e))
        })?;

    // Parse X-UCAN-Proofs: <base64>,<base64>,...
    let proof_bytes: Vec<Vec<u8>> = req
        .headers()
        .get("X-UCAN-Proofs")
        .map_err(|e| ServiceError::internal(format!("Header read error: {}", e)))?
        .map(|header| {
            header
                .split(',')
                .enumerate()
                .map(|(i, p)| {
                    base64::engine::general_purpose::STANDARD
                        .decode(p.trim())
                        .map_err(|e| {
                            ServiceError::invalid_base64(format!(
                                "Invalid base64 in X-UCAN-Proofs[{}]: {}",
                                i, e
                            ))
                        })
                })
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok((ucan_bytes, proof_bytes))
}

/// Get R2 config from environment.
fn get_r2_config(ctx: &RouteContext<()>) -> std::result::Result<R2Config, ServiceError> {
    Ok(R2Config {
        account_id: ctx
            .var("R2_ACCOUNT_ID")
            .map_err(|e| ServiceError::internal(format!("Missing R2_ACCOUNT_ID: {}", e)))?
            .to_string(),
        access_key_id: ctx
            .secret("R2_ACCESS_KEY_ID")
            .map_err(|e| ServiceError::internal(format!("Missing R2_ACCESS_KEY_ID: {}", e)))?
            .to_string(),
        secret_access_key: ctx
            .secret("R2_SECRET_ACCESS_KEY")
            .map_err(|e| ServiceError::internal(format!("Missing R2_SECRET_ACCESS_KEY: {}", e)))?
            .to_string(),
        bucket: ctx
            .var("R2_BUCKET_NAME")
            .map_err(|e| ServiceError::internal(format!("Missing R2_BUCKET_NAME: {}", e)))?
            .to_string(),
    })
}

/// Create a 307 Temporary Redirect response.
fn redirect_307(url: &str) -> std::result::Result<Response, ServiceError> {
    let mut response = Response::empty()
        .map_err(|e| ServiceError::internal(format!("Response build failed: {}", e)))?;
    response
        .headers_mut()
        .set("Location", url)
        .map_err(|e| ServiceError::internal(format!("Header set failed: {}", e)))?;
    Ok(response.with_status(307))
}

/// Map VerificationError to ServiceError with appropriate error codes.
fn map_verification_error(err: VerificationError) -> ServiceError {
    match err {
        VerificationError::ParseError(msg) => ServiceError::invalid_cbor(msg),
        VerificationError::InvalidSignature(msg) => ServiceError::signature_invalid(msg),
        VerificationError::AudienceMismatch { expected, got } => {
            ServiceError::audience_mismatch(&expected, &got)
        }
        VerificationError::Expired => ServiceError::invocation_expired(),
        VerificationError::ChainInvalid(msg) => {
            if msg.contains("Subject not allowed") {
                ServiceError::subject_not_allowed()
            } else if msg.contains("Proof[") && msg.contains("expired") {
                ServiceError::new(ErrorCode::ProofExpired, msg)
            } else if msg.contains("Proof[") && msg.contains("not yet valid") {
                ServiceError::new(ErrorCode::ProofNotYetValid, msg)
            } else {
                ServiceError::chain_invalid(msg)
            }
        }
        VerificationError::Unauthorized(msg) => {
            if msg.contains("Command mismatch") {
                ServiceError::new(ErrorCode::CommandMismatch, msg)
            } else {
                ServiceError::chain_invalid(msg)
            }
        }
        VerificationError::ProofNotFound(cid) => ServiceError::proof_not_found(&cid),
        VerificationError::PolicyFailed(msg) => ServiceError::chain_invalid(msg),
    }
}
