//! Test client for the UCAN Access Service.
//!
//! This CLI tool provides utilities for:
//! - Generating Ed25519 keypairs
//! - Creating UCAN delegations
//! - Sending invocations to the access service
//! - End-to-end testing
//! - Comprehensive edge case and failure mode testing
//!
//! Usage:
//!   cargo run --bin test-client --features test-client -- <command>

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use ipld_core::cid::Cid;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use ucan::{
    Delegation,
    delegation::{builder::DelegationBuilder, subject::DelegatedSubject},
    did::{Ed25519Did, Ed25519Signer},
    invocation::builder::InvocationBuilder,
    promise::Promised,
    time::timestamp::Timestamp,
};

// ============================================================================
// CLI Structure
// ============================================================================

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

    /// Run authentication and authorization failure tests
    TestAuth {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Run input validation tests
    TestValidation {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Run delegation chain scenario tests
    TestDelegation {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Run storage edge case tests
    TestStorage {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },

    /// Run all test suites
    TestAll {
        /// Service URL
        #[arg(long)]
        service_url: String,

        /// Verbose output
        #[arg(long, short)]
        verbose: bool,
    },
}

// ============================================================================
// API Types
// ============================================================================

/// Service info response
#[derive(Debug, Deserialize)]
struct ServiceInfo {
    service: String,
    version: String,
}

/// Invocation request
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
    #[allow(dead_code)]
    expires: u64,
}

/// Get response
#[derive(Debug, Deserialize)]
struct GetResponse {
    url: String,
    #[allow(dead_code)]
    expires: u64,
}

/// Error response from the service
#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Result of a single test
#[derive(Debug)]
struct TestResult {
    name: String,
    passed: bool,
    message: String,
}

impl TestResult {
    fn pass(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: true,
            message: String::new(),
        }
    }

    fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            passed: false,
            message: message.into(),
        }
    }
}

/// Test suite runner
struct TestSuite {
    name: String,
    results: Vec<TestResult>,
    verbose: bool,
}

impl TestSuite {
    fn new(name: impl Into<String>, verbose: bool) -> Self {
        Self {
            name: name.into(),
            results: Vec::new(),
            verbose,
        }
    }

    fn record(&mut self, result: TestResult) {
        if self.verbose {
            let status = if result.passed { "PASS" } else { "FAIL" };
            println!("  [{}] {}", status, result.name);
            if !result.passed && !result.message.is_empty() {
                println!("        {}", result.message);
            }
        }
        self.results.push(result);
    }

    fn summary(&self) -> (usize, usize) {
        let passed = self.results.iter().filter(|r| r.passed).count();
        let failed = self.results.len() - passed;
        (passed, failed)
    }

    fn print_summary(&self) {
        let (passed, failed) = self.summary();
        let status = if failed == 0 { "PASSED" } else { "FAILED" };
        println!(
            "\n{}: {} ({} passed, {} failed)",
            self.name, status, passed, failed
        );

        if !self.verbose && failed > 0 {
            println!("  Failed tests:");
            for result in &self.results {
                if !result.passed {
                    println!("    - {}: {}", result.name, result.message);
                }
            }
        }
    }

    fn all_passed(&self) -> bool {
        self.results.iter().all(|r| r.passed)
    }
}

/// Test context with pre-setup keys and service info
struct TestContext {
    client: Client,
    service_url: String,
    space_signer: Ed25519Signer,
    operator_signer: Ed25519Signer,
    delegation_bytes: Vec<u8>,
    delegation_cid: Cid,
}

impl TestContext {
    async fn new(service_url: &str) -> anyhow::Result<Self> {
        let client = Client::new();

        // Verify service is reachable
        let _info: ServiceInfo = client.get(service_url).send().await?.json().await?;

        // Generate test keypairs
        let mut space_seed = [0u8; 32];
        getrandom::getrandom(&mut space_seed)?;
        let space_signer = Ed25519Signer::new(SigningKey::from_bytes(&space_seed));

        let mut operator_seed = [0u8; 32];
        getrandom::getrandom(&mut operator_seed)?;
        let operator_signer = Ed25519Signer::new(SigningKey::from_bytes(&operator_seed));

        // Create delegation (space -> operator)
        let delegation = DelegationBuilder::new()
            .issuer(space_signer.clone())
            .audience(*operator_signer.did())
            .subject(DelegatedSubject::Specific(*space_signer.did()))
            .command(vec!["blob".to_string()])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let delegation_bytes = serde_ipld_dagcbor::to_vec(&delegation)?;
        let delegation_cid = delegation.to_cid();

        Ok(Self {
            client,
            service_url: service_url.to_string(),
            space_signer,
            operator_signer,
            delegation_bytes,
            delegation_cid,
        })
    }

