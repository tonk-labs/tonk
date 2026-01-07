//! Test client for the UCAN Access Service.
//!
//! This CLI tool provides utilities for:
//! - Generating Ed25519 keypairs
//! - Creating UCAN delegations
//! - Sending invocations to the access service
//! - End-to-end testing
//!
//! Usage:
//!   cargo run --bin test-client --features test-client -- <command>

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use ucan::{
    Delegation,
    delegation::{builder::DelegationBuilder, subject::DelegatedSubject},
    did::{Ed25519Did, Ed25519Signer},
    invocation::builder::InvocationBuilder,
    promise::Promised,
};

#[derive(Parser)]
#[command(name = "test-client")]
#[command(about = "Test client for UCAN Access Service")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a new Ed25519 keypair
    GenerateKeypair,

    /// Create a delegation from space to operator
    Delegate {
        /// Base64-encoded 32-byte seed for space signing key
        #[arg(long)]
        space_key: String,

        /// Operator DID (did:key:z...)
        #[arg(long)]
        operator_did: String,

        /// Command to delegate (default: "/" for all)
        #[arg(long, default_value = "/")]
        command: String,
    },

    /// Send a blob/allocate invocation
    InvokeAllocate {
        /// Service URL (e.g., https://tonk-access-service.xxx.workers.dev)
        #[arg(long)]
        service_url: String,

        /// Base64-encoded 32-byte seed for operator signing key
        #[arg(long)]
        operator_key: String,

        /// Space DID (did:key:z...)
        #[arg(long)]
        space_did: String,

        /// Base64-encoded delegation (DAG-CBOR)
        #[arg(long)]
        delegation: String,

        /// Path to file to allocate
        #[arg(long)]
        file: PathBuf,
    },

    /// Send a blob/get invocation
    InvokeGet {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Base64-encoded 32-byte seed for operator signing key
        #[arg(long)]
        operator_key: String,

        /// Space DID (did:key:z...)
        #[arg(long)]
        space_did: String,

        /// Base64-encoded delegation (DAG-CBOR)
        #[arg(long)]
        delegation: String,

        /// Hex-encoded blob digest (SHA-256)
        #[arg(long)]
        digest: String,
    },

    /// Run end-to-end test
    E2eTest {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Test file content (default: "Hello, UCAN!")
        #[arg(long, default_value = "Hello, UCAN!")]
        content: String,
    },

    /// Get service info
    ServiceInfo {
        /// Service URL
        #[arg(long)]
        service_url: String,
    },
}

/// Service info response
#[derive(Debug, Deserialize)]
struct ServiceInfo {
    service: String,
    version: String,
    did: String,
}

/// Allocate request
#[derive(Debug, Serialize)]
struct InvocationRequest {
    invocation: String,
    proofs: Vec<String>,
}

/// Allocate response
#[derive(Debug, Deserialize)]
struct AllocateResponse {
    size: u64,
    address: Option<UploadAddress>,
}

#[derive(Debug, Deserialize)]
struct UploadAddress {
    url: String,
    headers: std::collections::HashMap<String, String>,
    expires: u64,
}

/// Get response
#[derive(Debug, Deserialize)]
struct GetResponse {
    url: String,
    expires: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::GenerateKeypair => cmd_generate_keypair(),
        Commands::Delegate {
            space_key,
            operator_did,
            command,
        } => cmd_delegate(&space_key, &operator_did, &command),
        Commands::InvokeAllocate {
            service_url,
            operator_key,
            space_did,
            delegation,
            file,
        } => cmd_invoke_allocate(&service_url, &operator_key, &space_did, &delegation, &file).await,
        Commands::InvokeGet {
            service_url,
            operator_key,
            space_did,
            delegation,
            digest,
        } => {
            cmd_invoke_get(
                &service_url,
                &operator_key,
                &space_did,
                &delegation,
                &digest,
            )
            .await
        }
        Commands::E2eTest {
            service_url,
            content,
        } => cmd_e2e_test(&service_url, &content).await,
        Commands::ServiceInfo { service_url } => cmd_service_info(&service_url).await,
    }
}

