//! Account-spots coverage against the account directory — the plain
//! facts in the account DB that replaced the spot-backup escrow.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use dialog_query::{Output as _, Query, Term};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::account_spots;
use tonk_cli::site::TonkSite;
use tonk_cli::spot::SpotEntry;
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

#[tokio::test]
async fn list_reports_named_unnamed_and_pullable_rows() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let named_subject = fixture
        .record_directory_space(81, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let unnamed_subject = fixture.record_directory_space(82, None, None).await?;

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 2);
    let named = rows
        .iter()
        .find(|row| row.subject == named_subject.to_string())
        .unwrap();
    assert_eq!(named.remote_name.as_deref(), Some("garden"));
    assert!(named.pullable);
    assert!(named.local_name.is_none());
    let unnamed = rows
        .iter()
        .find(|row| row.subject == unnamed_subject.to_string())
        .unwrap();
    assert!(unnamed.remote_name.is_none());
    assert!(!unnamed.pullable);
    Ok(())
}

#[tokio::test]
async fn list_and_pull_choose_the_first_alias_for_a_local_subject() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("aliased-site"),
        fixture.config.clone(),
    )
    .await?;
    configure_upstream(&site, "http://127.0.0.1:9/ucan/").await?;
    mark_content_confirmed(&site, fixture.link.issuer(), "alpha").await?;
    let canonical = site.root.canonicalize()?;
    let mut registry = fixture.store.load()?;
    for name in ["zeta", "alpha"] {
        registry.spots.insert(
            name.to_string(),
            SpotEntry {
                site: canonical.clone(),
            },
        );
    }
    fixture.store.save(&registry)?;
    assert_eq!(
        account_spots::record_site_in("alpha", &site, &fixture.store).await?,
        account_spots::RecordOutcome::Recorded
    );

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_name.as_deref(), Some("alpha"));

    let outcome = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        site.repository.did().as_ref(),
        None,
    )
    .await?;
    assert!(outcome.already_local);
    assert_eq!(outcome.name, "alpha");
    assert_eq!(outcome.site, canonical);
    Ok(())
}

#[tokio::test]
async fn pull_requires_an_explicit_name_before_local_mutation() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let unnamed_subject = fixture
        .record_directory_space(83, None, Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        unnamed_subject.as_ref(),
        None,
    )
    .await
    .expect_err("nameless directory rows require --name");
    assert!(error.to_string().contains("pass --name"), "{error:#}");
    assert!(!fixture.store.canonical_site("garden").exists());
    assert!(fixture.store.load()?.spots.is_empty());

    let invalid_subject = fixture
        .record_directory_space(84, Some("My Garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        invalid_subject.as_ref(),
        None,
    )
    .await
    .expect_err("UI labels are not slugified");
    assert!(error.to_string().contains("pass --name"), "{error:#}");

    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        invalid_subject.as_ref(),
        Some("Bad Name"),
    )
    .await
    .expect_err("explicit names are validated");
    assert!(error.to_string().contains("pass --name"), "{error:#}");

    let occupied = fixture.store.canonical_site("occupied");
    let mut registry = fixture.store.load()?;
    registry.spots.insert(
        "occupied".to_string(),
        SpotEntry {
            site: fixture.tmp.path().join("broken-local-entry"),
        },
    );
    fixture.store.save(&registry)?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        invalid_subject.as_ref(),
        Some("occupied"),
    )
    .await
    .expect_err("occupied explicit names are not overwritten");
    assert!(error.to_string().contains("pass --name"), "{error:#}");

    let colliding_subject = fixture
        .record_directory_space(89, Some("occupied"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        colliding_subject.as_ref(),
        None,
    )
    .await
    .expect_err("an occupied stored name requires an explicit alternative");
    assert!(error.to_string().contains("pass --name"), "{error:#}");
    assert!(!occupied.exists());
    Ok(())
}

#[tokio::test]
async fn pull_preserves_suppression_and_cleans_up_when_initial_sync_is_offline() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let subject = fixture
        .record_directory_space(85, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let mut registry = fixture.store.load()?;
    registry.suppress(subject.as_ref());
    fixture.store.save(&registry)?;

    let error = account_spots::pull(&fixture.profile, &fixture.store, subject.as_ref(), None)
        .await
        .expect_err("an unreachable initial sync must not register a partial space");
    assert!(
        error
            .to_string()
            .contains("initial pull from 'origin' failed"),
        "{error:#}"
    );
    let registry = fixture.store.load()?;
    assert!(!registry.spots.contains_key("garden"));
    assert!(registry.is_suppressed(subject.as_ref()));
    assert!(registry.bindings.is_empty());
    assert!(!fixture.store.canonical_site("garden").exists());
    Ok(())
}