    /// Build allocate arguments for testing
    fn build_allocate_args(&self, digest: &[u8], size: u64) -> BTreeMap<String, Promised> {
        let mut args: BTreeMap<String, Promised> = BTreeMap::new();
        args.insert(
            "space".to_string(),
            Promised::String(self.space_signer.did().to_string()),
        );

        let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
        blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
        blob_map.insert("size".to_string(), Promised::String(size.to_string()));
        args.insert("blob".to_string(), Promised::Map(blob_map));

        args
    }

    /// Build a valid invocation request.
    fn build_invocation(
        &self,
        issuer: &Ed25519Signer,
        subject: Ed25519Did,
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
        proofs: Vec<Cid>,
    ) -> anyhow::Result<(Vec<u8>, InvocationRequest)> {
        let invocation = InvocationBuilder::new()
            .issuer(issuer.clone())
            .audience(subject)
            .subject(subject)
            .command(command)
            .arguments(args)
            .proofs(proofs)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

        let request = InvocationRequest {
            invocation: BASE64.encode(&invocation_bytes),
            proofs: vec![BASE64.encode(&self.delegation_bytes)],
        };

        Ok((invocation_bytes, request))
    }

    /// Build an invocation with custom audience (for testing audience mismatch).
    fn build_invocation_with_audience(
        &self,
        issuer: &Ed25519Signer,
        audience: Ed25519Did,
        subject: Ed25519Did,
        command: Vec<String>,
        args: BTreeMap<String, Promised>,
        proofs: Vec<Cid>,
    ) -> anyhow::Result<(Vec<u8>, InvocationRequest)> {
        let invocation = InvocationBuilder::new()
            .issuer(issuer.clone())
            .audience(audience)
            .subject(subject)
            .command(command)
            .arguments(args)
            .proofs(proofs)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

        let request = InvocationRequest {
            invocation: BASE64.encode(&invocation_bytes),
            proofs: vec![BASE64.encode(&self.delegation_bytes)],
        };

        Ok((invocation_bytes, request))
    }

    /// Send a request and check for expected error
    async fn expect_error(&self, request: &InvocationRequest, expected_code: &str) -> TestResult {
        let response = match self
            .client
            .post(&self.service_url)
            .json(request)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return TestResult::fail(
                    format!("expect_{}", expected_code.to_lowercase()),
                    format!("Request failed: {}", e),
                );
            }
        };

        let status = response.status();
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                return TestResult::fail(
                    format!("expect_{}", expected_code.to_lowercase()),
                    format!("Failed to read response: {}", e),
                );
            }
        };

        // Should be an error response
        if status.is_success() {
            return TestResult::fail(
                format!("expect_{}", expected_code.to_lowercase()),
                format!("Expected error {} but got success: {}", expected_code, body),
            );
        }

        // Parse error response
        match serde_json::from_str::<ErrorResponse>(&body) {
            Ok(err) => {
                if err.error.code == expected_code {
                    TestResult::pass(format!("expect_{}", expected_code.to_lowercase()))
                } else {
                    TestResult::fail(
                        format!("expect_{}", expected_code.to_lowercase()),
                        format!(
                            "Expected error code {}, got {}: {}",
                            expected_code, err.error.code, err.error.message
                        ),
                    )
                }
            }
            Err(_) => TestResult::fail(
                format!("expect_{}", expected_code.to_lowercase()),
                format!(
                    "Expected structured error response, got: {} {}",
                    status, body
                ),
            ),
        }
    }

    /// Send a request and expect success
    async fn expect_success(&self, request: &InvocationRequest, test_name: &str) -> TestResult {
        let response = match self
            .client
            .post(&self.service_url)
            .json(request)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return TestResult::fail(test_name, format!("Request failed: {}", e)),
        };

        let status = response.status();
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                return TestResult::fail(test_name, format!("Failed to read response: {}", e));
            }
        };

        if status.is_success() {
            TestResult::pass(test_name)
        } else {
            TestResult::fail(
                test_name,
                format!("Expected success, got {} {}", status, body),
            )
        }
    }
}

