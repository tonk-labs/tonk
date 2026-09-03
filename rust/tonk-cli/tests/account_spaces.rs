//! Account-spaces coverage against the account directory — the plain
//! facts in the account DB that replaced the space-backup escrow.

mod common;

use anyhow::Result;
use dialog_query::{Output as _, Query, Term};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::account_spaces;
use tonk_cli::site::TonkSite;
use tonk_cli::space::SpaceEntry;
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

fn tonk_stage_entries(store: &tonk_cli::space::SpaceStore) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(store.spaces_root()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".tonk-stage-"))
        })
        .map(|entry| entry.path())
        .collect()
}

#[tokio::test]
async fn list_reports_named_unnamed_and_pullable_rows() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let named_subject = fixture
        .record_directory_space(81, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let unnamed_subject = fixture.record_directory_space(82, None, None).await?;

    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
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
    let canonical = site.root.canonicalize()?;
    let mut registry = fixture.store.load()?;
    for name in ["zeta", "alpha"] {
        registry
            .spaces
            .insert(name.to_string(), SpaceEntry::at(canonical.clone()));
    }
    fixture.store.save(&registry)?;
    assert_eq!(
        account_spaces::record_site_in("alpha", &site, &fixture.store).await?,
        account_spaces::RecordOutcome::Recorded
    );

    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_name.as_deref(), Some("alpha"));

    let outcome = account_spaces::pull(&fixture.profile, &fixture.store, "alpha", None).await?;
    assert!(outcome.already_local);
    assert_eq!(outcome.subject, site.repository.did().to_string());
    assert_eq!(outcome.name, "alpha");
    assert_eq!(outcome.site, canonical);
    Ok(())
}

