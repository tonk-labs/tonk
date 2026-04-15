//! Integration tests for UCAN access service.
//!
//! These tests verify that the tonk-access-service correctly authorizes
//! UCAN-delegated push/pull operations by spinning up the access service
//! backed by a local S3 server, then exercising it through dialog-repository.
//!
//! Run with:
//! ```bash
//! cargo test -p tonk-access-service --features integration-tests --test ucan_integration
//! ```

#![cfg(feature = "integration-tests")]

use dialog_query::Attribute;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::helpers::{test_operator_with_profile, unique_name};
use dialog_repository::{ArtifactSelector, Entity, RepositoryExt as _};
use futures_util::StreamExt;
use tonk_access_service::helpers::AccessServiceAddress;

/// A simple typed attribute for testing.
#[derive(Attribute, Clone)]
#[domain("user")]
struct Name(String);

/// Test that a profile can create a repo, push data through the UCAN access
/// service, and then pull it back.
#[dialog_common::test]
async fn it_pushes_and_pulls_via_ucan(env: AccessServiceAddress) -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;

    let repo = profile
        .repository(unique_name("ucan-push-pull"))
        .create()
        .perform(&operator)
        .await?;

    // Delegate repo ownership to the profile
    let ownership = repo
        .access()
        .claim(&repo)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;

    // Set up UCAN remote pointing at our access service
    let address = UcanAddress::new(&env.access_service_url);
    let origin = repo
        .remote("origin")
        .create(address)
        .perform(&operator)
        .await?;

    let branch = repo.branch("main").open().perform(&operator).await?;
    let upstream = origin.branch("main").open().perform(&operator).await?;
    branch.set_upstream(upstream).perform(&operator).await?;

    // Commit and push
    let alice: Entity = "user:alice".parse()?;
    branch
        .transaction()
        .assert(Name::of(alice).is("Alice".to_string()))
        .commit()
        .perform(&operator)
        .await?;

    let push_result = branch.push().perform(&operator).await?;
    assert!(push_result.is_some(), "push should succeed");

    // Pull should find no new changes (just pushed)
    let pull_result = branch.pull().perform(&operator).await?;
    assert!(pull_result.is_none(), "pull after push should return None");

    // Verify data survives round-trip via raw artifact query
    let results: Vec<_> = branch
        .claims()
        .select(ArtifactSelector::new().the("user/name".parse()?))
        .perform(&operator)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(results.len(), 1);

    Ok(())
}

/// Test that two profiles can collaborate via UCAN delegation through
/// the access service: Alice pushes, delegates to Bob, Bob pulls and pushes,
/// Alice pulls Bob's changes.
#[dialog_common::test]
async fn it_collaborates_via_ucan_delegation(env: AccessServiceAddress) -> anyhow::Result<()> {
    let (alice_op, alice_profile) = test_operator_with_profile().await;

    // Alice creates repo and delegates ownership to her profile
    let alice_repo = alice_profile
        .repository(unique_name("collab-alice"))
        .create()
        .perform(&alice_op)
        .await?;

    let ownership = alice_repo
        .access()
        .claim(&alice_repo)
        .delegate(alice_profile.did())
        .perform(&alice_op)
        .await?;
    alice_profile
        .access()
        .save(ownership)
        .perform(&alice_op)
        .await?;

    // Alice sets up UCAN remote
    let address = UcanAddress::new(&env.access_service_url);
    let alice_origin = alice_repo
        .remote("origin")
        .create(address.clone())
        .perform(&alice_op)
        .await?;

    let alice_branch = alice_repo.branch("main").open().perform(&alice_op).await?;
    let upstream = alice_origin
        .branch("main")
        .open()
        .perform(&alice_op)
        .await?;
    alice_branch
        .set_upstream(upstream)
        .perform(&alice_op)
        .await?;

    // Alice commits and pushes
    let alice_entity: Entity = "user:alice".parse()?;
    alice_branch
        .transaction()
        .assert(Name::of(alice_entity).is("Alice".to_string()))
        .commit()
        .perform(&alice_op)
        .await?;
    alice_branch.push().perform(&alice_op).await?;

    // Bob creates his own profile and operator
    let (bob_op, bob_profile) = test_operator_with_profile().await;

    // Alice delegates repo access to Bob
    let invite = alice_profile
        .access()
        .claim(&alice_repo)
        .delegate(bob_profile.did())
        .perform(&alice_op)
        .await?;

    bob_profile.access().save(invite).perform(&bob_op).await?;

    // Bob creates his own repo pointing at Alice's remote subject
    let bob_repo = bob_profile
        .repository(unique_name("collab-bob"))
        .open()
        .perform(&bob_op)
        .await?;

    let bob_origin = bob_repo
        .remote("origin")
        .create(address)
        .subject(alice_repo.did())
        .perform(&bob_op)
        .await?;

    let bob_branch = bob_repo.branch("main").open().perform(&bob_op).await?;
    let remote_branch = bob_origin.branch("main").open().perform(&bob_op).await?;
    bob_branch
        .set_upstream(remote_branch)
        .perform(&bob_op)
        .await?;

    // Bob pulls Alice's data
    let pull_result = bob_branch.pull().perform(&bob_op).await?;
    assert!(pull_result.is_some(), "Bob should pull Alice's data");

    // Bob commits his own change and pushes
    let bob_entity: Entity = "user:bob".parse()?;
    bob_branch
        .transaction()
        .assert(Name::of(bob_entity).is("Bob".to_string()))
        .commit()
        .perform(&bob_op)
        .await?;
    bob_branch.push().perform(&bob_op).await?;

    // Alice pulls Bob's changes
    let alice_pull = alice_branch.pull().perform(&alice_op).await?;
    assert!(alice_pull.is_some(), "Alice should pull Bob's changes");

    // Alice should have both artifacts
    let alice_results: Vec<_> = alice_branch
        .claims()
        .select(ArtifactSelector::new().the("user/name".parse()?))
        .perform(&alice_op)
        .await?
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        alice_results.len(),
        2,
        "Alice should have both artifacts after pulling"
    );

    Ok(())
}