// ============================================================================
// Main
// ============================================================================

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
        Commands::TestAuth {
            service_url,
            verbose,
        } => cmd_test_auth(&service_url, verbose).await,
        Commands::TestValidation {
            service_url,
            verbose,
        } => cmd_test_validation(&service_url, verbose).await,
        Commands::TestDelegation {
            service_url,
            verbose,
        } => cmd_test_delegation(&service_url, verbose).await,
        Commands::TestStorage {
            service_url,
            verbose,
        } => cmd_test_storage(&service_url, verbose).await,
        Commands::TestAll {
            service_url,
            verbose,
        } => cmd_test_all(&service_url, verbose).await,
    }
}

// ============================================================================
// Original Commands
// ============================================================================

fn cmd_generate_keypair() -> anyhow::Result<()> {
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
    let space_seed = BASE64.decode(space_key_b64)?;
    let space_seed: [u8; 32] = space_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Space key must be 32 bytes"))?;
    let space_signing_key = SigningKey::from_bytes(&space_seed);
    let space_signer = Ed25519Signer::new(space_signing_key);

    let operator_did: Ed25519Did = operator_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid operator DID: {:?}", e))?;

    let cmd_segments: Vec<String> = if command == "/" {
        vec![]
    } else {
        command
            .trim_start_matches('/')
            .split('/')
            .map(|s| s.to_string())
            .collect()
    };

    let delegation = DelegationBuilder::new()
        .issuer(space_signer.clone())
        .audience(operator_did)
        .subject(DelegatedSubject::Specific(*space_signer.did()))
        .command(cmd_segments)
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

    let cbor_bytes = serde_ipld_dagcbor::to_vec(&delegation)?;
    let cbor_b64 = BASE64.encode(&cbor_bytes);
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
    let operator_seed = BASE64.decode(operator_key_b64)?;
    let operator_seed: [u8; 32] = operator_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Operator key must be 32 bytes"))?;
    let operator_signing_key = SigningKey::from_bytes(&operator_seed);
    let operator_signer = Ed25519Signer::new(operator_signing_key);

    let space_did: Ed25519Did = space_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid space DID: {:?}", e))?;

    let delegation_bytes = BASE64.decode(delegation_b64)?;
    let delegation: Delegation<Ed25519Did> = serde_ipld_dagcbor::from_slice(&delegation_bytes)?;
    let delegation_cid = delegation.to_cid();

    let file_content = tokio::fs::read(file_path).await?;
    let file_size = file_content.len() as u64;
    let digest = Sha256::digest(&file_content);

    let client = Client::new();

    println!("Space DID: {}", space_did_str);
    println!("File size: {} bytes", file_size);
    println!("SHA-256 digest: {}", hex::encode(digest));

    let mut args: BTreeMap<String, Promised> = BTreeMap::new();
    args.insert(
        "space".to_string(),
        Promised::String(space_did_str.to_string()),
    );

    let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
    blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
    blob_map.insert("size".to_string(), Promised::String(file_size.to_string()));
    args.insert("blob".to_string(), Promised::Map(blob_map));

    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(space_did)
        .subject(space_did)
        .command(vec!["blob".to_string(), "allocate".to_string()])
        .arguments(args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    let request = InvocationRequest {
        invocation: BASE64.encode(&invocation_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

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
    let operator_seed = BASE64.decode(operator_key_b64)?;
    let operator_seed: [u8; 32] = operator_seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Operator key must be 32 bytes"))?;
    let operator_signing_key = SigningKey::from_bytes(&operator_seed);
    let operator_signer = Ed25519Signer::new(operator_signing_key);

    let space_did: Ed25519Did = space_did_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid space DID: {:?}", e))?;

    let delegation_bytes = BASE64.decode(delegation_b64)?;
    let delegation: Delegation<Ed25519Did> = serde_ipld_dagcbor::from_slice(&delegation_bytes)?;
    let delegation_cid = delegation.to_cid();

    let digest = hex::decode(digest_hex)?;

    let client = Client::new();

    println!("Space DID: {}", space_did_str);

    let mut args: BTreeMap<String, Promised> = BTreeMap::new();
    args.insert(
        "space".to_string(),
        Promised::String(space_did_str.to_string()),
    );
    args.insert("digest".to_string(), Promised::Bytes(digest));

    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(space_did)
        .subject(space_did)
        .command(vec!["blob".to_string(), "get".to_string()])
        .arguments(args)
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    let request = InvocationRequest {
        invocation: BASE64.encode(&invocation_bytes),
        proofs: vec![BASE64.encode(&delegation_bytes)],
    };

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
    blob_map.insert(
        "size".to_string(),
        Promised::String(content_size.to_string()),
    );
    alloc_args.insert("blob".to_string(), Promised::Map(blob_map));

    let alloc_invocation = InvocationBuilder::new()
        .issuer(operator_signer.clone())
        .audience(*space_signer.did())
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
        .audience(*space_signer.did())
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
    println!("Service:  {}", info.service);
    println!("Version:  {}", info.version);

    Ok(())
}

// ============================================================================
// Test Suites
// ============================================================================

/// Test authentication and authorization failures
async fn cmd_test_auth(service_url: &str, verbose: bool) -> anyhow::Result<()> {
    println!("=== Authentication & Authorization Tests ===\n");

    let ctx = TestContext::new(service_url).await?;
    let mut suite = TestSuite::new("Auth Tests", verbose);

    let digest = Sha256::digest(b"test content");
    let args = ctx.build_allocate_args(&digest, 12);

    // Test 1: Invalid signature (tamper with invocation bytes)
    {
        let (mut inv_bytes, _) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args.clone(),
            vec![ctx.delegation_cid],
        )?;

        // Tamper with the bytes
        let mid = inv_bytes.len() / 2;
        if let Some(byte) = inv_bytes.get_mut(mid) {
            *byte = byte.wrapping_add(1);
        }

        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&ctx.delegation_bytes)],
        };

        // Should fail with either signature invalid or parse error
        let response = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "SIGNATURE_INVALID" || err.error.code == "INVALID_CBOR" {
                    suite.record(TestResult::pass("tampered_signature"));
                } else {
                    suite.record(TestResult::fail(
                        "tampered_signature",
                        format!(
                            "Expected SIGNATURE_INVALID or INVALID_CBOR, got {}",
                            err.error.code
                        ),
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "tampered_signature",
                    format!("Got error but couldn't parse: {}", body),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "tampered_signature",
                "Expected error but got success",
            ));
        }
    }

    // Test 2: Wrong audience (audience != subject)
    {
        // Generate a different DID to use as wrong audience
        let mut wrong_seed = [0u8; 32];
        getrandom::getrandom(&mut wrong_seed)?;
        let wrong_signer = Ed25519Signer::new(SigningKey::from_bytes(&wrong_seed));

        let (_, request) = ctx.build_invocation_with_audience(
            &ctx.operator_signer,
            *wrong_signer.did(), // Wrong audience
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args.clone(),
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_error(&request, "AUDIENCE_MISMATCH").await);
    }

    // Test 3: Expired invocation
    {
        // Build invocation with past expiration
        // Create a timestamp in the past (1 hour ago)
        let past_time = SystemTime::now() - Duration::from_secs(3600);
        let past_exp = Timestamp::new(past_time).unwrap();

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args.clone())
            .proofs(vec![ctx.delegation_cid])
            .expiration(past_exp)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&ctx.delegation_bytes)],
        };

        suite.record(ctx.expect_error(&request, "INVOCATION_EXPIRED").await);
    }

    // Test 4: Missing proofs (empty proofs array)
    {
        let (_, mut request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args.clone(),
            vec![ctx.delegation_cid],
        )?;

        request.proofs = vec![]; // Empty proofs!

        suite.record(ctx.expect_error(&request, "PROOF_NOT_FOUND").await);
    }

    // Test 5: Wrong proof CID (reference non-existent delegation)
    {
        // Generate a random CID that won't match
        let fake_cid_bytes = [0u8; 32];
        let fake_cid = ipld_core::cid::Cid::new_v1(
            0x71, // dag-cbor
            ipld_core::cid::multihash::Multihash::wrap(0x12, &fake_cid_bytes).unwrap(),
        );

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args.clone())
            .proofs(vec![fake_cid]) // Wrong CID!
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&ctx.delegation_bytes)],
        };

        suite.record(ctx.expect_error(&request, "PROOF_NOT_FOUND").await);
    }

    // Test 6: Wrong subject (invoke for different space)
    {
        // Generate a different space
        let mut other_seed = [0u8; 32];
        getrandom::getrandom(&mut other_seed)?;
        let other_space = Ed25519Signer::new(SigningKey::from_bytes(&other_seed));

        // But use the delegation for the original space
        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *other_space.did(), // Wrong subject!
            vec!["blob".to_string(), "allocate".to_string()],
            args.clone(),
            vec![ctx.delegation_cid],
        )?;

        // Should fail - subject doesn't match delegation
        let response = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "SUBJECT_NOT_ALLOWED" || err.error.code == "CHAIN_INVALID" {
                    suite.record(TestResult::pass("wrong_subject"));
                } else {
                    suite.record(TestResult::fail(
                        "wrong_subject",
                        format!(
                            "Expected SUBJECT_NOT_ALLOWED or CHAIN_INVALID, got {}",
                            err.error.code
                        ),
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "wrong_subject",
                    format!("Got error but couldn't parse: {}", body),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "wrong_subject",
                "Expected error but got success",
            ));
        }
    }

    // Test 7: Operator signs as if they were the space (invalid issuer chain)
    {
        // Operator tries to invoke directly without delegation chain
        // The invocation's issuer is operator, but the proof chain doesn't connect
        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args.clone())
            .proofs(vec![]) // No proofs - should fail
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![], // No proofs
        };

        // Should fail with chain invalid or proof not found
        let response = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "CHAIN_INVALID" || err.error.code == "PROOF_NOT_FOUND" {
                    suite.record(TestResult::pass("no_delegation_chain"));
                } else {
                    suite.record(TestResult::fail(
                        "no_delegation_chain",
                        format!(
                            "Expected CHAIN_INVALID or PROOF_NOT_FOUND, got {}",
                            err.error.code
                        ),
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "no_delegation_chain",
                    format!("Got error but couldn't parse: {}", body),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "no_delegation_chain",
                "Expected error but got success",
            ));
        }
    }

    suite.print_summary();

    if suite.all_passed() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some auth tests failed"))
    }
}

