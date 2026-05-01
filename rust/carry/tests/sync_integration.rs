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
    let entity = tonk_schema::runtime::derive_entity(of)?;
    let stmt =
        tonk_schema::runtime::make_statement(the, entity, dialog_query::Value::String(is.into()))?;
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

// ---------------------------------------------------------------------------
// `carry join` (step 5b) — exercise join_cmd::execute end-to-end so we
// catch regressions in the verifier-credentialed local-replica flow,
// the renewal detection, and the wrong-subject error path.
// ---------------------------------------------------------------------------

/// Resolve a unique `Directory::At(...)` under the platform temp dir.
/// Mirrors `isolated_site`'s naming convention.
fn unique_at(label: &str) -> Directory {
    Directory::At(
        std::env::temp_dir()
            .join(unique_name(label))
            .to_string_lossy()
            .into_owned(),
    )
}

#[dialog_common::test]
async fn carry_join_creates_replica_keyed_to_invited_subject(
    addr: AccessServiceAddress,
) -> Result<()> {
    let alice = setup_owner("join-alice", &addr.access_service_url).await?;
    assert_claim(&alice, "com.test/title", "note:1", "alice writes").await?;
    alice.branch.push().perform(&alice.operator).await?;

    // Mint a scoped invite addressed to bob's profile DID. Bob's
    // profile is created in advance so we have a stable DID to
    // delegate to; he discards the .carry/ that `Site::init`
    // mints and joins fresh from a separate parent dir, which
    // exercises `Site::init_from_invite`.
    let bob_profile_location = unique_at("join-bob-profile");
    let bob_setup_dir = tempfile::TempDir::new()?;
    let bob_setup = Site::init(
        bob_setup_dir.path(),
        Some(bob_profile_location.clone()),
        Some(unique_at("join-bob-setup-repo")),
    )
    .await?;
    let bob_did = bob_setup.profile.did();
    drop(bob_setup);
    drop(bob_setup_dir);

    let invite = carry::invite_cmd::create_invite(&alice, Some(&bob_did), None).await?;

    // Bob joins from a fresh parent directory — no existing
    // `.carry/`, so `join_cmd::execute` must create one keyed to
    // alice's repo DID via `Site::init_from_invite`.
    let bob_join_parent = tempfile::TempDir::new()?;
    carry::join_cmd::execute(
        Some(&invite.url),
        Some(bob_join_parent.path()),
        Some(bob_profile_location.clone()),
    )
    .await?;

    // Re-resolve the joined site to inspect post-conditions.
    let joined = Site::resolve(
        Some(bob_join_parent.path()),
        Some(bob_profile_location.clone()),
    )
    .await?;

    assert_eq!(
        joined.repo.did().to_string(),
        alice.repo.did().to_string(),
        "joined repo DID should equal the invited subject's DID",
    );

    // Bob's profile-meta must contain alice's replica (filtered
    // through `list_cmd::list`, which strips the self-replica).
    let spaces = carry::list_cmd::list(Some(bob_profile_location.clone())).await?;
    assert!(
        spaces
            .iter()
            .any(|r| r.subject.0.to_string() == alice.repo.did().to_string()),
        "list should surface the joined replica",
    );

    // And the pull during join should have brought alice's claim
    // through. Re-run pull for good measure (idempotent) and
    // verify the data landed.
    joined.branch.pull().perform(&joined.operator).await?;
    let values = query_values(&joined, "com.test/title").await?;
    assert_eq!(values, vec!["alice writes"]);

    drop(bob_join_parent);
    Ok(())
}

#[dialog_common::test]
async fn carry_join_renews_when_subject_matches(addr: AccessServiceAddress) -> Result<()> {
    let alice = setup_owner("renew-alice", &addr.access_service_url).await?;
    alice.branch.push().perform(&alice.operator).await?;

    let bob_profile_location = unique_at("renew-bob-profile");
    let bob_setup_dir = tempfile::TempDir::new()?;
    let bob_setup = Site::init(
        bob_setup_dir.path(),
        Some(bob_profile_location.clone()),
        Some(unique_at("renew-bob-setup-repo")),
    )
    .await?;
    let bob_did = bob_setup.profile.did();
    drop(bob_setup);
    drop(bob_setup_dir);

    let invite_one = carry::invite_cmd::create_invite(&alice, Some(&bob_did), None).await?;

    // First join — fresh.
    let bob_join_parent = tempfile::TempDir::new()?;
    carry::join_cmd::execute(
        Some(&invite_one.url),
        Some(bob_join_parent.path()),
        Some(bob_profile_location.clone()),
    )
    .await?;

    // Second join into the same `.carry/` with a fresh invite for
    // the same subject — should be detected as renewal and not
    // error out. The remote is already wired so the second join
    // must skip remote add (which would otherwise fail because
    // `origin` exists).
    let invite_two = carry::invite_cmd::create_invite(&alice, Some(&bob_did), None).await?;
    carry::join_cmd::execute(
        Some(&invite_two.url),
        Some(bob_join_parent.path()),
        Some(bob_profile_location.clone()),
    )
    .await?;

    drop(bob_join_parent);
    Ok(())
}

#[dialog_common::test]
async fn carry_join_rejects_unrelated_carry_dir(addr: AccessServiceAddress) -> Result<()> {
    let alice = setup_owner("reject-alice", &addr.access_service_url).await?;
    alice.branch.push().perform(&alice.operator).await?;

    let bob_profile_location = unique_at("reject-bob-profile");

    // Bob has an unrelated, freshly-init'd `.carry/` (signer
    // credential, fresh DID — not alice's subject).
    let bob_existing_dir = tempfile::TempDir::new()?;
    let bob_existing = Site::init(
        bob_existing_dir.path(),
        Some(bob_profile_location.clone()),
        Some(unique_at("reject-bob-existing-repo")),
    )
    .await?;
    let bob_did = bob_existing.profile.did();
    drop(bob_existing);

    // Mint an invite for bob, then attempt to join inside the
    // existing unrelated `.carry/`. join_cmd should refuse —
    // silently overlaying alice's subject onto bob's pre-existing
    // repo would corrupt the storage and meta-branch shape.
    let invite = carry::invite_cmd::create_invite(&alice, Some(&bob_did), None).await?;
    let result = carry::join_cmd::execute(
        Some(&invite.url),
        Some(bob_existing_dir.path()),
        Some(bob_profile_location.clone()),
    )
    .await;

    let err = result.expect_err("join into unrelated .carry/ should error");
    let msg = format!("{err}");
    assert!(
        msg.contains("for a different space"),
        "unexpected error message: {msg}",
    );

    drop(bob_existing_dir);
    Ok(())
}