fn cmd_generate_keypair() -> anyhow::Result<()> {
    // Generate random seed
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed)?;

    let signing_key = SigningKey::from_bytes(&seed);
    let signer = Ed25519Signer::new(signing_key);

    println!("=== New Ed25519 Keypair ===");
    println!();
    println!("DID:        {}", signer.did());
    println!("Secret Key: {}", BASE64.encode(seed));
    println!();
    println!("Keep the secret key safe! You'll need it to sign delegations and invocations.");

    Ok(())
}

fn cmd_delegate(space_key_b64: &str, operator_did_str: &str, command: &str) -> anyhow::Result<()> {
    // Parse space key
    let space_seed = BASE64.decode(space_key_b64)?;
    let space_seed: [u8; 32] = space_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Space key must be 32 bytes"))?;
    let space_signing_key = SigningKey::from_bytes(&space_seed);
    let space_signer = Ed25519Signer::new(space_signing_key);

    // Parse operator DID
    let operator_did: Ed25519Did = operator_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid operator DID: {:?}", e))?;

    // Parse command
    let cmd_segments: Vec<String> = if command == "/" {
        vec![]
    } else {
        command.split('/').map(|s| s.to_string()).collect()
    };

    // Build delegation
    let delegation = DelegationBuilder::new()
        .issuer(space_signer.clone())
        .audience(operator_did)
        .subject(DelegatedSubject::Specific(*space_signer.did()))
        .command(cmd_segments)
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

    // Serialize to DAG-CBOR
    let cbor_bytes = serde_ipld_dagcbor::to_vec(&delegation)?;
    let cbor_b64 = BASE64.encode(&cbor_bytes);

    // Compute CID for reference
    let cid = delegation.to_cid();

    println!("=== Delegation Created ===");
    println!();
    println!("Issuer (Space):   {}", space_signer.did());
    println!("Audience (Op):    {}", operator_did);
    println!("Subject:          {}", space_signer.did());
    println!(
        "Command:          {}",
        if command == "/" { "/" } else { command }
    );
    println!("CID:              {}", cid);
    println!();
    println!("Delegation (base64 DAG-CBOR):");
    println!("{}", cbor_b64);

    Ok(())
}