#[tokio::test]
async fn pull_rejects_an_ambiguous_directory_name_with_exact_subjects() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let first = fixture
        .record_directory_space(83, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let second = fixture
        .record_directory_space(84, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;

    let error = account_spaces::pull(&fixture.profile, &fixture.store, "garden", None)
        .await
        .expect_err("a duplicate account-directory name must be disambiguated by subject");
    let message = error.to_string();
    assert!(message.contains("ambiguous"), "{error:#}");
    assert!(message.contains(first.as_ref()), "{error:#}");
    assert!(message.contains(second.as_ref()), "{error:#}");
    Ok(())
}

#[tokio::test]
async fn pull_requires_an_explicit_name_before_local_mutation() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let unnamed_subject = fixture
        .record_directory_space(83, None, Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let error = account_spaces::pull(
        &fixture.profile,
        &fixture.store,
        unnamed_subject.as_ref(),
        None,
    )
    .await
    .expect_err("nameless directory rows require --name");
    assert!(error.to_string().contains("pass --name"), "{error:#}");
    assert!(!fixture.store.canonical_site("garden").exists());
    assert!(fixture.store.load()?.spaces.is_empty());

    let invalid_subject = fixture
        .record_directory_space(84, Some("My Garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let error = account_spaces::pull(
        &fixture.profile,
        &fixture.store,
        invalid_subject.as_ref(),
        None,
    )
    .await
    .expect_err("UI labels are not slugified");
    assert!(error.to_string().contains("pass --name"), "{error:#}");

    let error = account_spaces::pull(
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
    registry.spaces.insert(
        "occupied".to_string(),
        SpaceEntry::at(fixture.tmp.path().join("broken-local-entry")),
    );
    fixture.store.save(&registry)?;
    let error = account_spaces::pull(
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
    let error = account_spaces::pull(
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
async fn pull_cleans_up_an_unverified_replica_when_initial_sync_is_offline() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let subject = fixture
        .record_directory_space(85, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;

    let error = account_spaces::pull(&fixture.profile, &fixture.store, subject.as_ref(), None)
        .await
        .expect_err("an offline initial pull cannot be verified or registered");
    assert!(error.to_string().contains("initial pull"), "{error:#}");
    assert!(!fixture.store.canonical_site("garden").exists());
    assert!(fixture.store.load()?.spaces.is_empty());
    assert!(
        tonk_stage_entries(&fixture.store).is_empty(),
        "returned errors clean their marked stage"
    );
    Ok(())
}

#[dialog_common::test]
async fn pull_does_not_publish_a_replica_that_fails_membership_validation(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    fixture.activate_with(&env).await?;
    let source_root = fixture.tmp.path().join("published-without-membership");
    let source = TonkSite::init_at_with(&source_root, fixture.config.clone()).await?;
    let subject = source.repository.did();
    env.provision_subject(subject.as_str()).await?;
    name_repository(&source, "untrusted").await?;
    tonk_cli::remote::add(
        &source,
        "origin",
        &env.access_service_url,
        Some(subject.clone()),
    )
    .await?;
    tonk_cli::remote::set_upstream(&source, "origin").await?;
    tonk_cli::sync::push(&source).await?;
    // Deliberately omit `record_founder_membership`: remote bytes alone do not
    // prove that this account is entitled to register the replica.
    assert_eq!(
        account_spaces::record_site_in("untrusted", &source, &fixture.store).await?,
        account_spaces::RecordOutcome::Recorded
    );

    let error = account_spaces::pull(&fixture.profile, &fixture.store, subject.as_ref(), None)
        .await
        .expect_err("membership validation must precede canonical publication");

    assert!(
        error
            .to_string()
            .contains("no signed membership for this account profile"),
        "{error:#}"
    );
    assert!(!fixture.store.canonical_site("untrusted").exists());
    assert!(fixture.store.load()?.spaces.is_empty());
    assert!(
        tonk_stage_entries(&fixture.store).is_empty(),
        "a failed membership check cleans only its stage"
    );
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
    registry
        .spaces
        .insert("adopted".to_string(), SpaceEntry::at(adopted.root.clone()));
    fixture.store.save(&registry)?;
    assert_eq!(
        account_spaces::record_site_in("adopted", &adopted, &fixture.store).await?,
        account_spaces::RecordOutcome::Recorded
    );

    let outcome = account_spaces::pull(
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
    registry
        .spaces
        .insert("unlisted".to_string(), SpaceEntry::at(local.root.clone()));
    fixture.store.save(&registry)?;
    let error = account_spaces::pull(
        &fixture.profile,
        &fixture.store,
        local.repository.did().as_ref(),
        None,
    )
    .await
    .expect_err("an unlisted local subject is not an account space");
    assert!(error.to_string().contains("no mount record"), "{error:#}");
    Ok(())
}

#[dialog_common::test]
async fn pull_from_a_live_access_service_syncs_the_canonical_unbound_site(
    env: AccessServiceAddress,
) -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    fixture.activate_with(&env).await?;
    let source_root = fixture.tmp.path().join("published-source");
    let source = TonkSite::init_at_with(&source_root, fixture.config.clone()).await?;
    let subject = source.repository.did();
    // The published space has to be someone's to serve before it syncs.
    env.provision_subject(subject.as_str()).await?;
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
    tonk_cli::site::record_founder_membership(&source).await?;
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
        account_spaces::record_site_in("garden", &source, &fixture.store).await?,
        account_spaces::RecordOutcome::Recorded
    );

    let outcome =
        account_spaces::pull(&fixture.profile, &fixture.store, subject.as_ref(), None).await?;
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
    assert_eq!(registry.spaces.len(), 1);
    assert_eq!(registry.spaces["garden"].site, outcome.site);
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

#[tokio::test]
async fn record_lists_owned_joined_and_newly_remote_sites() -> Result<()> {
    use tonk_cli::account_spaces::RecordOutcome;

    let fixture = common::AccountFixture::new().await?;
    let dead_remote = "http://127.0.0.1:9/ucan/";

    let owned = TonkSite::init_at_with(
        &fixture.store.canonical_site("owned-alias"),
        fixture.config.clone(),
    )
    .await?;
    name_repository(&owned, "synced-owned").await?;
    configure_upstream(&owned, dead_remote).await?;
    tonk_cli::space::register_existing_unbound(&fixture.store, "owned-alias", &owned.root)?;
    assert_eq!(
        account_spaces::record_site_in("owned-alias", &owned, &fixture.store).await?,
        RecordOutcome::Recorded
    );
    assert_eq!(
        account_spaces::record_site_in("owned-alias", &owned, &fixture.store).await?,
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
    tonk_cli::space::register_existing_unbound(&fixture.store, "joined-alias", &joined.root)?;
    assert_eq!(
        account_spaces::record_site_in("joined-alias", &joined, &fixture.store).await?,
        RecordOutcome::Recorded
    );

    let local_only = TonkSite::init_at_with(
        &fixture.store.canonical_site("local-fallback"),
        fixture.config.clone(),
    )
    .await?;
    tonk_cli::space::register_existing_unbound(&fixture.store, "local-fallback", &local_only.root)?;
    assert_eq!(
        account_spaces::record_site_in("local-fallback", &local_only, &fixture.store).await?,
        RecordOutcome::NoUpstream
    );
    let before = account_spaces::list(&fixture.profile, &fixture.store).await?;
    assert!(
        !before
            .iter()
            .any(|row| row.subject == local_only.repository.did().to_string())
    );
    configure_upstream(&local_only, dead_remote).await?;
    assert_eq!(
        account_spaces::record_site_in("local-fallback", &local_only, &fixture.store).await?,
        RecordOutcome::Recorded
    );

    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
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
    let canonical = site.root.canonicalize()?;
    let mut registry = fixture.store.load()?;
    for name in ["zeta", "alpha"] {
        registry
            .spaces
            .insert(name.to_string(), SpaceEntry::at(canonical.clone()));
    }
    fixture.store.save(&registry)?;

    let warnings = account_spaces::record_registered(&fixture.profile, &fixture.store).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, site.repository.did().to_string());
    assert_eq!(rows[0].remote_name.as_deref(), Some("alpha"));

    let warnings = account_spaces::record_registered(&fixture.profile, &fixture.store).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].remote_name.as_deref(), Some("alpha"));
    Ok(())
}

/// The regression `tonk account space` shipped with: `account status`
/// reported "signed in: yes" while the account-space listing claimed no
/// account was configured, because the command required a prior command to
/// have hydrated the link. It hydrates on demand now.
#[dialog_common::test]
async fn list_hydrates_a_linked_but_unhydrated_profile(env: AccessServiceAddress) -> Result<()> {
    // Descriptor remotes are canonical with a trailing slash.
    let remote = format!("{}/", env.access_service_url.trim_end_matches('/'));
    let fixture = common::AccountFixture::unhydrated_with_account_remote(&remote).await?;
    // Hydration syncs the account space, which is servable only once
    // its customer has confirmed the emailed activation link.
    fixture.activate_with(&env).await?;
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

    let rows = account_spaces::list(&fixture.profile, &fixture.store).await?;
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

/// The endpoints a revocation must reach: one per distinct access
/// service, not one per space.
///
/// A device grant is a powerline, so revoking a device has to be told to
/// every service that could still honour it. Several spaces usually
/// share one service, and telling it repeatedly is wasted work — while
/// missing one leaves the revoked device serving that space.
#[tokio::test]
async fn access_endpoints_are_distinct_across_spaces() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;

    // Two spaces on one service, a third on another.
    fixture
        .record_directory_space(91, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture
        .record_directory_space(92, Some("orchard"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture
        .record_directory_space(93, Some("meadow"), Some("http://127.0.0.1:10/ucan/"))
        .await?;
    // And a local-only space, which records no remote row at all.
    fixture
        .record_directory_space(94, Some("shed"), None)
        .await?;

    let account = fixture.account_branch().await?;
    let operator = fixture.operator().await?;
    let endpoints = tonk_schema::directory::access_endpoints(&account, &operator).await?;

    let found: Vec<&str> = endpoints.iter().map(String::as_str).collect();
    assert_eq!(
        found,
        vec!["http://127.0.0.1:10/ucan/", "http://127.0.0.1:9/ucan/"],
        "each service must appear once, and a space with no remote must add nothing"
    );

    // Note: this pins deduplication and the no-remote case. Whether a
    // non-UCAN remote is skipped is not exercised here, because the
    // fixture only records UCAN addresses; that a revocation actually
    // reaches every service is pinned end to end by
    // `it_denies_a_revoked_device_at_every_service_it_reaches`.

    Ok(())
}