/// Test input validation
async fn cmd_test_validation(service_url: &str, verbose: bool) -> anyhow::Result<()> {
    println!("=== Input Validation Tests ===\n");

    let ctx = TestContext::new(service_url).await?;
    let mut suite = TestSuite::new("Validation Tests", verbose);

    // Test 1: Malformed JSON body
    {
        let response = ctx
            .client
            .post(&ctx.service_url)
            .header("content-type", "application/json")
            .body("not valid json {{{")
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "INVALID_REQUEST_BODY" {
                    suite.record(TestResult::pass("malformed_json"));
                } else {
                    suite.record(TestResult::fail(
                        "malformed_json",
                        format!("Expected INVALID_REQUEST_BODY, got {}", err.error.code),
                    ));
                }
            } else {
                // Some HTTP error without our structured format is also acceptable
                suite.record(TestResult::pass("malformed_json"));
            }
        } else {
            suite.record(TestResult::fail(
                "malformed_json",
                "Expected error but got success",
            ));
        }
    }

    // Test 2: Invalid base64 in invocation field
    {
        let request = InvocationRequest {
            invocation: "not-valid-base64!!!".to_string(),
            proofs: vec![BASE64.encode(&ctx.delegation_bytes)],
        };

        suite.record(ctx.expect_error(&request, "INVALID_BASE64").await);
    }

    // Test 3: Valid base64 but invalid CBOR
    {
        let request = InvocationRequest {
            invocation: BASE64.encode(b"this is valid base64 but not CBOR"),
            proofs: vec![BASE64.encode(&ctx.delegation_bytes)],
        };

        suite.record(ctx.expect_error(&request, "INVALID_CBOR").await);
    }

    // Test 4: Missing 'space' argument
    {
        let digest = Sha256::digest(b"test");
        let mut args: BTreeMap<String, Promised> = BTreeMap::new();
        // Deliberately omit 'space'
        let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
        blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
        blob_map.insert("size".to_string(), Promised::String("4".to_string()));
        args.insert("blob".to_string(), Promised::Map(blob_map));

        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_error(&request, "MISSING_ARGUMENT").await);
    }

    // Test 5: Missing 'blob' argument
    {
        let mut args: BTreeMap<String, Promised> = BTreeMap::new();
        args.insert(
            "space".to_string(),
            Promised::String(ctx.space_signer.did().to_string()),
        );
        // Deliberately omit 'blob'

        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_error(&request, "MISSING_ARGUMENT").await);
    }

    // Test 6: Invalid size (non-numeric string)
    {
        let digest = Sha256::digest(b"test");
        let mut args: BTreeMap<String, Promised> = BTreeMap::new();
        args.insert(
            "space".to_string(),
            Promised::String(ctx.space_signer.did().to_string()),
        );
        let mut blob_map: BTreeMap<String, Promised> = BTreeMap::new();
        blob_map.insert("digest".to_string(), Promised::Bytes(digest.to_vec()));
        blob_map.insert(
            "size".to_string(),
            Promised::String("not-a-number".to_string()),
        );
        args.insert("blob".to_string(), Promised::Map(blob_map));

        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_error(&request, "INVALID_ARGUMENT").await);
    }

    // Test 7: Unknown command (blob/delete)
    {
        let digest = Sha256::digest(b"test");
        let args = ctx.build_allocate_args(&digest, 4);

        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "delete".to_string()], // Unknown command!
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_error(&request, "UNKNOWN_CAPABILITY").await);
    }

    // Test 8: Invalid base64 in proofs
    {
        let digest = Sha256::digest(b"test");
        let args = ctx.build_allocate_args(&digest, 4);

        let (inv_bytes, _) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec!["not-valid-base64!!!".to_string()],
        };

        suite.record(ctx.expect_error(&request, "INVALID_BASE64").await);
    }

    suite.print_summary();

    if suite.all_passed() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some validation tests failed"))
    }
}

