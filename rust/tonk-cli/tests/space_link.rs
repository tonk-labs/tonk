//! Linking a local-only space into the signed-in account, live against an
//! account service and an access service.

mod common;

use anyhow::Result;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::inventory::SpaceRole;
use tonk_cli::site::{SiteConfig, TonkSite};
use tonk_cli::space::{AccountRecord, SpaceStore};

const OTHER_ACCOUNT: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

fn signed_in(
    fixture: &common::AccountFixture,
    env: &AccessServiceAddress,
) -> Result<AccountRecord> {
    let mut record = AccountRecord::new(fixture.link.issuer().to_string());
    record.access_remote = Some(env.access_service_url.clone());
    Ok(record)
}

async fn local_space(store: &SpaceStore, config: &SiteConfig, name: &str) -> Result<()> {
    tonk_cli::space::create(store, name, None, None, config.clone()).await?;
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

    // The space did not move: same name, same site, same subject. And the
    // registry still says nothing about who owns it.
    let entry = store.load()?.spaces["garden"].clone();
    assert_eq!(entry.site, outcome.site);
    let site = TonkSite::open_with(&entry.site, config.clone()).await?;
    assert_eq!(site.repository.did().to_string(), outcome.subject);

    // Ownership is on the space's own content branch, keyed on the account
    // root so it converges across every device on that account.
    let roster = tonk_cli::inventory::read_roster(&site).await?;
    let founder = roster.founder().expect("a founder row");
    assert_eq!(founder.did, account.root);
    assert_eq!(
        tonk_cli::inventory::role_for_site(&site).await?,
        SpaceRole::Owner
    );
    assert_eq!(
        tonk_cli::remote::upstream_remote(&site).await?.as_deref(),
        Some(tonk_cli::remote::DEFAULT_REMOTE)
    );

    let report = tonk_cli::inventory::list_local(&store, &config).await?;
    let rendered = tonk_cli::inventory::render(&report.rows);
    println!("{rendered}");
    let row = &report.rows[0];
    assert_eq!(row.owner.as_deref(), Some(account.root.as_str()));
    assert!(row.owner_is_you);
    assert_eq!(row.role, SpaceRole::Owner);
    assert!(rendered.contains("you ("), "{rendered}");

    let listed = tonk_cli::account_spaces::list(&fixture.profile, &store).await?;
    assert!(
        listed
            .iter()
            .any(|row| row.subject == outcome.subject && row.local_name.is_some()),
        "the account directory should list the linked space: {listed:?}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_links_a_signed_repository_after_its_creating_profile_is_gone(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    let store = fixture.store.clone();
    let mut recovery_config = fixture.config.clone();
    recovery_config.profile_name = format!("space-link-recovery-{}", rand::random::<u64>());
    recovery_config.require_account = false;
    recovery_config.provision_account_spaces = false;
    local_space(&store, &recovery_config, "garden").await?;

    fixture.activate_with(&env).await?;
    let account = signed_in(&fixture, &env)?;
    store.set_account(Some(account.clone()))?;

    let outcome = tonk_cli::space_link::execute(&store, &fixture.config, "garden").await?;

    assert_eq!(outcome.account, account.root);
    assert!(!outcome.already_linked);
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
    let first = tonk_cli::space_link::execute(&store, &config, "garden").await?;
    let after_first = std::fs::read(store.registry_path())?;

    let again = tonk_cli::space_link::execute(&store, &config, "garden").await?;

    assert!(again.already_linked);
    assert_eq!(again.subject, first.subject);
    assert_eq!(std::fs::read(store.registry_path())?, after_first);
    Ok(())
}

#[dialog_common::test]
async fn it_finishes_an_already_linked_space_after_an_invite_was_minted(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "garden").await?;
    store.set_account(Some(signed_in(&fixture, &env)?))?;
    let first = tonk_cli::space_link::execute(&store, &config, "garden").await?;
    let entry = store.load()?.spaces["garden"].clone();
    let site = TonkSite::open_with(&entry.site, config.clone()).await?;
    tonk_cli::invite::mint_with_relay(&site, None, None, None).await?;

    let again = tonk_cli::space_link::execute(&store, &config, "garden").await?;

    assert!(again.already_linked);
    assert_eq!(again.subject, first.subject);
    let listed = tonk_cli::account_spaces::list(&fixture.profile, &store).await?;
    assert!(
        listed
            .iter()
            .any(|row| row.subject == first.subject && row.local_name.is_some()),
        "the account directory should still list the linked space: {listed:?}"
    );
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
    let message = error.to_string();
    assert!(
        message.contains("already belongs to an account"),
        "{error:#}"
    );
    // The refusal names the owner the *space* says it has, not a registry tag.
    assert!(message.contains(&first.root), "{error:#}");

    // …and the space is still there, still open, for the account that is not
    // its owner: possession is not permission, but it is not a lock either.
    let resolved = store.resolve(Some("garden"), None, None)?;
    assert_eq!(resolved.name, "garden");
    let site = TonkSite::open_with(&resolved.site, config.clone()).await?;
    assert!(site.branch().await.is_ok());
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
    let entry = store.load()?.spaces["garden"].clone();
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
    assert!(
        tonk_cli::inventory::read_roster(&site).await?.is_empty(),
        "a refused link writes no roster"
    );
    Ok(())
}

/// The listing after the exact switch a person makes: sign in, link a space,
/// sign out, sign in as somebody else. What they had is still there, still
/// named, still theirs to edit — and the owner column, not a refusal, is what
/// says whose it is.
#[dialog_common::test]
async fn it_lists_a_previous_accounts_space_with_its_owner(
    env: AccessServiceAddress,
) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    fixture.activate_with(&env).await?;
    let store = fixture.store.clone();
    let config = fixture.config.clone();
    local_space(&store, &config, "scratch").await?;
    local_space(&store, &config, "garden").await?;
    store.set_account(Some(signed_in(&fixture, &env)?))?;
    let linked = tonk_cli::space_link::execute(&store, &config, "garden").await?;

    store.set_account(None)?;
    store.set_account(Some(AccountRecord::new(OTHER_ACCOUNT)))?;

    let report = tonk_cli::inventory::list_local(&store, &config).await?;
    let rendered = tonk_cli::inventory::render(&report.rows);
    println!("{rendered}");

    let garden = report
        .rows
        .iter()
        .find(|row| row.name == "garden")
        .expect("garden row");
    assert_eq!(garden.owner.as_deref(), Some(linked.account.as_str()));
    assert!(!garden.owner_is_you);
    let scratch = report
        .rows
        .iter()
        .find(|row| row.name == "scratch")
        .expect("scratch row");
    assert_eq!(scratch.owner, None);
    assert_eq!(scratch.role, SpaceRole::Local);
    assert!(!rendered.contains("ACCESS"), "{rendered}");
    assert!(!rendered.contains("another account"), "{rendered}");
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    Ok(())
}
