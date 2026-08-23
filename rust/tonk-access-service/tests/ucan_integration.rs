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

use dialog_artifacts::{ArtifactSelector, Entity};
use dialog_operator::helpers::{test_operator_with_profile, unique_name};
use dialog_query::Attribute;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::Blob;
use dialog_repository::RepositoryExt as _;
use futures_util::{StreamExt, stream};
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
    // The gate serves a subject only while an active customer pays
    // for it; these tests are about sync, not registration.
    env.provision_subject(repo.did().as_str()).await?;

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

    // Metering rides the same path: every permit the push and pull drew
    // landed one invocation row in ingest.
    let ingest: serde_json::Value = reqwest::get(format!(
        "{}/_test/ingest",
        env.access_service_url.trim_end_matches('/')
    ))
    .await?
    .json()
    .await?;
    assert!(
        ingest["invocations"].as_u64().unwrap_or(0) > 0,
        "presigned operations must be recorded: {ingest}"
    );

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
    // Bob syncs Alice's repository, so it is Alice's subject that must
    // be paid for; Bob's own repo is never pushed.
    env.provision_subject(alice_repo.did().as_str()).await?;

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

/// Test that blob Import/Read authorization flows through the UCAN access
/// service unchanged: Alice writes a blob, references it from a fact, and
/// pushes (shipping the bytes through the access service to S3). A second
/// replica of the same repository — Bob, holding a delegation for Alice's
/// repo subject — pulls the revision and lazily hydrates the bytes it never
/// wrote, reading them back through the same `/ucan/` route.
#[dialog_common::test]
async fn it_syncs_blobs_via_ucan(env: AccessServiceAddress) -> anyhow::Result<()> {
    // --- Alice: create repo, delegate ownership, wire the UCAN remote. ---
    let (operator, profile) = test_operator_with_profile().await;

    let repo = profile
        .repository(unique_name("ucan-blob"))
        .create()
        .perform(&operator)
        .await?;

    let ownership = repo
        .access()
        .claim(&repo)
        .delegate(profile.did())
        .perform(&operator)
        .await?;
    profile.access().save(ownership).perform(&operator).await?;
    // The gate serves a subject only while an active customer pays
    // for it; these tests are about sync, not registration.
    env.provision_subject(repo.did().as_str()).await?;

    let address = UcanAddress::new(&env.access_service_url);
    let origin = repo
        .remote("origin")
        .create(address.clone())
        .perform(&operator)
        .await?;

    let branch = repo.branch("main").open().perform(&operator).await?;
    let upstream = origin.branch("main").open().perform(&operator).await?;
    branch.set_upstream(upstream).perform(&operator).await?;

    // Write a blob and reference it from a fact.
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 241) as u8).collect();
    let chunks: Vec<Result<Vec<u8>, _>> = payload.chunks(16384).map(|c| Ok(c.to_vec())).collect();
    let blob_entity = Blob::import(stream::iter(chunks))
        .write(branch.blobs())
        .perform(&operator)
        .await?;
    branch
        .transaction()
        .assert(Name::of(blob_entity.clone()).is("vacation.png".to_string()))
        .commit()
        .perform(&operator)
        .await?;

    // Push ships the blob through the access service to S3.
    assert!(
        branch.push().perform(&operator).await?.is_some(),
        "push should ship the blob and succeed"
    );

    // --- Bob: a second replica of the same repository. Alice delegates repo
    //     access to Bob, who opens his own repo pointing at Alice's subject. ---
    let (operator_b, profile_b) = test_operator_with_profile().await;

    let invite = profile
        .access()
        .claim(&repo)
        .delegate(profile_b.did())
        .perform(&operator)
        .await?;
    profile_b.access().save(invite).perform(&operator_b).await?;

    let repo_b = profile_b
        .repository(unique_name("ucan-blob-b"))
        .open()
        .perform(&operator_b)
        .await?;

    let origin_b = repo_b
        .remote("origin")
        .create(address)
        .subject(repo.did())
        .perform(&operator_b)
        .await?;

    let branch_b = repo_b.branch("main").open().perform(&operator_b).await?;
    let upstream_b = origin_b.branch("main").open().perform(&operator_b).await?;
    branch_b
        .set_upstream(upstream_b)
        .perform(&operator_b)
        .await?;

    // Bob pulls the revision, then lazily hydrates the bytes he never wrote.
    assert!(
        branch_b.pull().perform(&operator_b).await?.is_some(),
        "Bob should pull Alice's revision"
    );

    let size_b = Blob::from(blob_entity.clone())
        .size(branch_b.blobs())
        .perform(&operator_b)
        .await?;
    assert_eq!(size_b, Some(payload.len() as u64));

    let mut reader = Blob::from(blob_entity)
        .read(branch_b.blobs())
        .perform(&operator_b)
        .await?;
    let mut out = Vec::new();
    while let Some(chunk) = reader.next().await? {
        out.extend(chunk);
    }
    assert_eq!(
        out, payload,
        "hydrated blob bytes should match the original"
    );

    Ok(())
}

