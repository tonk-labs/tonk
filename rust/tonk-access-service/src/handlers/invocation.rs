//! UCAN invocation handler.
//!
//! Handles POST requests containing DAG-CBOR encoded UCAN invocations.

use crate::{
    error::{ErrorCode, ServiceError},
    identity::ServiceIdentity,
    r2::{Method, R2Config, presign},
    ucan::{BlobAllocate, BlobGet, Capability, VerificationError, verify_invocation},
};
use worker::*;

/// Request body structure.
/// The invocation and proofs are sent together.
#[derive(serde::Deserialize)]
struct InvocationRequest {
    /// DAG-CBOR encoded invocation (base64)
    invocation: String,
    /// DAG-CBOR encoded proofs (base64 array)
    proofs: Vec<String>,
}

/// Response for blob/allocate
#[derive(serde::Serialize)]
struct AllocateResponse {
    /// Bytes allocated (0 if already exists)
    size: u64,
    /// Upload address (None if blob exists)
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<UploadAddress>,
}

#[derive(serde::Serialize)]
struct UploadAddress {
    url: String,
    headers: std::collections::HashMap<String, String>,
    expires: u64,
}

/// Response for blob/get
#[derive(serde::Serialize)]
struct GetResponse {
    url: String,
    expires: u64,
}

pub async fn handle(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    match handle_inner(&mut req, &ctx).await {
        Ok(response) => Ok(response),
        Err(err) => err.to_response(),
    }
}

async fn handle_inner(
    req: &mut Request,
    ctx: &RouteContext<()>,
) -> std::result::Result<Response, ServiceError> {
    // Parse request body
    let body: InvocationRequest = req
        .json()
        .await
        .map_err(|e| ServiceError::invalid_request_body(format!("Invalid JSON: {}", e)))?;

    // Decode base64 invocation
    let invocation_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body.invocation)
            .map_err(|e| {
                ServiceError::invalid_base64(format!("Invalid base64 invocation: {}", e))
            })?;

    // Decode base64 proofs
    let proof_bytes: Vec<Vec<u8>> = body
        .proofs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, p).map_err(|e| {
                ServiceError::invalid_base64(format!("Invalid base64 proof[{}]: {}", i, e))
            })
        })
        .collect::<std::result::Result<_, _>>()?;

    // Get service identity
    let kv = ctx
        .kv("CONFIG")
        .map_err(|e| ServiceError::internal(format!("KV binding error: {}", e)))?;
    let identity = ServiceIdentity::get_or_create(&kv)
        .await
        .map_err(|e| ServiceError::internal(format!("Identity error: {}", e)))?;

    // Verify invocation
    let verified = verify_invocation(&invocation_bytes, &proof_bytes, identity.did())
        .await
        .map_err(map_verification_error)?;

    // Parse capability
    let capability =
        Capability::from_invocation(&verified.command, &verified.arguments).map_err(|e| {
            if e.contains("Unknown capability") {
                ServiceError::unknown_capability(&e)
            } else if e.contains("Missing") {
                ServiceError::new(ErrorCode::MissingArgument, e)
            } else {
                ServiceError::invalid_argument(e)
            }
        })?;

    // Get R2 config from environment
    let r2_config = R2Config {
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
    };

    // Get R2 bucket binding for existence checks
    let bucket = ctx
        .bucket("BUCKET")
        .map_err(|e| ServiceError::internal(format!("R2 bucket binding error: {}", e)))?;

    // Handle capability
    match capability {
        Capability::BlobAllocate(alloc) => handle_allocate(alloc, &r2_config, &bucket).await,
        Capability::BlobGet(get) => handle_get(get, &r2_config).await,
    }
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
        VerificationError::NotYetValid => ServiceError::new(
            ErrorCode::InvocationExpired,
            "Invocation not yet valid (nbf in future)",
        ),
        VerificationError::ChainInvalid(msg) => {
            // Check for specific chain errors
            if msg.contains("Subject not allowed") {
                ServiceError::subject_not_allowed()
            } else if msg.contains("Proof[") && msg.contains("expired") {
                // Extract index if possible
                ServiceError::new(ErrorCode::ProofExpired, msg)
            } else if msg.contains("Proof[") && msg.contains("not yet valid") {
                ServiceError::new(ErrorCode::ProofNotYetValid, msg)
            } else {
                ServiceError::chain_invalid(msg)
            }
        }
        VerificationError::MissingProofs => {
            ServiceError::new(ErrorCode::ProofNotFound, "No proofs provided")
        }
        VerificationError::Unauthorized(msg) => {
            if msg.contains("Command mismatch") {
                // Parse out the commands if possible
                ServiceError::new(ErrorCode::CommandMismatch, msg)
            } else {
                ServiceError::chain_invalid(msg)
            }
        }
        VerificationError::ProofNotFound(cid) => ServiceError::proof_not_found(&cid),
        VerificationError::PolicyFailed(msg) => ServiceError::chain_invalid(msg),
    }
}

async fn handle_allocate(
    alloc: BlobAllocate,
    config: &R2Config,
    bucket: &Bucket,
) -> std::result::Result<Response, ServiceError> {
    // Build object key: {space}/{digest_hex}
    let digest_hex = hex::encode(&alloc.blob.digest.bytes);
    let key = format!("{}/{}", alloc.space, digest_hex);

    // Check if blob already exists
    let exists = bucket
        .head(&key)
        .await
        .map(|obj| obj.is_some())
        .unwrap_or(false);

    if exists {
        // Blob already exists - return size 0, no address
        let response = AllocateResponse {
            size: 0,
            address: None,
        };
        return Response::from_json(&response)
            .map_err(|e| ServiceError::internal(format!("JSON serialization error: {}", e)));
    }

    // Generate pre-signed PUT URL
    let presigned = presign::presign_url(
        config,
        Method::Put,
        &key,
        3600, // 1 hour
        Some("application/octet-stream"),
    )
    .map_err(|e| ServiceError::internal(format!("Presign failed: {}", e)))?;

    let response = AllocateResponse {
        size: alloc.blob.size,
        address: Some(UploadAddress {
            url: presigned.url,
            headers: presigned.headers.into_iter().collect(),
            expires: presigned.expires_at,
        }),
    };

    Response::from_json(&response)
        .map_err(|e| ServiceError::internal(format!("JSON serialization error: {}", e)))
}

async fn handle_get(
    get: BlobGet,
    config: &R2Config,
) -> std::result::Result<Response, ServiceError> {
    // Build object key
    let digest_hex = hex::encode(&get.digest.bytes);
    let key = format!("{}/{}", get.space, digest_hex);

    // Generate pre-signed GET URL
    let presigned = presign::presign_url(
        config,
        Method::Get,
        &key,
        3600, // 1 hour
        None,
    )
    .map_err(|e| ServiceError::internal(format!("Presign failed: {}", e)))?;

    let response = GetResponse {
        url: presigned.url,
        expires: presigned.expires_at,
    };

    Response::from_json(&response)
        .map_err(|e| ServiceError::internal(format!("JSON serialization error: {}", e)))
}