async fn cmd_invoke_allocate(
    service_url: &str,
    operator_key_b64: &str,
    space_did_str: &str,
    delegation_b64: &str,
    file_path: &PathBuf,
) -> anyhow::Result<()> {
    // Parse operator key
    let operator_seed = BASE64.decode(operator_key_b64)?;
    let operator_seed: [u8; 32] = operator_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Operator key must be 32 bytes"))?;
    let operator_signing_key = SigningKey::from_bytes(&operator_seed);
    let operator_signer = Ed25519Signer::new(operator_signing_key);

    // Parse space DID
    let space_did: Ed25519Did = space_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid space DID: {:?}", e))?;

    // Parse delegation
    let delegation_bytes = BASE64.decode(delegation_b64)?;
    let delegation: Delegation<Ed25519Did> = serde_ipld_dagcbor::from_slice(&delegation_bytes)?;
    let delegation_cid = delegation.to_cid();

    // Read file and compute digest
    let file_content = tokio::fs::read(file_path).await?;
    let file_size = file_content.len() as u64;
    let digest = Sha256::digest(&file_content);

    // Get service DID
    let client = Client::new();
    let info: ServiceInfo = client.get(service_url).send().await?.json().await?;
    let service_did: Ed25519Did = info
        .did
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid service DID: {:?}", e))?;

    println!("Service DID: {}", info.did);
    println!("File size: {} bytes", file_size);
    println!("SHA-256 digest: {}", hex::encode(digest));

    // Build arguments
    let mut args: BTreeMap<String, Promised> = BTreeMap::new();
    args.insert(
        "space".to_string(),
        Promised::String(space_did_str.to_string()),
    );

    let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
    blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
    // Use String instead of Integer to avoid i128 deserialization issues with serde_ipld_dagcbor
    blob_map.insert("size".to_string(), Promised::String(file_size.to_string()));
    args.insert("blob".to_string(), Promised::Map(blob_map));

    // Build invocation
    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(service_did)
        .subject(space_did)
        .command(vec!["blob".to_string(), "allocate".to_string()])
        .arguments(args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    // Serialize
    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    // Build request
    let request = InvocationRequest {
        invocation: BASE64.encode(&invocation_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

    // Send request
    println!("\nSending blob/allocate invocation...");
    let response = client.post(service_url).json(&request).send().await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        println!("Error ({}): {}", status, body);
        return Err(anyhow::anyhow!("Request failed"));
    }

    let allocate_response: AllocateResponse = serde_json::from_str(&body)?;

    println!("\n=== Allocate Response ===");
    println!("Size: {}", allocate_response.size);

    if let Some(address) = allocate_response.address {
        println!("Upload URL: {}", address.url);
        println!("Headers: {:?}", address.headers);
        println!("Expires: {}", address.expires);

        // Upload the file
        println!("\nUploading file...");
        let mut upload_request = client.put(&address.url).body(file_content);
        for (k, v) in &address.headers {
            upload_request = upload_request.header(k, v);
        }
        let upload_response = upload_request.send().await?;

        if upload_response.status().is_success() {
            println!("Upload successful!");
        } else {
            println!(
                "Upload failed: {} - {}",
                upload_response.status(),
                upload_response.text().await?
            );
        }
    } else {
        println!("Blob already exists (no upload needed)");
    }

    Ok(())
}

async fn cmd_invoke_get(
    service_url: &str,
    operator_key_b64: &str,
    space_did_str: &str,
    delegation_b64: &str,
    digest_hex: &str,
) -> anyhow::Result<()> {
    // Parse operator key
    let operator_seed = BASE64.decode(operator_key_b64)?;
    let operator_seed: [u8; 32] = operator_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Operator key must be 32 bytes"))?;
    let operator_signing_key = SigningKey::from_bytes(&operator_seed);
    let operator_signer = Ed25519Signer::new(operator_signing_key);

    // Parse space DID
    let space_did: Ed25519Did = space_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid space DID: {:?}", e))?;

    // Parse delegation
    let delegation_bytes = BASE64.decode(delegation_b64)?;
    let delegation: Delegation<Ed25519Did> = serde_ipld_dagcbor::from_slice(&delegation_bytes)?;
    let delegation_cid = delegation.to_cid();

    // Parse digest
    let digest = hex::decode(digest_hex)?;

    // Get service DID
    let client = Client::new();
    let info: ServiceInfo = client.get(service_url).send().await?.json().await?;
    let service_did: Ed25519Did = info
        .did
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid service DID: {:?}", e))?;

    println!("Service DID: {}", info.did);

    // Build arguments
    let mut args: BTreeMap<String, Promised> = BTreeMap::new();
    args.insert(
        "space".to_string(),
        Promised::String(space_did_str.to_string()),
    );
    args.insert("digest".to_string(), Promised::Bytes(digest));

    // Build invocation
    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(service_did)
        .subject(space_did)
        .command(vec!["blob".to_string(), "get".to_string()])
        .arguments(args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    // Serialize
    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    // Build request
    let request = InvocationRequest {
        invocation: BASE64.encode(&invocation_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

    // Send request
    println!("\nSending blob/get invocation...");
    let response = client.post(service_url).json(&request).send().await?;

    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        println!("Error ({}): {}", status, body);
        return Err(anyhow::anyhow!("Request failed"));
    }

    let get_response: GetResponse = serde_json::from_str(&body)?;

    println!("\n=== Get Response ===");
    println!("Download URL: {}", get_response.url);
    println!("Expires: {}", get_response.expires);

    // Download the content
    println!("\nDownloading...");
    let download_response = client.get(&get_response.url).send().await?;

    if download_response.status().is_success() {
        let content = download_response.bytes().await?;
        println!("Downloaded {} bytes", content.len());
        if let Ok(text) = std::str::from_utf8(&content) {
            println!("Content: {}", text);
        }
    } else {
        println!(
            "Download failed: {} - {}",
            download_response.status(),
            download_response.text().await?
        );
    }

    Ok(())
}

async fn cmd_e2e_test(service_url: &str, content: &str) -> anyhow::Result<()> {
    println!("=== End-to-End Test ===\n");

    let client = Client::new();

    // Step 1: Get service info
    println!("Step 1: Getting service info...");
    let info: ServiceInfo = client.get(service_url).send().await?.json().await?;
    println!("  Service: {}", info.service);
    println!("  Version: {}", info.version);
    println!("  DID: {}", info.did);

    let service_did: Ed25519Did = info
        .did
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid service DID: {:?}", e))?;

    // Step 2: Generate keypairs
    println!("\nStep 2: Generating keypairs...");

    let mut space_seed = [0u8; 32];
    getrandom::getrandom(&mut space_seed)?;
    let space_signer = Ed25519Signer::new(SigningKey::from_bytes(&space_seed));
    println!("  Space DID: {}", space_signer.did());

    let mut operator_seed = [0u8; 32];
    getrandom::getrandom(&mut operator_seed)?;
    let operator_signer = Ed25519Signer::new(SigningKey::from_bytes(&operator_seed));
    println!("  Operator DID: {}", operator_signer.did());

    // Step 3: Create delegation
    println!("\nStep 3: Creating delegation (space -> operator)...");
    let delegation = DelegationBuilder::new()
        .issuer(space_signer.clone())
        .audience(*operator_signer.did())
        .subject(DelegatedSubject::Specific(*space_signer.did()))
        .command(vec!["blob".to_string()])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

    let delegation_bytes = serde_ipld_dagcbor::to_vec(&delegation)?;
    let delegation_cid = delegation.to_cid();
    println!("  Delegation CID: {}", delegation_cid);

    // Step 4: Prepare test content
    println!("\nStep 4: Preparing test content...");
    let content_bytes = content.as_bytes();
    let content_size = content_bytes.len() as u64;
    let digest = Sha256::digest(content_bytes);
    println!("  Content: \"{}\"", content);
    println!("  Size: {} bytes", content_size);
    println!("  SHA-256: {}", hex::encode(digest));

    // Step 5: Send blob/allocate
    println!("\nStep 5: Sending blob/allocate invocation...");

    let mut alloc_args: BTreeMap<String, Promised> = BTreeMap::new();
    alloc_args.insert(
        "space".to_string(),
        Promised::String(space_signer.did().to_string()),
    );
    let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
    blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
    // Use String instead of Integer to avoid i128 deserialization issues with serde_ipld_dagcbor
    blob_map.insert(
        "size".to_string(),
        Promised::String(content_size.to_string()),
    );
    alloc_args.insert("blob".to_string(), Promised::Map(blob_map));

    let alloc_invocation = InvocationBuilder::new()
        .issuer(operator_signer.clone())
        .audience(service_did)
        .subject(*space_signer.did())
        .command(vec!["blob".to_string(), "allocate".to_string()])
        .arguments(alloc_args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build allocate invocation: {:?}", e))?;

    let alloc_inv_bytes = serde_ipld_dagcbor::to_vec(&alloc_invocation)?;

    let alloc_request = InvocationRequest {
        invocation: BASE64.encode(&alloc_inv_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

    let alloc_response = client.post(service_url).json(&alloc_request).send().await?;

    let alloc_status = alloc_response.status();
    let alloc_body = alloc_response.text().await?;

    if !alloc_status.is_success() {
        println!("  FAILED: {} - {}", alloc_status, alloc_body);
        return Err(anyhow::anyhow!("Allocate failed"));
    }

    let alloc_result: AllocateResponse = serde_json::from_str(&alloc_body)?;
    println!("  Response size: {}", alloc_result.size);

    // Step 6: Upload blob
    if let Some(address) = alloc_result.address {
        println!("\nStep 6: Uploading blob to presigned URL...");
        println!("  URL: {}...", &address.url[..80.min(address.url.len())]);

        let mut upload_request = client.put(&address.url).body(content_bytes.to_vec());
        for (k, v) in &address.headers {
            upload_request = upload_request.header(k, v);
        }
        let upload_response = upload_request.send().await?;

        if upload_response.status().is_success() {
            println!("  Upload: SUCCESS");
        } else {
            let err_body = upload_response.text().await?;
            println!("  Upload: FAILED - {}", err_body);
            return Err(anyhow::anyhow!("Upload failed"));
        }
    } else {
        println!("\nStep 6: Blob already exists, skipping upload");
    }

    // Step 7: Send blob/get
    println!("\nStep 7: Sending blob/get invocation...");

    let mut get_args: BTreeMap<String, Promised> = BTreeMap::new();
    get_args.insert(
        "space".to_string(),
        Promised::String(space_signer.did().to_string()),
    );
    get_args.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));

    let get_invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(service_did)
        .subject(*space_signer.did())
        .command(vec!["blob".to_string(), "get".to_string()])
        .arguments(get_args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build get invocation: {:?}", e))?;

    let get_inv_bytes = serde_ipld_dagcbor::to_vec(&get_invocation)?;

    let get_request = InvocationRequest {
        invocation: BASE64.encode(&get_inv_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

    let get_response = client.post(service_url).json(&get_request).send().await?;

    let get_status = get_response.status();
    let get_body = get_response.text().await?;

    if !get_status.is_success() {
        println!("  FAILED: {} - {}", get_status, get_body);
        return Err(anyhow::anyhow!("Get failed"));
    }

    let get_result: GetResponse = serde_json::from_str(&get_body)?;
    println!("  Got presigned URL");

    // Step 8: Download and verify
    println!("\nStep 8: Downloading and verifying content...");
    let download_response = client.get(&get_result.url).send().await?;

    if !download_response.status().is_success() {
        println!("  Download: FAILED");
        return Err(anyhow::anyhow!("Download failed"));
    }

    let downloaded = download_response.bytes().await?;
    let downloaded_str = std::str::from_utf8(&downloaded)?;

    if downloaded_str == content {
        println!("  Content matches: SUCCESS");
    } else {
        println!("  Content mismatch!");
        println!("  Expected: \"{}\"", content);
        println!("  Got: \"{}\"", downloaded_str);
        return Err(anyhow::anyhow!("Content verification failed"));
    }

    println!("\n=== All Tests Passed! ===");
    println!("\nTest artifacts:");
    println!("  Space Key: {}", BASE64.encode(space_seed));
    println!("  Operator Key: {}", BASE64.encode(operator_seed));
    println!("  Delegation: {}", BASE64.encode(&delegation_bytes));

    Ok(())
}

async fn cmd_service_info(service_url: &str) -> anyhow::Result<()> {
    let client = Client::new();
    let response = client.get(service_url).send().await?;

    if !response.status().is_success() {
        let body = response.text().await?;
        println!("Error: {}", body);
        return Err(anyhow::anyhow!("Request failed"));
    }

    let info: ServiceInfo = response.json().await?;

    println!("=== Service Info ===");
    println!("Service: {}", info.service);
    println!("Version: {}", info.version);
    println!("DID:     {}", info.did);

    Ok(())
}