/// Every `POST /ucan/` carries a non-safelisted `Content-Type`, so the
/// browser preflights it. The preflight must say how long it stays
/// good, or a cold load re-issues `OPTIONS` between batches of fetches.
#[dialog_common::test]
async fn it_caches_the_ucan_preflight(env: AccessServiceAddress) -> anyhow::Result<()> {
    let response = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/ucan/", env.access_service_url),
        )
        .send()
        .await?;

    assert_eq!(response.status(), 204);
    assert_eq!(
        response
            .headers()
            .get("Access-Control-Max-Age")
            .and_then(|value| value.to_str().ok()),
        Some("86400"),
        "the preflight must carry a cache lifetime",
    );

    Ok(())
}

/// A refused presign answers with the reason itself, not prose.
///
/// The chain walk knows exactly which question failed, and the worker
/// has always answered with that. The native server rendered every
/// denial into `Authorization failed: {e}`, which no client can parse,
/// so a caller talking to it could not tell an expired proof from a
/// revoked one — the distinction its retry logic turns on.
///
/// Expiry is the case used here because it is the one a client is
/// expected to act on: fetch a fresh delegation rather than give up.
#[dialog_common::test]
async fn it_answers_a_refusal_with_its_typed_reason(
    env: AccessServiceAddress,
) -> anyhow::Result<()> {
    use dialog_credentials::{Ed25519Signer, Signer};
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::time::Timestamp;
    use dialog_ucan_core::{Container, DelegationBuilder, InvocationBuilder};
    use dialog_varsig::Principal as _;

    let space = Ed25519Signer::generate().await.expect("space key");
    let device = Ed25519Signer::generate().await.expect("device key");
    env.provision_subject(space.did().as_str()).await?;

    let expired_at =
        Timestamp::new(std::time::SystemTime::now() - std::time::Duration::from_secs(3_600))
            .expect("a representable timestamp");
    let grant = DelegationBuilder::new()
        .issuer(Signer::from(space.clone()))
        .audience(&device.did())
        .subject(Subject::Specific(space.did()))
        .command(vec![])
        .expiration(expired_at)
        .try_build()
        .await
        .expect("a delegation");

    let invocation = InvocationBuilder::new()
        .issuer(Signer::from(device.clone()))
        .audience(&space.did())
        .subject(&space.did())
        .command(vec!["archive".to_string(), "get".to_string()])
        .arguments(std::collections::BTreeMap::new())
        .proofs(vec![grant.to_cid()])
        .try_build()
        .await
        .expect("an invocation");

    let container = Container::new(vec![
        serde_ipld_dagcbor::to_vec(&invocation).expect("the invocation encodes"),
        grant.encoded().to_vec(),
    ])
    .into_bytes()
    .expect("a container");

    let response = reqwest::Client::new()
        .post(env.ucan_endpoint())
        .header("Content-Type", "application/cbor")
        .body(container)
        .send()
        .await?;

    assert_eq!(
        response.status().as_u16(),
        401,
        "an expired proof is an authentication-shaped refusal, not a bad request"
    );
    let body: serde_json::Value = response.json().await?;
    assert_eq!(
        body["kind"], "Expired",
        "the refusal must name itself so a client can act on it; body was: {body}"
    );
    assert_eq!(
        body["expiration"].as_u64(),
        Some(expired_at.to_unix()),
        "and carry the bound that lapsed"
    );

    Ok(())
}
