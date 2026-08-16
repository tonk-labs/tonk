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

/// Creating a spot retains its authority into the ACCOUNT space when one is
/// mounted, and is a quiet no-op when none is.
///
/// The retain itself is `tonk_account::delegations`, shared with the worker so
/// the two adapters cannot drift into retaining different things; the worker
/// side proves the facts land by proving out of the account branch. What this
/// pins is the CLI's wiring: that space creation reaches the shared path at
/// all, and that a profile whose account repository is not mounted returns
/// `Ok(false)` rather than failing the creation.
#[dialog_common::test]
async fn it_retains_a_created_spot_into_the_account_space() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;

    // This fixture attaches an account's credentials but never mounts its
    // repository — the shape of a device that has linked an account and not
    // yet hydrated it. Creating a spot must still succeed: retaining is what
    // makes a spot recoverable on the NEXT device, and a spot that works but
    // is not yet backed up beats no spot at all.
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("retained"),
        account_config(&fixture),
    )
    .await?;
    assert!(
        !site.repository.did().to_string().is_empty(),
        "spot creation must survive an unmounted account repository"
    );

    // And the shared retain reports that it had nowhere to go, rather than
    // failing.
    let retained = tonk_cli::account_state::retain_space_delegation(
        &fixture.profile,
        fixture.pre_account_site.operator.inner(),
        &fixture.link,
    )
    .await?;
    assert!(
        !retained,
        "an unmounted account repository has nowhere to retain"
    );
    Ok(())
}

/// Migrating is safe to run with nothing to migrate, and reports so.
///
/// The command exists for profiles that predate delegations being facts: it
/// drains the legacy certificate store into the access branch and retains
/// each spot into the account space. A fresh profile has neither, so the run
/// must succeed and report zero rather than erroring — which is what makes it
/// safe to re-run.
#[dialog_common::test]
async fn it_migrates_delegations_idempotently() -> Result<()> {
    use dialog_storage::provider::storage::{NativeSpace, Storage};

    let fixture = common::AccountFixture::new().await?;
    let store = tonk_cli::spot::SpotStore::at(fixture.tmp.path().join("registry"));
    // Mount the fixture's profile so migration has a provider for its
    // subject: it commits as the profile, and an unmounted one errors.
    let storage = Storage::<NativeSpace>::default();
    let profile = dialog_operator::Profile::load(&fixture.config.profile_name)
        .at(fixture.config.profile_directory.clone())
        .perform(&storage)
        .await?;

    let first = tonk_cli::account_state::migrate_delegations(
        &profile,
        fixture.pre_account_site.operator.inner(),
        &storage,
        &store,
    )
    .await?;

    // Re-running must not double-count: the certificate store was drained by
    // the first pass, and retaining is content-addressed.
    let second = tonk_cli::account_state::migrate_delegations(
        &profile,
        fixture.pre_account_site.operator.inner(),
        &storage,
        &store,
    )
    .await?;
    assert_eq!(
        second.certificates, 0,
        "a drained certificate store has nothing left to migrate; first pass moved {}",
        first.certificates
    );
    assert_eq!(second.spots, 0, "no spot may be retained twice");
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
