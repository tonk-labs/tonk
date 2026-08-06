//! Live coverage for authorizing a spot's remote under an account.

mod common;

use anyhow::Result;
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_account::backup::space_root_site;
use tonk_cli::site::{SiteConfig, TonkSite};

/// Open sites the way a real install does, with the account boundary in
/// front of every remote fork. The shared fixture config leaves it off so
/// that tests without an account can still run.
fn account_config(fixture: &common::AccountFixture) -> SiteConfig {
    SiteConfig {
        require_account: true,
        ..fixture.config.clone()
    }
}

async fn configure_upstream(site: &TonkSite, endpoint: &str) -> Result<()> {
    tonk_cli::remote::add(site, "origin", endpoint, Some(site.repository.did())).await?;
    tonk_cli::remote::set_upstream(site, "origin").await?;
    Ok(())
}

/// Releases before the account-root prefix existed stored no such
/// credential, so upgrading left every spot they created with nothing under
/// that key. Authorization has to rebuild the prefix from the certificates
/// the profile already holds instead of reporting the spot as undelegated.
#[dialog_common::test]
async fn it_pushes_a_spot_whose_account_prefix_was_never_stored(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("upgraded"),
        account_config(&fixture),
    )
    .await?;
    let prefix_site = space_root_site(&site.repository.did(), fixture.link.issuer());
    fixture
        .profile
        .credential()
        .site(prefix_site.clone())
        .save(Vec::<u8>::new())
        .perform(&site.operator)
        .await?;
    configure_upstream(&site, &env.access_service_url).await?;

    tonk_cli::sync::push(&site).await?;

    let restored = fixture
        .profile
        .credential()
        .site(prefix_site)
        .load::<Vec<u8>>()
        .perform(&site.operator)
        .await?;
    assert!(
        !restored.is_empty(),
        "authorizing a remote must leave the recovered prefix stored"
    );
    Ok(())
}

/// A spot created before the account existed chains to the device root this
/// profile held at the time and reaches no account root at all. Linking an
/// account must adopt it rather than strand it offline.
#[dialog_common::test]
async fn it_pushes_a_spot_created_before_the_account_existed(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let path = fixture.pre_account_site.root.clone();
    let site = TonkSite::open_with(&path, account_config(&fixture)).await?;
    configure_upstream(&site, &env.access_service_url).await?;

    tonk_cli::sync::push(&site).await?;

    let prefix = tonk_cli::site::account_root_prefix(&site, fixture.link.issuer()).await?;
    assert_eq!(prefix.subject(), Some(&site.repository.did()));
    assert_eq!(prefix.audience(), fixture.link.issuer());
    Ok(())
}
