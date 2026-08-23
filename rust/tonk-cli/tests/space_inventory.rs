//! The local space listing: one row per registered replica, with the owner
//! and role read from the roster the space itself carries.

mod common;

use anyhow::Result;
use tonk_cli::inventory::{SpaceRole, list_local, render};
use tonk_cli::site::{SiteConfig, TonkSite};
use tonk_cli::spot::{AccountRecord, SpotStore};
use tonk_schema::{MemberName, MemberRole, Membership};

const ACCOUNT_A: &str = "did:key:z6MkAccountAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ACCOUNT_B: &str = "did:key:z6MkAccountBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

/// One roster row to write onto a space's content branch.
struct Row<'a> {
    member: &'a str,
    role: &'a str,
    name: Option<&'a str>,
}

/// Register a space and give it the roster `rows` describes. An empty
/// `rows` leaves it local-only: no roster at all.
async fn add_space(
    store: &SpotStore,
    config: &SiteConfig,
    name: &str,
    rows: &[Row<'_>],
) -> Result<TonkSite> {
    let outcome = tonk_cli::spot::create(store, name, None, None, config.clone()).await?;
    let site = TonkSite::open_with(&outcome.site, config.clone()).await?;
    if rows.is_empty() {
        return Ok(site);
    }
    let session = site.branch().await?;
    let mut transaction = session.handle().transaction();
    for row in rows {
        let membership = Membership::new(row.member.parse()?, site.repository.did());
        let stamp = if row.role == MemberRole::FOUNDER {
            MemberRole::founder(membership.this().clone())
        } else {
            MemberRole::member(membership.this().clone())
        };
        transaction = transaction.assert(membership.clone()).assert(stamp);
        if let Some(name) = row.name {
            transaction =
                transaction.assert(MemberName::new(membership.this().clone(), name.to_owned()));
        }
    }
    transaction.commit().perform(&site.operator).await?;
    Ok(site)
}

fn fixture(tmp: &std::path::Path) -> Result<(SpotStore, SiteConfig)> {
    let store = SpotStore::at(tmp.join("state"));
    let mut config = common::isolated_config(tmp)?;
    config.account_store = store.clone();
    Ok((store, config))
}

/// The device DID a replica's own profile signs with — the fallback the
/// listing matches on when no account is signed in.
async fn device_did(site: &TonkSite) -> String {
    site.profile.did().to_string()
}

#[dialog_common::test]
async fn it_reads_owner_and_role_from_each_space_s_own_roster() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;

    add_space(&store, &config, "scratch", &[]).await?;
    add_space(
        &store,
        &config,
        "garden",
        &[Row {
            member: ACCOUNT_A,
            role: MemberRole::FOUNDER,
            name: None,
        }],
    )
    .await?;
    add_space(
        &store,
        &config,
        "roadmap",
        &[
            Row {
                member: ACCOUNT_B,
                role: MemberRole::FOUNDER,
                name: Some("Ada Lovelace"),
            },
            Row {
                member: ACCOUNT_A,
                role: MemberRole::MEMBER,
                name: None,
            },
        ],
    )
    .await?;

    let report = list_local(&store, &config).await?;
    let rows: Vec<_> = report
        .rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.owner.as_deref(),
                row.owner_name.as_deref(),
                row.owner_is_you,
                row.role,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("garden", Some(ACCOUNT_A), None, true, SpaceRole::Owner),
            (
                "roadmap",
                Some(ACCOUNT_B),
                Some("Ada Lovelace"),
                false,
                SpaceRole::Member
            ),
            ("scratch", None, None, false, SpaceRole::Local),
        ]
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);

    let json = serde_json::to_value(&report.rows)?;
    assert_eq!(json[0]["role"], "owner");
    assert_eq!(json[0]["owner"], ACCOUNT_A);
    assert_eq!(json[1]["ownerName"], "Ada Lovelace");
    assert_eq!(json[2]["role"], "local");
    assert_eq!(json[2]["owner"], serde_json::Value::Null);
    // No per-space account tag survives anywhere, including the JSON.
    assert!(json[0].get("access").is_none(), "{json}");
    assert!(json[0].get("account").is_none(), "{json}");
    Ok(())
}

/// The listing is a listing, not an access check: a space owned by another
/// account is listed with its owner and no refusal, and signing out changes
/// only whether the owner reads as `you`.
#[dialog_common::test]
async fn it_lists_another_accounts_space_without_marking_it_out_of_reach() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    store.set_account(Some(AccountRecord::new(ACCOUNT_A)))?;
    add_space(
        &store,
        &config,
        "roadmap",
        &[Row {
            member: ACCOUNT_B,
            role: MemberRole::FOUNDER,
            name: None,
        }],
    )
    .await?;

    let report = list_local(&store, &config).await?;
    assert_eq!(report.rows[0].owner.as_deref(), Some(ACCOUNT_B));
    assert!(!report.rows[0].owner_is_you);
    assert_eq!(report.rows[0].role, SpaceRole::Unlisted);
    let rendered = render(&report.rows);
    assert!(!rendered.contains("another account"), "{rendered}");
    assert!(!rendered.contains("ACCESS"), "{rendered}");

    store.set_account(None)?;
    let signed_out = list_local(&store, &config).await?;
    assert_eq!(signed_out.rows[0].owner.as_deref(), Some(ACCOUNT_B));
    assert!(!signed_out.rows[0].owner_is_you);
    Ok(())
}