/// Test delegation chain scenarios
async fn cmd_test_delegation(service_url: &str, verbose: bool) -> anyhow::Result<()> {
    println!("=== Delegation Chain Tests ===\n");

    let ctx = TestContext::new(service_url).await?;
    let mut suite = TestSuite::new("Delegation Tests", verbose);

    let digest = Sha256::digest(b"delegation test content");

    // Test 1: Direct delegation (space -> operator) - should pass
    {
        let args = ctx.build_allocate_args(&digest, 25);
        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_success(&request, "direct_delegation").await);
    }

    // Test 2: Attenuated command (delegate /blob, invoke /blob/allocate) - should pass
    {
        // The default delegation is for "blob" command, which should allow "blob/allocate"
        let args = ctx.build_allocate_args(&digest, 25);
        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        suite.record(ctx.expect_success(&request, "attenuated_command").await);
    }

    // Test 3: Over-attenuated (delegate /blob/get only, invoke /blob/allocate) - should fail
    {
        // Create a delegation that only allows blob/get
        let get_only_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["blob".to_string(), "get".to_string()]) // Only get!
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let get_only_bytes = serde_ipld_dagcbor::to_vec(&get_only_delegation)?;
        let get_only_cid = get_only_delegation.to_cid();

        let args = ctx.build_allocate_args(&digest, 25);

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()]) // But we invoke allocate!
            .arguments(args)
            .proofs(vec![get_only_cid])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&get_only_bytes)],
        };

        suite.record(ctx.expect_error(&request, "COMMAND_MISMATCH").await);
    }

    // Test 4: Expired delegation
    {
        // Create a timestamp in the past (1 hour ago)
        let past_time = SystemTime::now() - Duration::from_secs(3600);
        let past_exp = Timestamp::new(past_time).unwrap();

        let expired_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["blob".to_string()])
            .expiration(past_exp)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let expired_bytes = serde_ipld_dagcbor::to_vec(&expired_delegation)?;
        let expired_cid = expired_delegation.to_cid();

        let args = ctx.build_allocate_args(&digest, 25);

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args)
            .proofs(vec![expired_cid])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&expired_bytes)],
        };

        suite.record(ctx.expect_error(&request, "PROOF_EXPIRED").await);
    }

    // Test 5: Not-yet-valid delegation (nbf in future)
    {
        // Create a timestamp in the future (1 hour from now)
        let future_time = SystemTime::now() + Duration::from_secs(3600);
        let future_nbf = Timestamp::new(future_time).unwrap();

        let future_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["blob".to_string()])
            .not_before(future_nbf)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let future_bytes = serde_ipld_dagcbor::to_vec(&future_delegation)?;
        let future_cid = future_delegation.to_cid();

        let args = ctx.build_allocate_args(&digest, 25);

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args)
            .proofs(vec![future_cid])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![BASE64.encode(&future_bytes)],
        };

        suite.record(ctx.expect_error(&request, "PROOF_NOT_YET_VALID").await);
    }

    // Test 6: Multi-level delegation (space -> intermediate -> operator)
    {
        // Generate intermediate identity
        let mut intermediate_seed = [0u8; 32];
        getrandom::getrandom(&mut intermediate_seed)?;
        let intermediate_signer = Ed25519Signer::new(SigningKey::from_bytes(&intermediate_seed));

        // Space delegates to intermediate
        let delegation1 = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*intermediate_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["blob".to_string()])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation1: {:?}", e))?;

        let delegation1_bytes = serde_ipld_dagcbor::to_vec(&delegation1)?;
        let delegation1_cid = delegation1.to_cid();

        // Intermediate delegates to operator
        let delegation2 = DelegationBuilder::new()
            .issuer(intermediate_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["blob".to_string()])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation2: {:?}", e))?;

        let delegation2_bytes = serde_ipld_dagcbor::to_vec(&delegation2)?;
        let delegation2_cid = delegation2.to_cid();

        let args = ctx.build_allocate_args(&digest, 25);

        // Operator invokes with both proofs
        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["blob".to_string(), "allocate".to_string()])
            .arguments(args)
            .proofs(vec![delegation1_cid, delegation2_cid])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;
        let request = InvocationRequest {
            invocation: BASE64.encode(&inv_bytes),
            proofs: vec![
                BASE64.encode(&delegation1_bytes),
                BASE64.encode(&delegation2_bytes),
            ],
        };

        suite.record(ctx.expect_success(&request, "multi_level_delegation").await);
    }

    suite.print_summary();

    if suite.all_passed() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some delegation tests failed"))
    }
}