#[tokio::test]
async fn pull_requires_a_directory_row_and_returns_an_adopted_site() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;

    let adopted_root = fixture.tmp.path().join("adopted-site");
    let adopted = TonkSite::init_at_with(&adopted_root, fixture.config.clone()).await?;
    let adopted_subject = adopted.repository.did();
    configure_upstream(&adopted, "http://127.0.0.1:9/ucan/").await?;
    let mut registry = fixture.store.load()?;
    registry.spots.insert(
        "adopted".to_string(),
        SpotEntry {
            site: adopted.root.clone(),
        },
    );
    fixture.store.save(&registry)?;
    assert_eq!(
        account_spots::record_site_in("adopted", &adopted, &fixture.store).await?,
        account_spots::RecordOutcome::Recorded
    );

    let outcome = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        adopted_subject.as_ref(),
        None,
    )
    .await?;
    assert!(outcome.already_local);
    assert_eq!(outcome.name, "adopted");
    assert_eq!(outcome.site, adopted.root);
    assert_ne!(outcome.site, fixture.store.canonical_site("adopted"));

    let local_root = fixture.tmp.path().join("unlisted-local-site");
    let local = TonkSite::init_at_with(&local_root, fixture.config.clone()).await?;
    let mut registry = fixture.store.load()?;
    registry.spots.insert(
        "unlisted".to_string(),
        SpotEntry {
            site: local.root.clone(),
        },
    );
    fixture.store.save(&registry)?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        local.repository.did().as_ref(),
        None,
    )
    .await
    .expect_err("an unlisted local subject is not an account spot");
    assert!(error.to_string().contains("no mount record"), "{error:#}");
    Ok(())
}

#[dialog_common::test]
async fn pull_from_a_live_access_service_syncs_the_canonical_unbound_site(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let source_root = fixture.tmp.path().join("published-source");
    let source = TonkSite::init_at_with(&source_root, fixture.config.clone()).await?;
    let subject = source.repository.did();
    let source_branch = source.branch().await?;
    source_branch
        .handle()
        .transaction()
        .assert(RepositoryName {
            this: subject.this(),
            name: tonk_schema::domain::repo::Name("garden".to_string()),
        })
        .commit()
        .perform(&source.operator)
        .await?;
    tonk_cli::remote::add(
        &source,
        "origin",
        &env.access_service_url,
        Some(subject.clone()),
    )
    .await?;
    tonk_cli::remote::set_upstream(&source, "origin").await?;
    tonk_cli::sync::push(&source).await?;
    let published_tree = source
        .branch()
        .await?
        .handle()
        .revision()
        .expect("published source has content")
        .tree;

    // The real record path: the site's own upstream configuration lands
    // in the account directory, exactly as `tonk remote add` would have
    // written it.
    assert_eq!(
        account_spots::record_site_in("garden", &source, &fixture.store).await?,
        account_spots::RecordOutcome::Recorded
    );

    let outcome =
        account_spots::pull(&fixture.profile, &fixture.store, subject.as_ref(), None).await?;
    assert!(outcome.warning.is_none(), "{:?}", outcome.warning);
    assert_eq!(
        outcome.site,
        fixture.store.canonical_site("garden").canonicalize()?
    );
    let pulled = TonkSite::open_with(&outcome.site, fixture.config.clone()).await?;
    assert_eq!(pulled.repository.did(), subject);
    assert_eq!(
        pulled
            .branch()
            .await?
            .handle()
            .revision()
            .expect("pull imports published content")
            .tree,
        published_tree
    );
    let names: Vec<RepositoryName> = pulled
        .branch()
        .await?
        .handle()
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(subject.this()),
            name: Term::var("name"),
        })
        .perform(&pulled.operator)
        .try_vec()
        .await?;
    assert_eq!(names[0].name.0, "garden");
    let origin = tonk_cli::remote::find(&pulled, "origin")
        .await?
        .expect("origin is configured");
    assert_eq!(origin.subject, subject);
    assert_eq!(origin.endpoint, env.access_service_url);
    assert_eq!(
        tonk_cli::remote::upstream_remote(&pulled).await?.as_deref(),
        Some("origin")
    );
    let registry = fixture.store.load()?;
    assert_eq!(registry.spots.len(), 1);
    assert_eq!(registry.spots["garden"].site, outcome.site);
    assert!(registry.bindings.is_empty());
    Ok(())
}

async fn name_repository(site: &TonkSite, name: &str) -> Result<()> {
    let subject = site.repository.did();
    site.branch()
        .await?
        .handle()
        .transaction()
        .assert(RepositoryName {
            this: subject.this(),
            name: tonk_schema::domain::repo::Name(name.to_string()),
        })
        .commit()
        .perform(&site.operator)
        .await?;
    Ok(())
}

