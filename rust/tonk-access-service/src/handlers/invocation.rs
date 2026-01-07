//! UCAN invocation handler.
//!
//! Handles POST requests containing DAG-CBOR encoded UCAN invocations.

use crate::{
    identity::ServiceIdentity,
    r2::{Method, R2Config, presign},
    ucan::{BlobAllocate, BlobGet, Capability, verify_invocation},
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
    // Parse request body
    let body: InvocationRequest = req
        .json()
        .await
        .map_err(|e| worker::Error::RustError(format!("Invalid request body: {}", e)))?;

    // Decode base64
    let invocation_bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &body.invocation)
            .map_err(|e| worker::Error::RustError(format!("Invalid base64 invocation: {}", e)))?;

    let proof_bytes: Vec<Vec<u8>> = body
        .proofs
        .iter()
        .map(|p| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, p))
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| worker::Error::RustError(format!("Invalid base64 proof: {}", e)))?;

    // Get service identity
    let kv = ctx.kv("CONFIG")?;
    let identity = ServiceIdentity::get_or_create(&kv)
        .await
        .map_err(|e| worker::Error::RustError(e.to_string()))?;

    // Verify invocation
    let verified = verify_invocation(&invocation_bytes, &proof_bytes, identity.did())
        .await
        .map_err(|e| worker::Error::RustError(format!("Verification failed: {}", e)))?;

    // Parse capability
    let capability = Capability::from_invocation(&verified.command, &verified.arguments)
        .map_err(|e| worker::Error::RustError(format!("Invalid capability: {}", e)))?;

    // Get R2 config from environment
    let r2_config = R2Config {
        account_id: ctx.var("R2_ACCOUNT_ID")?.to_string(),
        access_key_id: ctx.secret("R2_ACCESS_KEY_ID")?.to_string(),
        secret_access_key: ctx.secret("R2_SECRET_ACCESS_KEY")?.to_string(),
        bucket: ctx.var("R2_BUCKET_NAME")?.to_string(),
    };

    // Get R2 bucket binding for existence checks
    let bucket = ctx.bucket("BUCKET")?;

    // Handle capability
    match capability {
        Capability::BlobAllocate(alloc) => handle_allocate(alloc, &r2_config, &bucket).await,
        Capability::BlobGet(get) => handle_get(get, &r2_config).await,
    }
}

async fn handle_allocate(
    alloc: BlobAllocate,
    config: &R2Config,
    bucket: &Bucket,
) -> Result<Response> {
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
        return Response::from_json(&response);
    }

    // Generate pre-signed PUT URL
    let presigned = presign::presign_url(
        config,
        Method::Put,
        &key,
        3600, // 1 hour
        Some("application/octet-stream"),
    )
    .map_err(|e| worker::Error::RustError(format!("Presign failed: {}", e)))?;

    let response = AllocateResponse {
        size: alloc.blob.size,
        address: Some(UploadAddress {
            url: presigned.url,
            headers: presigned.headers.into_iter().collect(),
            expires: presigned.expires_at,
        }),
    };

    Response::from_json(&response)
}

async fn handle_get(get: BlobGet, config: &R2Config) -> Result<Response> {
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
    .map_err(|e| worker::Error::RustError(format!("Presign failed: {}", e)))?;

    let response = GetResponse {
        url: presigned.url,
        expires: presigned.expires_at,
    };

    Response::from_json(&response)
}