/// Signed out, the roster row this device's own profile holds is still ours.
#[dialog_common::test]
async fn it_claims_the_device_row_when_no_account_is_signed_in() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    let site = add_space(&store, &config, "garden", &[]).await?;
    let device = device_did(&site).await;

    let session = site.branch().await?;
    let membership = Membership::new(device.parse()?, site.repository.did());
    session
        .handle()
        .transaction()
        .assert(membership.clone())
        .assert(MemberRole::founder(membership.this().clone()))
        .commit()
        .perform(&site.operator)
        .await?;

    let report = list_local(&store, &config).await?;
    let row = report
        .rows
        .iter()
        .find(|row| row.name == "garden")
        .expect("garden row");
    assert_eq!(row.role, SpaceRole::Owner);
    assert_eq!(row.owner.as_deref(), Some(device.as_str()));
    Ok(())
}

#[dialog_common::test]
async fn it_retains_other_rows_when_one_site_is_unreadable() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let (store, config) = fixture(tmp.path())?;
    add_space(&store, &config, "healthy", &[]).await?;

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

mod rendering {
    use super::*;
    use tonk_cli::inventory::LocalSpaceInventoryRowV1;

    fn row(
        name: &str,
        subject: &str,
        owner: Option<&str>,
        owner_name: Option<&str>,
        owner_is_you: bool,
        role: SpaceRole,
    ) -> LocalSpaceInventoryRowV1 {
        LocalSpaceInventoryRowV1 {
            version: 1,
            name: name.to_owned(),
            subject: subject.to_owned(),
            owner: owner.map(str::to_owned),
            owner_name: owner_name.map(str::to_owned),
            owner_is_you,
            role,
            site: std::path::PathBuf::from("/tmp").join(name),
            local: true,
        }
    }

    /// Every human name is paired with an abbreviation of its stable
    /// identifier, and the signed-in account reads as `you`.
    #[test]
    fn it_pairs_every_name_with_an_abbreviated_identifier() {
        let rendered = render(&[
            row(
                "scratch",
                "did:key:z6Mkq7vpZZZZZZZZZZZZZZZZZZZZZZZZ",
                None,
                None,
                false,
                SpaceRole::Local,
            ),
            row(
                "garden",
                "did:key:z6Mk4e2bZZZZZZZZZZZZZZZZZZZZZZZZ",
                Some("did:key:z6Mkccc1ZZZZZZZZZZZZZZZZZZZZZZZZ"),
                None,
                true,
                SpaceRole::Owner,
            ),
            row(
                "roadmap",
                "did:key:z6Mkf0aaZZZZZZZZZZZZZZZZZZZZZZZZ",
                Some("did:key:z6Mkbbb9ZZZZZZZZZZZZZZZZZZZZZZZZ"),
                Some("Ada Lovelace"),
                false,
                SpaceRole::Member,
            ),
        ]);

        assert_eq!(
            rendered,
            "NAME                 OWNER                     ROLE\n\
             scratch (z6Mkq7vp)   -                         local\n\
             garden (z6Mk4e2b)    you (z6Mkccc1)            owner\n\
             roadmap (z6Mkf0aa)   Ada Lovelace (z6Mkbbb9)   member"
        );
    }

    /// Git's short-hash discipline: the abbreviation lengthens for the whole
    /// listing when any two identifiers in it share the default prefix.
    #[test]
    fn it_lengthens_an_ambiguous_abbreviation() {
        let rendered = render(&[
            row(
                "one",
                "did:key:z6MkSharedAAAA",
                None,
                None,
                false,
                SpaceRole::Local,
            ),
            row(
                "two",
                "did:key:z6MkSharedBBBB",
                None,
                None,
                false,
                SpaceRole::Local,
            ),
        ]);

        assert!(rendered.contains("one (z6MkSharedA)"), "{rendered}");
        assert!(rendered.contains("two (z6MkSharedB)"), "{rendered}");
    }

    /// A roster that names nobody we are renders as `-`, and a roster that
    /// could not be read renders as `unknown` — both listed, neither hidden.
    #[test]
    fn it_shows_an_unclaimed_or_unreadable_role_rather_than_hiding_the_row() {
        let rendered = render(&[
            row(
                "outside",
                "did:key:z6Mkaaa1ZZZZ",
                Some("did:key:z6Mkbbb2ZZZZ"),
                None,
                false,
                SpaceRole::Unlisted,
            ),
            row(
                "broken",
                "did:key:z6Mkccc3ZZZZ",
                None,
                None,
                false,
                SpaceRole::Unknown,
            ),
        ]);

        assert!(
            rendered.contains("outside (z6Mkaaa1)   z6Mkbbb2   -"),
            "{rendered}"
        );
        assert!(
            rendered.contains("broken (z6Mkccc3)    -          unknown"),
            "{rendered}"
        );
    }
}
