//! Integration tests for the UCAN-S3 sync path.
//!
//! Spins up a local UCAN access service (backed by an in-memory S3 server)
//! and exercises the full carry invite/join/push/pull flow between two
//! isolated sites.

use anyhow::{Context, Result};
use carry::identity_cmd::ProfileLocation;
use carry::site::{RepoLocation, Site};
use dialog_remote_ucan_s3::UcanAddress;
use dialog_repository::SiteAddress;
use dialog_repository::helpers::unique_location;
use futures_util::TryStreamExt;

/// Create an isolated Site with unique profile + repo storage.
async fn isolated_site(label: &str) -> Result<Site> {
    let temp_dir = tempfile::TempDir::new()?;
    let profile_location: ProfileLocation = unique_location(&format!("{}-profile", label));
    let repo_location: RepoLocation = unique_location(&format!("{}-repo", label));
    let site = Site::init(temp_dir.path(), Some(profile_location), Some(repo_location)).await?;
    std::mem::forget(temp_dir);
    Ok(site)
}

/// Start a local UCAN access service backed by an in-memory S3 server.
async fn start_access_service() -> Result<(
    tonk_access_service::helpers::AccessServiceAddress,
    tonk_access_service::helpers::AccessServer,
)> {
    tonk_access_service::helpers::access_service(Default::default())
        .await
        .context("Failed to start local UCAN access service")
}

/// Commit a single claim using carry's schema helpers.
async fn assert_claim(site: &Site, the: &str, of: &str, is: &str) -> Result<()> {
    let entity = carry::schema::derive_entity(of)?;
    let stmt =
        carry::schema::make_statement(the, entity, dialog_repository::Value::String(is.into()))?;
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
    let selector = dialog_repository::ArtifactSelector::new().the(the.parse()?);
    let results: Vec<dialog_repository::Artifact> = site
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
            dialog_repository::Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// Set up a site as the repo owner with a UCAN remote. Site::init
/// already does the repo->profile delegation, so we just add the remote.
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

/// Join a site using an invite token (mirrors `carry join` logic).
async fn join_with_token(label: &str, token: &str) -> Result<Site> {
    use dialog_capability::access::{Permit, Save};
    use dialog_capability::storage::Storage as StorageCap;
    use dialog_capability::{Policy, Subject};
    use dialog_capability_ucan::Ucan;
    use dialog_repository::helpers::unique_location;
    use dialog_repository::profile::access::Access;
    use dialog_storage::provider::Address;

    let site = isolated_site(label).await?;
    let decoded = carry::invite_cmd::decode_token(token)?;
    let ra = decoded
        .remote
        .as_ref()
        .expect("v3 token should contain remote address");

    // Import credential (must be a signer)
    let cred_export =
        dialog_credentials::credential::export::CredentialExport::try_from(decoded.cred_bytes)?;
    let credential = dialog_credentials::credential::Credential::import(cred_export).await?;
    let membership_signer = match credential {
        dialog_credentials::credential::Credential::Signer(s) => s,
        _ => anyhow::bail!("expected signer"),
    };

    // 0. Mount membership DID at a temp location so we can save chains
    let membership_did = dialog_varsig::Principal::did(&membership_signer);
    let membership_loc = unique_location("membership");
    let mount_addr = {
        use dialog_capability::storage::Location;
        Location::of(&membership_loc).address().clone()
    };
    StorageCap::mount::<Address>(membership_did.clone(), mount_addr)
        .perform(&site.operator)
        .await
        .map_err(|e| anyhow::anyhow!("mount failed: {:?}", e))?;

    // 1. Save token chain under membership DID
    Subject::from(membership_did)
        .attenuate(Permit)
        .invoke(Save::<Ucan>::new(decoded.chain))
        .perform(&site.operator)
        .await?;

    // 2. Membership re-delegates to our profile (claim against REMOTE subject)
    let remote_subject: dialog_capability::Capability<dialog_capability::Subject> =
        Subject::from(ra.subject().clone()).into();
    let extended = Access::new(&membership_signer)
        .claim(remote_subject)
        .delegate(dialog_varsig::Principal::did(&site.profile))
        .perform(&site.operator)
        .await?;

    // 3. Save extended chain under our profile
    site.profile
        .access()
        .save(extended)
        .perform(&site.operator)
        .await?;

    // Set up remote from the token
    let bob_origin = site
        .repo
        .remote("origin")
        .create(ra.site().clone())
        .subject(ra.subject().clone())
        .perform(&site.operator)
        .await?;
    let remote_main = bob_origin
        .branch("main")
        .open()
        .perform(&site.operator)
        .await?;
    site.branch
        .set_upstream(remote_main)
        .perform(&site.operator)
        .await?;

    Ok(site)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn basic_ucan_push_pull() -> Result<()> {
    let (access_addr, _server) = start_access_service().await?;
    let site = setup_owner("basic", &access_addr.access_service_url).await?;

    assert_claim(&site, "com.test/title", "note:1", "hello").await?;
    let push = site.branch.push().perform(&site.operator).await?;
    assert!(push.is_some(), "push should succeed");

    Ok(())
}

#[tokio::test]
async fn alice_pushes_bob_joins_and_pulls() -> Result<()> {
    let (access_addr, _server) = start_access_service().await?;
    let alice = setup_owner("alice", &access_addr.access_service_url).await?;

    // Alice writes and pushes
    assert_claim(&alice, "com.test/title", "note:1", "hello from alice").await?;
    let push = alice.branch.push().perform(&alice.operator).await?;
    assert!(push.is_some(), "Alice's push should succeed");

    // Alice generates invite
    let token = carry::invite_cmd::create_token(&alice).await?;
    assert!(token.starts_with("carry_inv3_"), "should be v3 token");

    // Bob joins and pulls
    let bob = join_with_token("bob", &token).await?;
    let pull = bob.branch.pull().perform(&bob.operator).await?;
    assert!(pull.is_some(), "Bob's pull should find Alice's data");

    let values = query_values(&bob, "com.test/title").await?;
    assert_eq!(values, vec!["hello from alice"]);

    Ok(())
}

#[tokio::test]
async fn bidirectional_sync_via_invite() -> Result<()> {
    let (access_addr, _server) = start_access_service().await?;
    let alice = setup_owner("bidir-alice", &access_addr.access_service_url).await?;

    assert_claim(&alice, "com.test/title", "note:alice", "alice's note").await?;
    alice.branch.push().perform(&alice.operator).await?;

    // Bob joins and pulls
    let token = carry::invite_cmd::create_token(&alice).await?;
    let bob = join_with_token("bidir-bob", &token).await?;
    bob.branch.pull().perform(&bob.operator).await?;

    // Bob writes and pushes
    assert_claim(&bob, "com.test/title", "note:bob", "bob's note").await?;
    bob.branch.push().perform(&bob.operator).await?;

    // Alice pulls Bob's changes
    let alice_pull = alice.branch.pull().perform(&alice.operator).await?;
    assert!(alice_pull.is_some(), "Alice should pull Bob's changes");

    let mut values = query_values(&alice, "com.test/title").await?;
    values.sort();
    assert_eq!(values, vec!["alice's note", "bob's note"]);

    Ok(())
}
