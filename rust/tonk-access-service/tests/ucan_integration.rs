//! Integration tests for UCAN access service.
//!
//! These tests verify that UCAN-authorized archive operations work correctly
//! through the UcanAuthorizer that powers the tonk-access-service.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test ucan_integration
//! ```

#![cfg(feature = "integration-tests")]

use dialog_s3_credentials::ucan::{
    Credentials as UcanCredentials, DelegationChain, test_helpers::create_delegation,
};
use dialog_storage::StorageBackend;
use dialog_storage::capability::Principal;
use dialog_storage::s3::{Bucket, Credentials, S3};
use tonk_access_service::helpers::{AccessServiceAddress, Operator};

/// Helper to create a test delegation chain from subject to operator.
fn create_test_delegation_chain(
    subject_signer: &ucan::did::Ed25519Signer,
    operator_did: &ucan::did::Ed25519Did,
    can: &[&str],
) -> DelegationChain {
    let subject_did = subject_signer.did();
    let delegation = create_delegation(subject_signer, operator_did, subject_did, can)
        .expect("Failed to create test delegation");
    DelegationChain::new(delegation)
}

fn create_ucan_bucket(
    env: &AccessServiceAddress,
    operator: Operator,
    delegation: DelegationChain,
    store: &str,
) -> Bucket<Operator> {
    let ucan_credentials = UcanCredentials::new(env.access_service_url.clone(), delegation);
    let s3 = S3::new(Credentials::Ucan(ucan_credentials), operator.clone());
    Bucket::new(s3, operator.did().to_string(), store)
}

/// Test storage put and get operations through UCAN authorization.
///
/// This test:
/// 1. Generates an operator keypair
/// 2. Creates a self-delegation for the operator with storage permissions
/// 3. Creates UCAN credentials pointing to the test access service
/// 4. Performs storage set to store a value
/// 5. Performs storage get to retrieve and verify the value
#[dialog_common::test]
async fn it_performs_storage_get_and_set_via_ucan(env: AccessServiceAddress) -> anyhow::Result<()> {
    let operator = Operator::generate();

    let delegation =
        create_test_delegation_chain(operator.signer(), operator.signer().did(), &["storage"]);

    let mut bucket = create_ucan_bucket(&env, operator, delegation, "test-store");

    let key = b"ucan-test-key".to_vec();
    let value = b"ucan-test-value".to_vec();

    // Set the value using UCAN authorization
    bucket.set(key.clone(), value.clone()).await?;

    // Get the value back
    let retrieved = bucket.get(&key).await?;
    assert_eq!(retrieved, Some(value));

    Ok(())
}

/// Test that storage operations with different stores are isolated.
#[dialog_common::test]
async fn it_isolates_stores_via_ucan(env: AccessServiceAddress) -> anyhow::Result<()> {
    let operator = Operator::generate();

    let delegation =
        create_test_delegation_chain(operator.signer(), operator.signer().did(), &["storage"]);

    let mut bucket_a = create_ucan_bucket(&env, operator.clone(), delegation.clone(), "store-a");
    let mut bucket_b = create_ucan_bucket(&env, operator, delegation, "store-b");

    let key = b"same-key".to_vec();
    let value_a = b"value-in-store-a".to_vec();
    let value_b = b"value-in-store-b".to_vec();

    // Set different values in different stores
    bucket_a.set(key.clone(), value_a.clone()).await?;
    bucket_b.set(key.clone(), value_b.clone()).await?;

    // Verify each store has its own value
    let retrieved_a = bucket_a.get(&key).await?;
    let retrieved_b = bucket_b.get(&key).await?;

    assert_eq!(retrieved_a, Some(value_a));
    assert_eq!(retrieved_b, Some(value_b));

    Ok(())
}

/// Test that getting a non-existent key returns None.
#[dialog_common::test]
async fn it_returns_none_for_nonexistent_key_via_ucan(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    let operator = Operator::generate();

    let delegation =
        create_test_delegation_chain(operator.signer(), operator.signer().did(), &["storage"]);

    let bucket = create_ucan_bucket(&env, operator, delegation, "test-nonexistent");

    // Try to get a key that doesn't exist
    let result = bucket.get(&b"nonexistent-ucan-key".to_vec()).await?;
    assert_eq!(result, None);

    Ok(())
}