/// Test storage edge cases
async fn cmd_test_storage(service_url: &str, verbose: bool) -> anyhow::Result<()> {
    println!("=== Storage Edge Case Tests ===\n");

    let ctx = TestContext::new(service_url).await?;
    let mut suite = TestSuite::new("Storage Tests", verbose);

    // Test 1: Allocate twice (second should return size 0)
    {
        // Use unique content for this test
        let content = format!("storage-test-{}", chrono::Utc::now().timestamp_millis());
        let content_bytes = content.as_bytes();
        let digest = Sha256::digest(content_bytes);

        let args = ctx.build_allocate_args(&digest, content_bytes.len() as u64);
        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args.clone(),
            vec![ctx.delegation_cid],
        )?;

        // First allocation
        let response1 = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let body1 = response1.text().await?;

        if let Ok(result1) = serde_json::from_str::<AllocateResponse>(&body1) {
            // Upload the blob
            if let Some(address) = result1.address {
                let mut upload_req = ctx.client.put(&address.url).body(content_bytes.to_vec());
                for (k, v) in &address.headers {
                    upload_req = upload_req.header(k, v);
                }
                let _ = upload_req.send().await?;
            }

            // Second allocation (same blob)
            let response2 = ctx
                .client
                .post(&ctx.service_url)
                .json(&request)
                .send()
                .await?;
            let body2 = response2.text().await?;

            if let Ok(result2) = serde_json::from_str::<AllocateResponse>(&body2) {
                if result2.size == 0 && result2.address.is_none() {
                    suite.record(TestResult::pass("duplicate_allocate"));
                } else {
                    suite.record(TestResult::fail(
                        "duplicate_allocate",
                        format!(
                            "Expected size 0 and no address, got size {} with address {:?}",
                            result2.size,
                            result2.address.is_some()
                        ),
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "duplicate_allocate",
                    format!("Failed to parse second response: {}", body2),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "duplicate_allocate",
                format!("Failed to parse first response: {}", body1),
            ));
        }
    }

    // Test 2: Get returns valid presigned URL (even for non-existent blob)
    {
        // Request a blob that doesn't exist
        let fake_digest = Sha256::digest(b"this blob definitely does not exist");

        let mut args: BTreeMap<String, Promised> = BTreeMap::new();
        args.insert(
            "space".to_string(),
            Promised::String(ctx.space_signer.did().to_string()),
        );
        args.insert("digest".to_string(), Promised::Bytes(fake_digest.to_vec()));

        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "get".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        let response = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            if let Ok(result) = serde_json::from_str::<GetResponse>(&body) {
                if !result.url.is_empty() {
                    suite.record(TestResult::pass("get_nonexistent_returns_url"));
                } else {
                    suite.record(TestResult::fail(
                        "get_nonexistent_returns_url",
                        "Got empty URL",
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "get_nonexistent_returns_url",
                    format!("Failed to parse response: {}", body),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "get_nonexistent_returns_url",
                format!("Expected success, got {}: {}", status, body),
            ));
        }
    }

    // Test 3: Large blob allocation (test with 1GB size claim)
    {
        let digest = Sha256::digest(b"large blob test");
        let large_size: u64 = 1024 * 1024 * 1024; // 1GB

        let args = ctx.build_allocate_args(&digest, large_size);
        let (_, request) = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["blob".to_string(), "allocate".to_string()],
            args,
            vec![ctx.delegation_cid],
        )?;

        let response = ctx
            .client
            .post(&ctx.service_url)
            .json(&request)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            if let Ok(result) = serde_json::from_str::<AllocateResponse>(&body) {
                if result.size == large_size || result.size == 0 {
                    suite.record(TestResult::pass("large_blob_allocate"));
                } else {
                    suite.record(TestResult::fail(
                        "large_blob_allocate",
                        format!("Unexpected size: {}", result.size),
                    ));
                }
            } else {
                suite.record(TestResult::fail(
                    "large_blob_allocate",
                    format!("Failed to parse response: {}", body),
                ));
            }
        } else {
            suite.record(TestResult::fail(
                "large_blob_allocate",
                format!("Expected success, got {}: {}", status, body),
            ));
        }
    }

    suite.print_summary();

    if suite.all_passed() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some storage tests failed"))
    }
}

