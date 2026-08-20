//! The local space listing: one row per registered replica, with the account
//! it belongs to, the role its signed membership proves, and whether the
//! signed-in account can reach it.

mod common;

use anyhow::Result;
use tonk_cli::inventory::{SpaceRole, list_local};
use tonk_cli::site::SiteConfig;
use tonk_cli::spot::{AccountRecord, SpotStore};
use tonk_schema::{MemberRole, Membership};

const ACCOUNT_A: &str = "did:key:z6MkAccountAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ACCOUNT_B: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

/// Register a space and, when `role` is given, stamp the signed membership an
/// account-owned replica carries and tag it to `account`.
async fn add_space(
    store: &SpotStore,
    config: &SiteConfig,
    name: &str,
    account: Option<(&str, SpaceRole)>,
) -> Result<()> {
    let outcome = tonk_cli::spot::create(store, name, None, None, config.clone()).await?;
    let Some((account, role)) = account else {
        return Ok(());
    };
    let site = tonk_cli::site::TonkSite::open_with(&outcome.site, config.clone()).await?;
    let membership = Membership::new(site.profile.did(), site.repository.did());
    let stamp = match role {
        SpaceRole::Owner => MemberRole::founder(membership.this().clone()),
        SpaceRole::Member => MemberRole::member(membership.this().clone()),
        SpaceRole::Local | SpaceRole::Unknown => unreachable!("not a signed role"),
    };
    let meta = site
        .repository
        .branch(tonk_cli::remote::META_BRANCH)
        .open()
        .perform(&site.operator)
        .await?;
    meta.transaction()
        .assert(membership)
        .assert(stamp)
        .commit()
        .perform(&site.operator)
        .await?;
    store.set_space_account(name, Some(account))?;
    Ok(())
}

fn fixture(tmp: &std::path::Path) -> Result<(SpotStore, SiteConfig)> {
    let store = SpotStore::at(tmp.join("state"));
    let mut config = common::isolated_config(tmp)?;
    config.account_store = store.clone();
    Ok((store, config))
}

#[dialog_common::test]
async fn it_lists_local_owner_and_member_replicas_with_their_accounts() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;

    add_space(&store, &config, "scratch", None).await?;
    add_space(
        &store,
        &config,
        "garden",
        Some((ACCOUNT_A, SpaceRole::Owner)),
    )
    .await?;
    add_space(
        &store,
        &config,
        "shared",
        Some((ACCOUNT_A, SpaceRole::Member)),
    )
    .await?;
    add_space(
        &store,
        &config,
        "roadmap",
        Some((ACCOUNT_B, SpaceRole::Owner)),
    )
    .await?;

    let report = list_local(&store, &config).await?;
    let rows: Vec<_> = report
        .rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.account.as_deref(),
                row.role,
                row.access,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("garden", Some(ACCOUNT_A), SpaceRole::Owner, true),
            ("roadmap", Some(ACCOUNT_B), SpaceRole::Owner, false),
            ("scratch", None, SpaceRole::Local, true),
            ("shared", Some(ACCOUNT_A), SpaceRole::Member, true),
        ]
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);

    let json = serde_json::to_value(&report.rows)?;
    assert_eq!(json[0]["role"], "owner");
    assert_eq!(json[0]["account"], ACCOUNT_A);
    assert_eq!(json[0]["access"], true);
    assert_eq!(json[1]["access"], false);
    assert_eq!(json[2]["role"], "local");
    assert_eq!(json[2]["account"], serde_json::Value::Null);
    Ok(())
}

#[dialog_common::test]
async fn it_reaches_every_replica_while_no_account_is_signed_in() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    add_space(
        &store,
        &config,
        "garden",
        Some((ACCOUNT_A, SpaceRole::Owner)),
    )
    .await?;
    add_space(
        &store,
        &config,
        "roadmap",
        Some((ACCOUNT_B, SpaceRole::Owner)),
    )
    .await?;

    let report = list_local(&store, &config).await?;

    assert!(
        report.rows.iter().all(|row| row.access),
        "{:?}",
        report.rows
    );
    Ok(())
}

#[dialog_common::test]
async fn it_retains_other_rows_when_one_site_is_unreadable() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    add_space(&store, &config, "healthy", None).await?;

    let broken = store.canonical_site("broken");
    std::fs::create_dir_all(&broken)?;
    let mut registry = store.load()?;
    registry
        .spots
        .insert("broken".to_owned(), tonk_cli::spot::SpotEntry::at(broken));
    store.save(&registry)?;

    let report = list_local(&store, &config).await?;

    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].name, "healthy");
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.diagnostics[0].starts_with("broken:"));
    Ok(())
}

#[dialog_common::test]
async fn it_lists_an_account_space_with_no_signed_role_rather_than_hiding_it() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    add_space(&store, &config, "half-linked", None).await?;
    store.set_space_account("half-linked", Some(ACCOUNT_A))?;

    let report = list_local(&store, &config).await?;

    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].role, SpaceRole::Unknown);
    assert_eq!(report.diagnostics.len(), 1);
    assert!(
        report.diagnostics[0].contains("membership"),
        "{:?}",
        report.diagnostics
    );
    Ok(())
}