async fn configure_upstream(site: &TonkSite, endpoint: &str) -> Result<()> {
    tonk_cli::remote::add(site, "origin", endpoint, Some(site.repository.did())).await?;
    tonk_cli::remote::set_upstream(site, "origin").await?;
    Ok(())
}

async fn mark_content_confirmed(
    site: &TonkSite,
    account_root: &dialog_varsig::Did,
    name: &str,
) -> Result<()> {
    if !tonk_cli::account_sync::record_current_revision_confirmed(site, account_root).await? {
        name_repository(site, name).await?;
        assert!(
            tonk_cli::account_sync::record_current_revision_confirmed(site, account_root).await?,
            "the seeded fixture site must have a local revision"
        );
    }
    Ok(())
}

fn directory_bytes(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
        let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, files)?;
            } else if path.is_file() {
                files.insert(
                    path.strip_prefix(root)?.to_string_lossy().into_owned(),
                    std::fs::read(path)?,
                );
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

#[dialog_common::test]
async fn archive_preserves_local_data_and_authority(env: AccessServiceAddress) -> Result<()> {
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::with_account_remote(&remote).await?;
    let store = fixture.config.account_store.clone();
    let local_root = store.canonical_site("garden");
    let site = TonkSite::init_at_with(&local_root, fixture.config.clone()).await?;
    name_repository(&site, "garden").await?;
    configure_upstream(&site, &remote).await?;
    tonk_cli::spot::register_existing_unbound(&store, "garden", &site.root)?;

    let operator = fixture.pre_account_site.operator.inner();
    let account =
        tonk_cli::account_state::open_account_branch_in(&fixture.profile, operator, &store)
            .await?
            .expect("the linked fixture account is hydrated");
    let subject = site.repository.did();
    let prefix = tonk_cli::site::account_root_prefix(&site, fixture.link.issuer()).await?;
    tonk_schema::account::record_active_account_space(
        &account,
        tonk_schema::account::AccountSpaceInput {
            account: fixture.link.issuer().clone(),
            subject: subject.clone(),
            name: Some("garden".to_string()),
            remote_url: Some(remote),
            revocation_url: None,
            confirmed_revision: None,
        },
        operator,
    )
    .await?;
    account.push().perform(operator).await?;
    assert_eq!(
        account_spots::record_site_in("garden", &site, &store).await?,
        account_spots::RecordOutcome::Recorded
    );

    let revision_before = site.branch().await?.handle().revision();
    let bytes_before = directory_bytes(&site.root)?;
    let registry_before = store.load()?;
    let authority_before = prefix.to_bytes()?;

    let archived = account_spots::archive_with_operator_for_integration_test(
        &fixture.profile,
        &store,
        subject.as_ref(),
        operator,
    )
    .await?;

    assert!(archived.newly_archived);
    assert!(archived.projection_warning.is_none(), "{archived:?}");
    let updated_account =
        tonk_cli::account_state::open_account_branch_in(&fixture.profile, operator, &store)
            .await?
            .expect("the account remains hydrated after archive");
    let rows = tonk_schema::account::list_account_spaces(&updated_account, operator).await?;
    assert!(
        rows.iter()
            .any(|row| row.subject == subject && row.archived),
        "the canonical account fact remains queryable"
    );
    assert_eq!(store.load()?, registry_before);
    assert_eq!(site.branch().await?.handle().revision(), revision_before);
    assert_eq!(directory_bytes(&site.root)?, bytes_before);
    assert_eq!(
        tonk_cli::site::load_account_root_prefix_for(
            &site.profile,
            site.operator.inner(),
            &subject,
            fixture.link.issuer(),
        )
        .await?
        .to_bytes()?,
        authority_before,
        "archive must not revoke repository authority"
    );
    let pull_error = account_spots::pull(&fixture.profile, &store, subject.as_ref(), None)
        .await
        .expect_err("an archived account space must not be pullable");
    assert!(
        pull_error.to_string().contains("is archived"),
        "{pull_error:#}"
    );

    let again = account_spots::archive_with_operator_for_integration_test(
        &fixture.profile,
        &store,
        subject.as_ref(),
        operator,
    )
    .await?;
    assert!(!again.newly_archived);
    assert_eq!(directory_bytes(&site.root)?, bytes_before);
    Ok(())
}

#[tokio::test]
async fn record_lists_owned_joined_and_newly_remote_sites() -> Result<()> {
    use tonk_cli::account_spots::RecordOutcome;

    let fixture = common::AccountFixture::new().await?;
    let dead_remote = "http://127.0.0.1:9/ucan/";

    let owned = TonkSite::init_at_with(
        &fixture.store.canonical_site("owned-alias"),
        fixture.config.clone(),
    )
    .await?;
    name_repository(&owned, "synced-owned").await?;
    configure_upstream(&owned, dead_remote).await?;
    mark_content_confirmed(&owned, fixture.link.issuer(), "synced-owned").await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "owned-alias", &owned.root)?;
    assert_eq!(
        account_spots::record_site_in("owned-alias", &owned, &fixture.store).await?,
        RecordOutcome::Recorded
    );
    assert_eq!(
        account_spots::record_site_in("owned-alias", &owned, &fixture.store).await?,
        RecordOutcome::Unchanged,
        "an unchanged configuration must not commit to the account"
    );

    let (_, joined_chain) = fixture.space_chain(90).await?;
    let joined = tonk_cli::site::mount_delegated_at(
        &fixture.store.canonical_site("joined-alias"),
        joined_chain,
        fixture.config.clone(),
    )
    .await?;
    configure_upstream(&joined, dead_remote).await?;
    mark_content_confirmed(&joined, fixture.link.issuer(), "joined-alias").await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "joined-alias", &joined.root)?;
    assert_eq!(
        account_spots::record_site_in("joined-alias", &joined, &fixture.store).await?,
        RecordOutcome::Recorded
    );

    let local_only = TonkSite::init_at_with(
        &fixture.store.canonical_site("local-fallback"),
        fixture.config.clone(),
    )
    .await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "local-fallback", &local_only.root)?;
    assert_eq!(
        account_spots::record_site_in("local-fallback", &local_only, &fixture.store).await?,
        RecordOutcome::NoUpstream
    );
    let before = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert!(
        !before
            .iter()
            .any(|row| row.subject == local_only.repository.did().to_string())
    );
    configure_upstream(&local_only, dead_remote).await?;
    assert_eq!(
        account_spots::record_site_in("local-fallback", &local_only, &fixture.store).await?,
        RecordOutcome::Recorded
    );

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .find(|row| row.subject == owned.repository.did().to_string())
            .unwrap()
            .remote_name
            .as_deref(),
        Some("synced-owned"),
        "synced RepositoryName takes precedence over the registry alias"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.subject == joined.repository.did().to_string())
            .unwrap()
            .remote_name
            .as_deref(),
        Some("joined-alias"),
        "an unnamed repository falls back to its registry name"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.subject == local_only.repository.did().to_string())
            .unwrap()
            .remote_name
            .as_deref(),
        Some("local-fallback")
    );
    assert!(rows.iter().all(|row| row.pullable));
    Ok(())
}

