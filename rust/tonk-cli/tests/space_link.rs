//! Linking a local-only space into the signed-in account, live against an
//! account service and an access service.

mod common;

use anyhow::Result;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::inventory::SpaceRole;
use tonk_cli::site::{SiteConfig, TonkSite};
use tonk_cli::spot::{AccountRecord, SpotStore};

/// On the discard port: parsed and stored, never called.
const DEAD_RELAY: &str = "http://127.0.0.1:9/revocations";

const OTHER_ACCOUNT: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

fn signed_in(
    fixture: &common::AccountFixture,
    env: &AccessServiceAddress,
) -> Result<AccountRecord> {
    let mut record = AccountRecord::new(fixture.link.issuer().to_string());
    record.access_remote = Some(env.access_service_url.clone());
    record.revocation_relay = Some(DEAD_RELAY.to_owned());
    Ok(record)
}

async fn local_space(store: &SpotStore, config: &SiteConfig, name: &str) -> Result<()> {
    tonk_cli::spot::create(store, name, None, None, config.clone()).await?;
    Ok(())
}

#[dialog_common::test]
async fn it_links_a_local_space_into_the_signed_in_account(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    let account = signed_in(&fixture, &env)?;
    store.set_account(Some(account.clone()))?;

    let outcome = tonk_cli::space_link::execute(&store, &config, "garden").await?;

    assert!(!outcome.already_linked);
    assert_eq!(outcome.name, "garden");
    assert_eq!(outcome.account, account.root);

    // The registry says the account owns it, and the space itself did not
    // move: same name, same site, same subject.
    let entry = store.load()?.spots["garden"].clone();
    assert_eq!(entry.account.as_deref(), Some(account.root.as_str()));
    assert_eq!(entry.site, outcome.site);
    let site = TonkSite::open_with(&entry.site, config.clone()).await?;
    assert_eq!(Some(site.repository.did().to_string()), outcome.subject);

    // It is owned, synced, and listed for the account's other devices.
    assert_eq!(
        tonk_cli::inventory::role_for_site(&site).await?,
        SpaceRole::Owner
    );
    assert_eq!(
        tonk_cli::remote::upstream_remote(&site).await?.as_deref(),
        Some(tonk_cli::remote::DEFAULT_REMOTE)
    );
    let listed = tonk_cli::account_spots::list(&fixture.profile, &store).await?;
    assert!(
        listed
            .iter()
            .any(|row| Some(&row.subject) == outcome.subject.as_ref() && row.local_name.is_some()),
        "the account directory should list the linked space: {listed:?}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_reports_an_already_linked_space_without_relinking_it(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    store.set_account(Some(signed_in(&fixture, &env)?))?;
    tonk_cli::space_link::execute(&store, &config, "garden").await?;
    let after_first = std::fs::read(store.registry_path())?;

    let again = tonk_cli::space_link::execute(&store, &config, "garden").await?;

    assert!(again.already_linked);
    assert_eq!(std::fs::read(store.registry_path())?, after_first);
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_to_move_a_linked_space_to_another_account(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    let first = signed_in(&fixture, &env)?;
    store.set_account(Some(first.clone()))?;
    tonk_cli::space_link::execute(&store, &config, "garden").await?;

    // Sign out, then sign in as somebody else on the same device.
    store.set_account(None)?;
    store.set_account(Some(AccountRecord::new(OTHER_ACCOUNT)))?;

    let error = tonk_cli::space_link::execute(&store, &config, "garden")
        .await
        .expect_err("a linked space cannot change accounts");
    assert!(
        error.to_string().contains("already belongs to an account"),
        "{error:#}"
    );

    // …and the second account cannot open it either, with the copy that
    // tells someone what to do about it.
    let refused = store
        .resolve(Some("garden"), None, None)
        .expect_err("another account must not resolve this space");
    assert!(
        refused
            .to_string()
            .contains("this account doesn't have access to 'garden'"),
        "{refused}"
    );

    // Signing the owner back in restores it, untouched.
    store.set_account(Some(first.clone()))?;
    let resolved = store.resolve(Some("garden"), None, None)?;
    assert_eq!(resolved.name, "garden");
    assert_eq!(
        store.load()?.spots["garden"].account.as_deref(),
        Some(first.root.as_str())
    );
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_to_link_while_signed_out(env: AccessServiceAddress) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    let before = std::fs::read(store.registry_path())?;

    let error = tonk_cli::space_link::execute(&store, &config, "garden")
        .await
        .expect_err("linking needs an account");

    assert!(
        error.to_string().contains("no account is signed in"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(store.registry_path())?, before);
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_space_that_already_syncs_with_another_remote(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    let entry = store.load()?.spots["garden"].clone();
    let site = TonkSite::open_with(&entry.site, config.clone()).await?;
    tonk_cli::remote::add(
        &site,
        "origin",
        "http://127.0.0.1:9/other/",
        Some(site.repository.did()),
    )
    .await?;
    tonk_cli::remote::set_upstream(&site, "origin").await?;
    store.set_account(Some(signed_in(&fixture, &env)?))?;

    let error = tonk_cli::space_link::execute(&store, &config, "garden")
        .await
        .expect_err("a space with its own upstream is not local-only");

    assert!(
        error
            .to_string()
            .contains("local-only space with no content upstream"),
        "{error:#}"
    );
    assert!(store.load()?.spots["garden"].account.is_none());
    Ok(())
}
