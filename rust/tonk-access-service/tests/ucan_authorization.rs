//! Tests for UCAN authorization logic.
//!
//! Verifies that the `UcanAuthorizer` (the core of tonk-access-service)
//! correctly authorizes UCAN invocations for storage, memory, and archive
//! operations, and rejects invalid or unauthorized requests.

use dialog_capability::Principal;
use dialog_credentials::Ed25519Signer;
use dialog_remote_s3::{Address, S3Authorization, s3::S3Credential};
use dialog_remote_ucan_s3::UcanAuthorizer;
use dialog_ucan_core::promise::Promised;
use dialog_ucan_core::subject::Subject as DelegatedSubject;
use dialog_ucan_core::{DelegationBuilder, InvocationBuilder, InvocationChain};
use dialog_varsig::eddsa::Ed25519Signature;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

async fn signer(seed: u8) -> Ed25519Signer {
    Ed25519Signer::import(&[seed; 32]).await.unwrap()
}

fn authorizer() -> UcanAuthorizer {
    let address = Address::builder("https://s3.us-east-1.amazonaws.com")
        .region("us-east-1")
        .bucket("test-bucket")
        .build()
        .unwrap();
    let authorization =
        S3Authorization::from(S3Credential::new("test-access-key", "test-secret-key"));
    UcanAuthorizer::new(address, authorization)
}

/// SHA-256 multihash: [0x12, 0x20, ...32 zero bytes]
fn dummy_checksum() -> Vec<u8> {
    let mut bytes = vec![0x12, 0x20];
    bytes.extend_from_slice(&[0u8; 32]);
    bytes
}

async fn build_container(
    subject: &Ed25519Signer,
    operator: &Ed25519Signer,
    command: Vec<String>,
    args: BTreeMap<String, Promised>,
) -> Vec<u8> {
    let subject_did = subject.did();

    let delegation = DelegationBuilder::<Ed25519Signature>::new()
        .issuer(subject.clone())
        .audience(operator)
        .subject(DelegatedSubject::Specific(subject_did.clone()))
        .command(command.clone())
        .try_build()
        .await
        .unwrap();

    let delegation_cid = delegation.to_cid();

    let invocation = InvocationBuilder::<Ed25519Signature>::new()
        .issuer(operator.clone())
        .audience(&subject_did)
        .subject(&subject_did)
        .command(command)
        .arguments(args)
        .proofs(vec![delegation_cid])
        .try_build()
        .await
        .unwrap();

    let mut delegations = HashMap::new();
    delegations.insert(delegation_cid, Arc::new(delegation));

    InvocationChain::new(invocation, delegations)
        .to_bytes()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_resolve() {
    let subject = signer(42).await;
    let operator = signer(1).await;

    let mut args = BTreeMap::new();
    args.insert("space".into(), Promised::String("test-space".into()));
    args.insert("cell".into(), Promised::String("test-cell".into()));

    let container = build_container(
        &subject,
        &operator,
        vec!["memory".into(), "resolve".into()],
        args,
    )
    .await;

    let permit = authorizer().authorize(&container).await.unwrap();
    assert_eq!(permit.method, "GET");
}

#[tokio::test]
async fn memory_publish() {
    let subject = signer(42).await;
    let operator = signer(1).await;

    let mut args = BTreeMap::new();
    args.insert("space".into(), Promised::String("test-space".into()));
    args.insert("cell".into(), Promised::String("test-cell".into()));
    args.insert("checksum".into(), Promised::Bytes(dummy_checksum()));

    let container = build_container(
        &subject,
        &operator,
        vec!["memory".into(), "publish".into()],
        args,
    )
    .await;

    let permit = authorizer().authorize(&container).await.unwrap();
    assert_eq!(permit.method, "PUT");
}

// ---------------------------------------------------------------------------
// Archive
// ---------------------------------------------------------------------------

#[tokio::test]
async fn archive_get() {
    let subject = signer(42).await;
    let operator = signer(1).await;

    let mut args = BTreeMap::new();
    args.insert("catalog".into(), Promised::String("blobs".into()));
    args.insert("digest".into(), Promised::Bytes([0u8; 32].to_vec()));

    let container = build_container(
        &subject,
        &operator,
        vec!["archive".into(), "get".into()],
        args,
    )
    .await;

    let permit = authorizer().authorize(&container).await.unwrap();
    assert_eq!(permit.method, "GET");
}

#[tokio::test]
async fn archive_put() {
    let subject = signer(42).await;
    let operator = signer(1).await;

    let mut args = BTreeMap::new();
    args.insert("catalog".into(), Promised::String("blobs".into()));
    args.insert("digest".into(), Promised::Bytes([0u8; 32].to_vec()));
    args.insert("checksum".into(), Promised::Bytes(dummy_checksum()));

    let container = build_container(
        &subject,
        &operator,
        vec!["archive".into(), "put".into()],
        args,
    )
    .await;

    let permit = authorizer().authorize(&container).await.unwrap();
    assert_eq!(permit.method, "PUT");
}

// ---------------------------------------------------------------------------
// Self-invocation (subject == operator)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn self_invocation() {
    let s = signer(42).await;

    let mut args = BTreeMap::new();
    args.insert("catalog".into(), Promised::String("blobs".into()));
    args.insert("digest".into(), Promised::Bytes([0u8; 32].to_vec()));

    let container = build_container(&s, &s, vec!["archive".into(), "get".into()], args).await;

    authorizer().authorize(&container).await.unwrap();
}

// ---------------------------------------------------------------------------
// Rejection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reject_invalid_container() {
    let result = authorizer().authorize(b"garbage").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn reject_unknown_command() {
    let subject = signer(42).await;
    let operator = signer(1).await;

    let container = build_container(
        &subject,
        &operator,
        vec!["bogus".into(), "command".into()],
        BTreeMap::new(),
    )
    .await;

    let result = authorizer().authorize(&container).await;
    assert!(result.is_err());
}