/// Run all test suites
async fn cmd_test_all(service_url: &str, verbose: bool) -> anyhow::Result<()> {
    println!("=== Running All Test Suites ===\n");

    let mut all_passed = true;
    let mut suite_results: Vec<(String, bool)> = Vec::new();

    // Run each test suite
    println!("--- Auth Tests ---");
    match cmd_test_auth(service_url, verbose).await {
        Ok(()) => suite_results.push(("Auth Tests".to_string(), true)),
        Err(_) => {
            all_passed = false;
            suite_results.push(("Auth Tests".to_string(), false));
        }
    }

    println!("\n--- Validation Tests ---");
    match cmd_test_validation(service_url, verbose).await {
        Ok(()) => suite_results.push(("Validation Tests".to_string(), true)),
        Err(_) => {
            all_passed = false;
            suite_results.push(("Validation Tests".to_string(), false));
        }
    }

    println!("\n--- Delegation Tests ---");
    match cmd_test_delegation(service_url, verbose).await {
        Ok(()) => suite_results.push(("Delegation Tests".to_string(), true)),
        Err(_) => {
            all_passed = false;
            suite_results.push(("Delegation Tests".to_string(), false));
        }
    }

    println!("\n--- Storage Tests ---");
    match cmd_test_storage(service_url, verbose).await {
        Ok(()) => suite_results.push(("Storage Tests".to_string(), true)),
        Err(_) => {
            all_passed = false;
            suite_results.push(("Storage Tests".to_string(), false));
        }
    }

    // Print summary
    println!("\n========================================");
    println!("          OVERALL TEST SUMMARY          ");
    println!("========================================\n");

    for (name, passed) in &suite_results {
        let status = if *passed { "PASS" } else { "FAIL" };
        println!("  [{}] {}", status, name);
    }

    let passed_count = suite_results.iter().filter(|(_, p)| *p).count();
    let total_count = suite_results.len();

    println!("\n  Total: {}/{} suites passed", passed_count, total_count);

    if all_passed {
        println!("\n  ALL TESTS PASSED!");
        Ok(())
    } else {
        println!("\n  SOME TESTS FAILED");
        Err(anyhow::anyhow!("Some test suites failed"))
    }
}
