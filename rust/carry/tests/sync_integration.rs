//! Integration tests for the UCAN-S3 sync path.
//!
//! Each test receives an [`AccessServiceAddress`] provisioned by
//! `#[dialog_common::test]`, which spins up a local UCAN access service
//! (backed by an in-memory S3 server) for the duration of the test and
//! tears it down on exit.
//!
//! Run with: `cargo test -p carry --features integration-tests`

#![cfg(any(feature = "integration-tests", feature = "web-integration-tests"))]

use anyhow::Result;
use carry::site::Site;
use dialog_effects::storage::Directory;
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;
use dialog_repository::helpers::unique_name;
use futures_util::TryStreamExt;
use tonk_access_service::helpers::AccessServiceAddress;

/// Create an isolated Site with unique profile + repo storage.
async fn isolated_site(label: &str) -> Result<Site> {
    let temp_dir = tempfile::TempDir::new()?;
    let profile_location = Directory::At(
        std::env::temp_dir()
            .join(unique_name(&format!("{}-profile", label)))
            .to_string_lossy()
            .into_owned(),
    );
    let repo_location = Directory::At(
        std::env::temp_dir()
            .join(unique_name(&format!("{}-repo", label)))
            .to_string_lossy()
            .into_owned(),
    );
    let site = Site::init(temp_dir.path(), Some(profile_location), Some(repo_location)).await?;
    std::mem::forget(temp_dir);
    Ok(site)
}

/// Commit a single claim.
async fn assert_claim(site: &Site, the: &str, of: &str, is: &str) -> Result<()> {
    let entity = carry::schema::derive_entity(of)?;
    let stmt = carry::schema::make_statement(the, entity, dialog_query::Value::String(is.into()))?;
    site.branch
        .transaction()
        .assert(stmt)
        .commit()
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("commit failed: {}", e))?;
    Ok(())
}

/// Query claim values matching a `the` attribute.
async fn query_values(site: &Site, the: &str) -> Result<Vec<String>> {
    let selector = dialog_artifacts::ArtifactSelector::new().the(the.parse()?);
    let results: Vec<dialog_artifacts::Artifact> = site
        .branch
        .claims()
        .select(selector)
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .try_collect()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(results
        .into_iter()
        .filter_map(|a| match a.is {
            dialog_query::Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// Set up a site as the repo owner with a UCAN remote.
async fn setup_owner(label: &str, access_url: &str) -> Result<Site> {
    let site = isolated_site(label).await?;

    let origin = site
        .repo
        .remote("origin")
        .create(SiteAddress::Ucan(UcanAddress::new(access_url)))
        .perform(&site.operator)
        .await?;
    let remote_main = origin.branch("main").open().perform(&site.operator).await?;
    site.branch
        .set_upstream(remote_main)
        .perform(&site.operator)
        .await?;

    Ok(site)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[dialog_common::test]
async fn basic_ucan_push_pull(addr: AccessServiceAddress) -> Result<()> {
    let site = setup_owner("basic", &addr.access_service_url).await?;

    assert_claim(&site, "com.test/title", "note:1", "hello").await?;
    let push = site.branch.push().perform(&site.operator).await?;
    assert!(push.is_some(), "push should succeed");

    Ok(())
}

#[dialog_common::test]
async fn alice_invites_bob_who_pulls(addr: AccessServiceAddress) -> Result<()> {
    let alice = setup_owner("alice", &addr.access_service_url).await?;

    // Alice writes and pushes
    assert_claim(&alice, "com.test/title", "note:1", "hello from alice").await?;
    alice.branch.push().perform(&alice.operator).await?;

    // Bob creates his site, shares his DID with Alice
    let bob = isolated_site("bob").await?;
    let bob_did = bob.profile.did();

    // Alice creates a scoped invite for Bob's DID
    let invite = carry::invite_cmd::create_invite(&alice, Some(&bob_did), None).await?;
    assert!(invite.url.contains("?access="));

    // Bob joins: parse URL, save delegation, set up remote
    let decoded = tonk_invite::Invite::parse_url(&invite.url).await?;
    let remote_url = decoded
        .remote_url
        .clone()
        .expect("URL should include remote endpoint");

    bob.profile
        .save(dialog_ucan::UcanDelegation(decoded.chain.clone()))
        .perform(&bob.operator)
        .await?;

    let bob_origin = bob
        .repo
        .remote("origin")
        .create(SiteAddress::Ucan(UcanAddress::new(remote_url.as_str())))
        .subject(decoded.subject().clone())
        .perform(&bob.operator)
        .await?;
    let remote_main = bob_origin
        .branch("main")
        .open()
        .perform(&bob.operator)
        .await?;
    bob.branch
        .set_upstream(remote_main)
        .perform(&bob.operator)
        .await?;

    // Bob pulls
    let pull = bob.branch.pull().perform(&bob.operator).await?;
    assert!(pull.is_some(), "Bob should pull Alice's data");

    let values = query_values(&bob, "com.test/title").await?;
    assert_eq!(values, vec!["hello from alice"]);

    Ok(())
}

#[dialog_common::test]
async fn bidirectional_sync(addr: AccessServiceAddress) -> Result<()> {
    let alice = setup_owner("bidir-alice", &addr.access_service_url).await?;

    assert_claim(&alice, "com.test/title", "note:alice", "alice's note").await?;
    alice.branch.push().perform(&alice.operator).await?;

    // Bob joins
    let bob = isolated_site("bidir-bob").await?;
    let invite = carry::invite_cmd::create_invite(&alice, Some(&bob.profile.did()), None).await?;
    let decoded = tonk_invite::Invite::parse_url(&invite.url).await?;
    let remote_url = decoded
        .remote_url
        .clone()
        .expect("URL should include remote endpoint");

    bob.profile
        .save(dialog_ucan::UcanDelegation(decoded.chain.clone()))
        .perform(&bob.operator)
        .await?;

    let bob_origin = bob
        .repo
        .remote("origin")
        .create(SiteAddress::Ucan(UcanAddress::new(remote_url.as_str())))
        .subject(decoded.subject().clone())
        .perform(&bob.operator)
        .await?;
    let remote_main = bob_origin
        .branch("main")
        .open()
        .perform(&bob.operator)
        .await?;
    bob.branch
        .set_upstream(remote_main)
        .perform(&bob.operator)
        .await?;

    bob.branch.pull().perform(&bob.operator).await?;

    // Bob writes and pushes
    assert_claim(&bob, "com.test/title", "note:bob", "bob's note").await?;
    bob.branch.push().perform(&bob.operator).await?;

    // Alice pulls
    let alice_pull = alice.branch.pull().perform(&alice.operator).await?;
    assert!(alice_pull.is_some(), "Alice should pull Bob's changes");

    let mut values = query_values(&alice, "com.test/title").await?;
    values.sort();
    assert_eq!(values, vec!["alice's note", "bob's note"]);

    Ok(())
}