#[tokio::test]
async fn record_sweep_uses_the_first_alias_once() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let site = TonkSite::init_at_with(
        &fixture.tmp.path().join("sweep-aliased-site"),
        fixture.config.clone(),
    )
    .await?;
    configure_upstream(&site, "http://127.0.0.1:9/ucan/").await?;
    mark_content_confirmed(&site, fixture.link.issuer(), "alpha").await?;
    let canonical = site.root.canonicalize()?;
    let mut registry = fixture.store.load()?;
    for name in ["zeta", "alpha"] {
        registry.spots.insert(
            name.to_string(),
            SpotEntry {
                site: canonical.clone(),
            },
        );
    }
    fixture.store.save(&registry)?;

    let warnings = account_spots::record_registered(&fixture.profile, &fixture.store).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, site.repository.did().to_string());
    assert_eq!(rows[0].remote_name.as_deref(), Some("alpha"));

    let warnings = account_spots::record_registered(&fixture.profile, &fixture.store).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].remote_name.as_deref(), Some("alpha"));
    Ok(())
}

/// The regression `tonk account spots` shipped with: `account status`
/// reported "signed in: yes" while `spots` claimed no account was
/// configured, because the spots commands required a prior command to
/// have hydrated the link. They hydrate on demand now.
#[dialog_common::test]
async fn list_hydrates_a_linked_but_unhydrated_profile(env: AccessServiceAddress) -> Result<()> {
    // Descriptor remotes are canonical with a trailing slash.
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::unhydrated_with_account_remote(&remote).await?;
    let operator = fixture.operator().await?;
    assert!(
        tonk_cli::account_state::open_account_branch_in(
            &fixture.profile,
            &operator,
            &fixture.store
        )
        .await?
        .is_none(),
        "the fixture must start unhydrated for this pin to mean anything"
    );

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert!(rows.is_empty());

    let operator = fixture.operator().await?;
    assert!(
        tonk_cli::account_state::open_account_branch_in(
            &fixture.profile,
            &operator,
            &fixture.store
        )
        .await?
        .is_some(),
        "listing hydrated the link"
    );
    Ok(())
}
