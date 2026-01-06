//! Test client for the UCAN Access Service.
//!
//! This CLI tool provides utilities for:
//! - Generating Ed25519 keypairs
//! - Creating UCAN delegations
//! - Sending GET/PUT requests to the access service
//! - End-to-end testing
//! - Comprehensive edge case and failure mode testing
//!
//! Usage:
//!   cargo run --bin test-client --features test-client -- <command>

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use getrandom_0_2 as getrandom;
use ipld_core::cid::Cid;
use reqwest::{Client, redirect::Policy};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::slice::from_ref;
use std::time::{Duration, SystemTime};
use ucan::{
    Delegation,
    delegation::{builder::DelegationBuilder, subject::DelegatedSubject},
    did::{Ed25519Did, Ed25519Signer},
    invocation::builder::InvocationBuilder,
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

    /// Send a PUT request (allocate blob)
    InvokePut {
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

        /// Path to file to upload
        #[arg(long)]
        file: PathBuf,
    },

    /// Send a GET request (get blob)
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

        /// Hex-encoded blob digest (blake3 hash for addressing)
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
        // Client that doesn't follow redirects (so we can test 307)
        let client = Client::builder().redirect(Policy::none()).build()?;

        // Verify service is reachable
        let _info: ServiceInfo = client.get(service_url).send().await?.json().await?;

        // Generate test keypairs
        let mut space_seed = [0u8; 32];
        getrandom::getrandom(&mut space_seed)?;
        let space_signer = Ed25519Signer::new(SigningKey::from_bytes(&space_seed));

        let mut operator_seed = [0u8; 32];
        getrandom::getrandom(&mut operator_seed)?;
        let operator_signer = Ed25519Signer::new(SigningKey::from_bytes(&operator_seed));

        // Create delegation (space -> operator) for http command
        let delegation = DelegationBuilder::new()
            .issuer(space_signer.clone())
            .audience(*operator_signer.did())
            .subject(DelegatedSubject::Specific(*space_signer.did()))
            .command(vec!["http".to_string()])
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

    /// Build a valid invocation for http/get or http/put.
    fn build_invocation(
        &self,
        issuer: &Ed25519Signer,
        subject: Ed25519Did,
        command: Vec<String>,
        proofs: Vec<Cid>,
    ) -> anyhow::Result<Vec<u8>> {
        let invocation = InvocationBuilder::new()
            .issuer(issuer.clone())
            .audience(subject)
            .subject(subject)
            .command(command)
            .arguments(BTreeMap::new()) // No arguments needed - params come from URL
            .proofs(proofs)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        Ok(serde_ipld_dagcbor::to_vec(&invocation)?)
    }

    /// Build an invocation with custom audience (for testing audience mismatch).
    fn build_invocation_with_audience(
        &self,
        issuer: &Ed25519Signer,
        audience: Ed25519Did,
        subject: Ed25519Did,
        command: Vec<String>,
        proofs: Vec<Cid>,
    ) -> anyhow::Result<Vec<u8>> {
        let invocation = InvocationBuilder::new()
            .issuer(issuer.clone())
            .audience(audience)
            .subject(subject)
            .command(command)
            .arguments(BTreeMap::new())
            .proofs(proofs)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        Ok(serde_ipld_dagcbor::to_vec(&invocation)?)
    }

    /// Build URL for blob endpoint
    fn build_blob_url(&self, space_did: &str, digest_hex: &str) -> String {
        format!("{}/{}/index/{}", self.service_url, space_did, digest_hex)
    }

    /// Send a GET request and check for expected error
    async fn expect_get_error(
        &self,
        url: &str,
        invocation_bytes: &[u8],
        proof_bytes: &[Vec<u8>],
        expected_code: &str,
    ) -> TestResult {
        let mut request = self.client.get(url).header(
            "Authorization",
            format!("Bearer {}", BASE64.encode(invocation_bytes)),
        );

        // Only add X-UCAN-Proofs header if there are proofs
        if !proof_bytes.is_empty() {
            let proofs_header = proof_bytes
                .iter()
                .map(|p| BASE64.encode(p))
                .collect::<Vec<_>>()
                .join(",");
            request = request.header("X-UCAN-Proofs", proofs_header);
        }

        let response = match request.send().await {
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

        // Should NOT be a 307 redirect
        if status.as_u16() == 307 {
            return TestResult::fail(
                format!("expect_{}", expected_code.to_lowercase()),
                format!("Expected error {} but got 307 redirect", expected_code),
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

    /// Send a GET request and expect 307 redirect
    async fn expect_get_redirect(
        &self,
        url: &str,
        invocation_bytes: &[u8],
        proof_bytes: &[Vec<u8>],
        test_name: &str,
    ) -> TestResult {
        let proofs_header = proof_bytes
            .iter()
            .map(|p| BASE64.encode(p))
            .collect::<Vec<_>>()
            .join(",");

        let response = match self
            .client
            .get(url)
            .header(
                "Authorization",
                format!("Bearer {}", BASE64.encode(invocation_bytes)),
            )
            .header("X-UCAN-Proofs", proofs_header)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return TestResult::fail(test_name, format!("Request failed: {}", e)),
        };

        let status = response.status();

        if status.as_u16() == 307 {
            if response.headers().get("location").is_some() {
                TestResult::pass(test_name)
            } else {
                TestResult::fail(test_name, "Got 307 but missing Location header")
            }
        } else {
            let body = response.text().await.unwrap_or_default();
            TestResult::fail(
                test_name,
                format!("Expected 307 redirect, got {} {}", status, body),
            )
        }
    }
}

/// Compute both blake3 (for addressing) and sha256 (for R2 integrity).
///
/// Returns (blake3_hex, sha256_base64).
fn compute_blob_hashes(data: &[u8]) -> (String, String) {
    let blake3_hash = blake3::hash(data);
    let sha256_hash = Sha256::digest(data);
    (blake3_hash.to_hex().to_string(), BASE64.encode(sha256_hash))
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
        Commands::InvokePut {
            service_url,
            operator_key,
            space_did,
            delegation,
            file,
        } => cmd_invoke_put(&service_url, &operator_key, &space_did, &delegation, &file).await,
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

async fn cmd_invoke_put(
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
    let (blake3_hex, sha256_b64) = compute_blob_hashes(&file_content);

    // Client that follows redirects for actual upload
    let client = Client::new();

    println!("Space DID: {}", space_did_str);
    println!("File size: {} bytes", file_content.len());
    println!("Blake3 (addressing): {}", blake3_hex);
    println!("SHA256 (integrity):  {}", sha256_b64);

    // Build invocation for http/put
    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(space_did)
        .subject(space_did)
        .command(vec!["http".to_string(), "put".to_string()])
        .arguments(BTreeMap::new())
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    // Build URL using blake3 for addressing
    let url = format!("{}/{}/index/{}", service_url, space_did_str, blake3_hex);

    println!("\nSending PUT request to {}...", url);

    // First get the redirect (include sha256 checksum for R2 integrity)
    let redirect_client = Client::builder().redirect(Policy::none()).build()?;
    let response = redirect_client
        .put(&url)
        .header(
            "Authorization",
            format!("Bearer {}", BASE64.encode(&invocation_bytes)),
        )
        .header("X-UCAN-Proofs", BASE64.encode(&delegation_bytes))
        .header("X-Checksum-SHA256", &sha256_b64)
        .send()
        .await?;

    let status = response.status();
    if status.as_u16() != 307 {
        let body = response.text().await?;
        println!("Error: Expected 307 redirect, got {} - {}", status, body);
        return Err(anyhow::anyhow!("Request failed"));
    }

    let presigned_url = response
        .headers()
        .get("location")
        .ok_or_else(|| anyhow::anyhow!("Missing Location header"))?
        .to_str()?;

    // Parse required headers from response (contains x-amz-checksum-sha256)
    let required_headers: Vec<(String, String)> = response
        .headers()
        .get("x-required-headers")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    println!("Got presigned URL, uploading...");

    // Upload to presigned URL with required headers (including checksum for R2 integrity)
    let mut upload_request = client
        .put(presigned_url)
        .header("Content-Type", "application/octet-stream");

    for (name, value) in &required_headers {
        upload_request = upload_request.header(name, value);
    }

    let upload_response = upload_request.body(file_content).send().await?;

    if upload_response.status().is_success() {
        println!("Upload successful!");
    } else {
        println!(
            "Upload failed: {} - {}",
            upload_response.status(),
            upload_response.text().await?
        );
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

    let client = Client::new();

    println!("Space DID: {}", space_did_str);
    println!("Digest: {}", digest_hex);

    // Build invocation for http/get
    let invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(space_did)
        .subject(space_did)
        .command(vec!["http".to_string(), "get".to_string()])
        .arguments(BTreeMap::new())
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

    let invocation_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

    // Build URL
    let url = format!("{}/{}/index/{}", service_url, space_did_str, digest_hex);

    println!("\nSending GET request to {}...", url);

    // First get the redirect
    let redirect_client = Client::builder().redirect(Policy::none()).build()?;
    let response = redirect_client
        .get(&url)
        .header(
            "Authorization",
            format!("Bearer {}", BASE64.encode(&invocation_bytes)),
        )
        .header("X-UCAN-Proofs", BASE64.encode(&delegation_bytes))
        .send()
        .await?;

    let status = response.status();
    if status.as_u16() != 307 {
        let body = response.text().await?;
        println!("Error: Expected 307 redirect, got {} - {}", status, body);
        return Err(anyhow::anyhow!("Request failed"));
    }

    let presigned_url = response
        .headers()
        .get("location")
        .ok_or_else(|| anyhow::anyhow!("Missing Location header"))?
        .to_str()?;

    println!("Got presigned URL, downloading...");

    // Download from presigned URL
    let download_response = client.get(presigned_url).send().await?;

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
    let redirect_client = Client::builder().redirect(Policy::none()).build()?;

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
        .command(vec!["http".to_string()])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

    let delegation_bytes = serde_ipld_dagcbor::to_vec(&delegation)?;
    let delegation_cid = delegation.to_cid();
    println!("  Delegation CID: {}", delegation_cid);

    // Step 4: Prepare test content
    println!("\nStep 4: Preparing test content...");
    let content_bytes = content.as_bytes();
    let (blake3_hex, sha256_b64) = compute_blob_hashes(content_bytes);
    println!("  Content: \"{}\"", content);
    println!("  Size: {} bytes", content_bytes.len());
    println!("  Blake3 (addressing): {}", blake3_hex);
    println!("  SHA256 (integrity):  {}", sha256_b64);

    // Step 5: Send PUT request
    println!("\nStep 5: Sending PUT request...");

    let put_invocation = InvocationBuilder::new()
        .issuer(operator_signer.clone())
        .audience(*space_signer.did())
        .subject(*space_signer.did())
        .command(vec!["http".to_string(), "put".to_string()])
        .arguments(BTreeMap::new())
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build PUT invocation: {:?}", e))?;

    let put_inv_bytes = serde_ipld_dagcbor::to_vec(&put_invocation)?;

    let put_url = format!(
        "{}/{}/index/{}",
        service_url,
        space_signer.did(),
        blake3_hex
    );
    let put_response = redirect_client
        .put(&put_url)
        .header(
            "Authorization",
            format!("Bearer {}", BASE64.encode(&put_inv_bytes)),
        )
        .header("X-UCAN-Proofs", BASE64.encode(&delegation_bytes))
        .header("X-Checksum-SHA256", &sha256_b64)
        .send()
        .await?;

    let put_status = put_response.status();
    if put_status.as_u16() != 307 {
        let body = put_response.text().await?;
        println!("  FAILED: Expected 307, got {} - {}", put_status, body);
        return Err(anyhow::anyhow!("PUT failed"));
    }

    let presigned_put_url = put_response
        .headers()
        .get("location")
        .ok_or_else(|| anyhow::anyhow!("Missing Location header"))?
        .to_str()?;

    // Parse required headers (contains x-amz-checksum-sha256 for R2 integrity)
    let required_headers: Vec<(String, String)> = put_response
        .headers()
        .get("x-required-headers")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    println!("  Got presigned URL");

    // Step 6: Upload blob
    println!("\nStep 6: Uploading blob to presigned URL...");
    let mut upload_request = client
        .put(presigned_put_url)
        .header("Content-Type", "application/octet-stream");

    for (name, value) in &required_headers {
        upload_request = upload_request.header(name, value);
    }

    let upload_response = upload_request.body(content_bytes.to_vec()).send().await?;

    if upload_response.status().is_success() {
        println!("  Upload: SUCCESS");
    } else {
        let err_body = upload_response.text().await?;
        println!("  Upload: FAILED - {}", err_body);
        return Err(anyhow::anyhow!("Upload failed"));
    }

    // Step 7: Send GET request
    println!("\nStep 7: Sending GET request...");

    let get_invocation = InvocationBuilder::new()
        .issuer(operator_signer)
        .audience(*space_signer.did())
        .subject(*space_signer.did())
        .command(vec!["http".to_string(), "get".to_string()])
        .arguments(BTreeMap::new())
        .proofs(vec![delegation_cid])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build GET invocation: {:?}", e))?;

    let get_inv_bytes = serde_ipld_dagcbor::to_vec(&get_invocation)?;

    let get_url = format!(
        "{}/{}/index/{}",
        service_url,
        space_signer.did(),
        blake3_hex
    );
    let get_response = redirect_client
        .get(&get_url)
        .header(
            "Authorization",
            format!("Bearer {}", BASE64.encode(&get_inv_bytes)),
        )
        .header("X-UCAN-Proofs", BASE64.encode(&delegation_bytes))
        .send()
        .await?;

    let get_status = get_response.status();
    if get_status.as_u16() != 307 {
        let body = get_response.text().await?;
        println!("  FAILED: Expected 307, got {} - {}", get_status, body);
        return Err(anyhow::anyhow!("GET failed"));
    }

    let presigned_get_url = get_response
        .headers()
        .get("location")
        .ok_or_else(|| anyhow::anyhow!("Missing Location header"))?
        .to_str()?;
    println!("  Got presigned URL");

    // Step 8: Download and verify
    println!("\nStep 8: Downloading and verifying content...");
    let download_response = client.get(presigned_get_url).send().await?;

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
    let digest_hex = hex::encode(digest);
    let url = ctx.build_blob_url(&ctx.space_signer.did().to_string(), &digest_hex);

    // Test 1: Invalid signature (tamper with invocation bytes)
    {
        let mut inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![ctx.delegation_cid],
        )?;

        // Tamper with the bytes
        let mid = inv_bytes.len() / 2;
        if let Some(byte) = inv_bytes.get_mut(mid) {
            *byte = byte.wrapping_add(1);
        }

        let result = ctx
            .expect_get_error(
                &url,
                &inv_bytes,
                from_ref(&ctx.delegation_bytes),
                "SIGNATURE_INVALID",
            )
            .await;

        // Accept either SIGNATURE_INVALID or INVALID_CBOR since tampering may break parsing
        if result.passed {
            suite.record(result);
        } else if result.message.contains("INVALID_CBOR") {
            suite.record(TestResult::pass("tampered_signature"));
        } else {
            suite.record(result);
        }
    }

    // Test 2: Wrong audience (audience != subject)
    {
        let mut wrong_seed = [0u8; 32];
        getrandom::getrandom(&mut wrong_seed)?;
        let wrong_signer = Ed25519Signer::new(SigningKey::from_bytes(&wrong_seed));

        let inv_bytes = ctx.build_invocation_with_audience(
            &ctx.operator_signer,
            *wrong_signer.did(), // Wrong audience
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![ctx.delegation_cid],
        )?;

        suite.record(
            ctx.expect_get_error(
                &url,
                &inv_bytes,
                from_ref(&ctx.delegation_bytes),
                "AUDIENCE_MISMATCH",
            )
            .await,
        );
    }

    // Test 3: Expired invocation
    {
        let past_time = SystemTime::now() - Duration::from_secs(3600);
        let past_exp = Timestamp::new(past_time).unwrap();

        let invocation = InvocationBuilder::new()
            .issuer(ctx.operator_signer.clone())
            .audience(*ctx.space_signer.did())
            .subject(*ctx.space_signer.did())
            .command(vec!["http".to_string(), "get".to_string()])
            .arguments(BTreeMap::new())
            .proofs(vec![ctx.delegation_cid])
            .expiration(past_exp)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build invocation: {:?}", e))?;

        let inv_bytes = serde_ipld_dagcbor::to_vec(&invocation)?;

        suite.record(
            ctx.expect_get_error(
                &url,
                &inv_bytes,
                from_ref(&ctx.delegation_bytes),
                "INVOCATION_EXPIRED",
            )
            .await,
        );
    }

    // Test 4: Missing proofs (empty proofs)
    {
        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![ctx.delegation_cid],
        )?;

        suite.record(
            ctx.expect_get_error(&url, &inv_bytes, &[], "PROOF_NOT_FOUND") // Empty proofs!
                .await,
        );
    }

    // Test 5: Wrong command (http/delete instead of http/get)
    {
        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "delete".to_string()], // Wrong command!
            vec![ctx.delegation_cid],
        )?;

        suite.record(
            ctx.expect_get_error(
                &url,
                &inv_bytes,
                from_ref(&ctx.delegation_bytes),
                "COMMAND_MISMATCH",
            )
            .await,
        );
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

    let digest_hex = hex::encode(Sha256::digest(b"test"));
    let url = ctx.build_blob_url(&ctx.space_signer.did().to_string(), &digest_hex);

    // Test 1: Invalid base64 in Authorization header
    {
        let proofs_header = BASE64.encode(&ctx.delegation_bytes);

        let response = ctx
            .client
            .get(&url)
            .header("Authorization", "Bearer not-valid-base64!!!")
            .header("X-UCAN-Proofs", proofs_header)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() && status.as_u16() != 307 {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "INVALID_BASE64" {
                    suite.record(TestResult::pass("invalid_base64_auth"));
                } else {
                    suite.record(TestResult::fail(
                        "invalid_base64_auth",
                        format!("Expected INVALID_BASE64, got {}", err.error.code),
                    ));
                }
            } else {
                suite.record(TestResult::pass("invalid_base64_auth")); // Some error is fine
            }
        } else {
            suite.record(TestResult::fail(
                "invalid_base64_auth",
                "Expected error but got success or redirect",
            ));
        }
    }

    // Test 2: Valid base64 but invalid CBOR
    {
        let proofs_header = BASE64.encode(&ctx.delegation_bytes);

        let response = ctx
            .client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", BASE64.encode(b"not cbor")),
            )
            .header("X-UCAN-Proofs", proofs_header)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() && status.as_u16() != 307 {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "INVALID_CBOR" {
                    suite.record(TestResult::pass("invalid_cbor"));
                } else {
                    suite.record(TestResult::fail(
                        "invalid_cbor",
                        format!("Expected INVALID_CBOR, got {}", err.error.code),
                    ));
                }
            } else {
                suite.record(TestResult::pass("invalid_cbor"));
            }
        } else {
            suite.record(TestResult::fail(
                "invalid_cbor",
                "Expected error but got success or redirect",
            ));
        }
    }

    // Test 3: Missing Authorization header
    {
        let response = ctx
            .client
            .get(&url)
            .header("X-UCAN-Proofs", BASE64.encode(&ctx.delegation_bytes))
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() && status.as_u16() != 307 {
            suite.record(TestResult::pass("missing_auth_header"));
        } else {
            suite.record(TestResult::fail(
                "missing_auth_header",
                "Expected error for missing Authorization header",
            ));
        }
    }

    // Test 4: Invalid base64 in X-UCAN-Proofs
    {
        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![ctx.delegation_cid],
        )?;

        let response = ctx
            .client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", BASE64.encode(&inv_bytes)),
            )
            .header("X-UCAN-Proofs", "not-valid-base64!!!")
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() && status.as_u16() != 307 {
            if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
                if err.error.code == "INVALID_BASE64" {
                    suite.record(TestResult::pass("invalid_base64_proofs"));
                } else {
                    suite.record(TestResult::fail(
                        "invalid_base64_proofs",
                        format!("Expected INVALID_BASE64, got {}", err.error.code),
                    ));
                }
            } else {
                suite.record(TestResult::pass("invalid_base64_proofs"));
            }
        } else {
            suite.record(TestResult::fail(
                "invalid_base64_proofs",
                "Expected error but got success or redirect",
            ));
        }
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
    let digest_hex = hex::encode(digest);
    let url = ctx.build_blob_url(&ctx.space_signer.did().to_string(), &digest_hex);

    // Test 1: Direct delegation (space -> operator) - should pass
    {
        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![ctx.delegation_cid],
        )?;

        suite.record(
            ctx.expect_get_redirect(
                &url,
                &inv_bytes,
                from_ref(&ctx.delegation_bytes),
                "direct_delegation",
            )
            .await,
        );
    }

    // Test 2: Over-attenuated (delegate /http/get only, invoke /http/put) - should fail
    {
        let get_only_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["http".to_string(), "get".to_string()]) // Only get!
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let get_only_bytes = serde_ipld_dagcbor::to_vec(&get_only_delegation)?;
        let get_only_cid = get_only_delegation.to_cid();

        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "put".to_string()], // But we invoke put!
            vec![get_only_cid],
        )?;

        // Use PUT endpoint for this test
        suite.record(
            ctx.expect_get_error(&url, &inv_bytes, &[get_only_bytes], "COMMAND_MISMATCH")
                .await,
        );
    }

    // Test 3: Expired delegation
    {
        let past_time = SystemTime::now() - Duration::from_secs(3600);
        let past_exp = Timestamp::new(past_time).unwrap();

        let expired_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["http".to_string()])
            .expiration(past_exp)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let expired_bytes = serde_ipld_dagcbor::to_vec(&expired_delegation)?;
        let expired_cid = expired_delegation.to_cid();

        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![expired_cid],
        )?;

        suite.record(
            ctx.expect_get_error(&url, &inv_bytes, &[expired_bytes], "PROOF_EXPIRED")
                .await,
        );
    }

    // Test 4: Not-yet-valid delegation (nbf in future)
    {
        let future_time = SystemTime::now() + Duration::from_secs(3600);
        let future_nbf = Timestamp::new(future_time).unwrap();

        let future_delegation = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["http".to_string()])
            .not_before(future_nbf)
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation: {:?}", e))?;

        let future_bytes = serde_ipld_dagcbor::to_vec(&future_delegation)?;
        let future_cid = future_delegation.to_cid();

        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![future_cid],
        )?;

        suite.record(
            ctx.expect_get_error(&url, &inv_bytes, &[future_bytes], "PROOF_NOT_YET_VALID")
                .await,
        );
    }

    // Test 5: Multi-level delegation (space -> intermediate -> operator)
    {
        let mut intermediate_seed = [0u8; 32];
        getrandom::getrandom(&mut intermediate_seed)?;
        let intermediate_signer = Ed25519Signer::new(SigningKey::from_bytes(&intermediate_seed));

        // Space delegates to intermediate
        let delegation1 = DelegationBuilder::new()
            .issuer(ctx.space_signer.clone())
            .audience(*intermediate_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["http".to_string()])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation1: {:?}", e))?;

        let delegation1_bytes = serde_ipld_dagcbor::to_vec(&delegation1)?;
        let delegation1_cid = delegation1.to_cid();

        // Intermediate delegates to operator
        let delegation2 = DelegationBuilder::new()
            .issuer(intermediate_signer.clone())
            .audience(*ctx.operator_signer.did())
            .subject(DelegatedSubject::Specific(*ctx.space_signer.did()))
            .command(vec!["http".to_string()])
            .try_build()
            .map_err(|e| anyhow::anyhow!("Failed to build delegation2: {:?}", e))?;

        let delegation2_bytes = serde_ipld_dagcbor::to_vec(&delegation2)?;
        let delegation2_cid = delegation2.to_cid();

        let inv_bytes = ctx.build_invocation(
            &ctx.operator_signer,
            *ctx.space_signer.did(),
            vec!["http".to_string(), "get".to_string()],
            vec![delegation1_cid, delegation2_cid],
        )?;

        suite.record(
            ctx.expect_get_redirect(
                &url,
                &inv_bytes,
                &[delegation1_bytes, delegation2_bytes],
                "multi_level_delegation",
            )
            .await,
        );
    }

    suite.print_summary();

    if suite.all_passed() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some delegation tests failed"))
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
