//! What a device may still do after its own grant is revoked.
//!
//! Revocation cuts off storage, not the device's local facts: the
//! device list and every other local read serve what is already here.
//! That only holds if nothing on the read path waits on the remote —
//! the e2e suite caught `tonk account devices` parked in a remote
//! resolve the service never answered for a revoked principal.

mod common;

use anyhow::{Context, Result};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_schema::prelude::DidExt as _;

/// Record `artifact` with the service, refusing anything but success.
async fn publish(env: &AccessServiceAddress, artifact: Vec<u8>) -> Result<()> {
    let response = reqwest::Client::new()
        .post(env.ucan_endpoint())
        .header(reqwest::header::CONTENT_TYPE, "application/cbor")
        .body(artifact)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to reach the access service")?;
    anyhow::ensure!(
        response.status().is_success(),
        "the service refused the revocation: {}",
        response.status()
    );
    Ok(())
}

/// A revoked device's local reads answer promptly. The bound is what is
/// under test: a wrong outcome is acceptable to this assertion, a hang
/// is not.
#[dialog_common::test]
async fn it_answers_local_reads_promptly_after_this_device_is_revoked(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let operator =
        tonk_cli::account_state::credential_operator_for_store(&fixture.profile, &fixture.store)
            .await?;
    // Mount and hydrate once while still authorized, as a linked device
    // would have.
    tonk_cli::account_state::ensure_with_operator_and_store(
        &fixture.profile,
        operator.clone(),
        fixture.store.clone(),
    )
    .await?;

    // Revoke this device's own grant and record it with the service.
    let target = fixture.link.proof_cids()[0];
    let artifact = tonk_identity::revocation::mint_self_revocation(
        fixture.profile.signer().signer().clone(),
        &fixture.link,
        &target,
    )
    .await
    .context("failed to sign self-revocation")?;
    publish(&env, artifact).await?;

    // The property: a local open completes promptly. Success or a
    // prompt error are both acceptable; parking on the remote is the
    // bug.
    let opened = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tonk_cli::account_state::open_account_branch_in(
            &fixture.profile,
            &operator,
            &fixture.store,
        ),
    )
    .await;
    anyhow::ensure!(
        opened.is_ok(),
        "a revoked device's local open parked on the remote instead of answering"
    );
    Ok(())
}

/// The whole `tonk account devices` path, run on a device another
/// principal has just revoked — the exact shape the e2e suite wedged
/// on. The command may succeed from local facts or fail naming the
/// remote; what it must not do is park.
#[dialog_common::test]
async fn it_lists_devices_promptly_after_another_principal_revokes_this_one(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let operator =
        tonk_cli::account_state::credential_operator_for_store(&fixture.profile, &fixture.store)
            .await?;
    // Hydrate once while authorized, as a linked device would have.
    tonk_cli::account_state::ensure_with_operator_and_store(
        &fixture.profile,
        operator.clone(),
        fixture.store.clone(),
    )
    .await?;

    // The root withdraws this device, the way the browser panel does.
    let target = fixture.link.proof_cids()[0];
    let artifact = tonk_identity::revocation::mint_root_revocation(
        fixture.root_signer().await?,
        &fixture.link,
        &target,
    )
    .await
    .context("failed to sign the root revocation")?;
    publish(&env, artifact).await?;

    let listed = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tonk_cli::account::devices_in(&fixture.profile, &fixture.store),
    )
    .await;
    anyhow::ensure!(
        listed.is_ok(),
        "a revoked device's list parked instead of answering"
    );
    Ok(())
}

/// The e2e shape exactly: the revocation is DELEGATED — minted by
/// another device under its own powerline, the way the browser panel
/// revokes — rather than signed by the root. The list must still answer
/// within a bound afterwards.
#[dialog_common::test]
async fn it_lists_devices_promptly_after_a_delegated_revocation(
    env: AccessServiceAddress,
) -> Result<()> {
    use dialog_varsig::Principal as _;

    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let operator =
        tonk_cli::account_state::credential_operator_for_store(&fixture.profile, &fixture.store)
            .await?;
    tonk_cli::account_state::ensure_with_operator_and_store(
        &fixture.profile,
        operator.clone(),
        fixture.store.clone(),
    )
    .await?;

    // A second device of the same account: the root grants it a
    // powerline, and it revokes this one under that powerline.
    let browser = dialog_credentials::Ed25519Signer::generate().await?;
    let browser_link = tonk_identity::delegation::mint_device_delegation(
        fixture.root_signer().await?,
        &browser.did(),
    )
    .await?;
    let target = fixture.link.proof_cids()[0];
    let artifact = tonk_identity::revocation::mint_delegated_revocation(
        browser,
        &fixture.link,
        &target,
        &browser_link,
    )
    .await
    .context("failed to mint the delegated revocation")?;
    publish(&env, artifact).await?;

    let started = std::time::Instant::now();
    let listed = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tonk_cli::account::devices_in(&fixture.profile, &fixture.store),
    )
    .await;
    eprintln!(
        "devices after delegated revocation took {:?}",
        started.elapsed()
    );
    anyhow::ensure!(
        listed.is_ok(),
        "a revoked device's list parked instead of answering"
    );
    Ok(())
}

/// The e2e shape without a browser: a second device of the same account
/// pushes new commits to the account remote, and this device's next
/// `tonk account devices` pulls them, adopts the access upstream, and
/// opens the account. This is the sequence that parked with no I/O in
/// e2e: the pull moved the access head past the local archive, and the
/// proof for hydrating it waited on its own fetch.
#[dialog_common::test]
async fn it_lists_devices_promptly_after_another_device_pushed(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let this = common::AccountFixture::with_account_remote(&remote).await?;
    this.activate_with(&env).await?;
    let operator =
        tonk_cli::account_state::credential_operator_for_store(&this.profile, &this.store).await?;
    tonk_cli::account_state::ensure_with_operator_and_store(
        &this.profile,
        operator.clone(),
        this.store.clone(),
    )
    .await?;

    // A second device on the same account (fixtures share the root seed):
    // hydrate it and push a commit the first device has not seen.
    let other = common::AccountFixture::with_account_remote(&remote).await?;
    let other_operator =
        tonk_cli::account_state::credential_operator_for_store(&other.profile, &other.store)
            .await?;
    tonk_cli::account_state::ensure_with_operator_and_store(
        &other.profile,
        other_operator.clone(),
        other.store.clone(),
    )
    .await?;
    let branch = tonk_cli::account_state::open_account_branch_in(
        &other.profile,
        &other_operator,
        &other.store,
    )
    .await?
    .context("the second device mounts the account")?;
    branch
        .transaction()
        .assert(tonk_schema::AccountDisplayName::new(
            other.link.issuer().this(),
            "pushed from the other device".to_string(),
        ))
        .commit()
        .perform(&other_operator)
        .await?;
    branch.push().perform(&other_operator).await?;

    for round in 0..3 {
        let started = std::time::Instant::now();
        let listed = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            tonk_cli::account::devices_in(&this.profile, &this.store),
        )
        .await;
        eprintln!("round {round}: devices took {:?}", started.elapsed());
        anyhow::ensure!(
            listed.is_ok(),
            "listing after another device's push parked instead of answering"
        );
        // Tighter than the timeout: the read path has bounds that turn a
        // wait into a warning, and this test is about there being no wait.
        anyhow::ensure!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "listing after another device's push waited {:?} on the remote",
            started.elapsed()
        );
    }
    Ok(())
}
