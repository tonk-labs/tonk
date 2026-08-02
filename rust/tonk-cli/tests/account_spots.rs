//! Live account-spots coverage against the native account service.

mod common;

use std::io::{Read as _, Write as _};
use std::net::TcpListener;

use anyhow::Result;
use dialog_query::{Output as _, Query, Term};
use tonk_access_service::helpers::AccessServiceAddress;
use tonk_cli::account_spots;
use tonk_cli::site::TonkSite;
use tonk_cli::spot::SpotEntry;
use tonk_schema::RepositoryName;
use tonk_schema::prelude::DidExt as _;

fn legacy_capability_server(
    good: Vec<u8>,
    fail_valid_get: bool,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut get_index = 0;
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                let Some(headers_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + length {
                    break;
                }
            }
            let path = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap()
                .to_string();
            let (status, content_type, body) = match path.as_str() {
                "/chains/list" => (
                    "200 OK",
                    "application/json",
                    serde_json::to_vec(&vec!["bad", "good"]).unwrap(),
                ),
                "/chains/get" => {
                    let response = if get_index == 0 {
                        (
                            "200 OK",
                            "application/octet-stream",
                            b"not an account spot".to_vec(),
                        )
                    } else if fail_valid_get {
                        (
                            "503 Service Unavailable",
                            "text/plain",
                            b"temporarily unavailable".to_vec(),
                        )
                    } else {
                        ("200 OK", "application/octet-stream", good.clone())
                    };
                    get_index += 1;
                    response
                }
                other => panic!("an unadvertised service must not receive {other}"),
            };
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (endpoint, handle)
}

#[tokio::test]
async fn list_reports_named_unnamed_and_pullable_rows() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (named_subject, named) = fixture
        .backup(81, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    let (unnamed_subject, unnamed) = fixture.backup(82, None, None).await?;
    fixture.put(&named).await?;
    fixture.put(&unnamed).await?;

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
async fn list_uses_legacy_routes_when_the_capability_is_absent() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (subject, backup) = fixture
        .backup(87, None, Some("https://access.example/ucan/"))
        .await?;
    let (endpoint, server) = legacy_capability_server(serde_json::to_vec(&backup)?, false);
    tonk_cli::account::attach_for_integration_test(
        &fixture.profile,
        &TonkSite::open_with(
            fixture.tmp.path().join(".tonk").as_path(),
            fixture.config.clone(),
        )
        .await?
        .operator,
        fixture.config.clone(),
        &endpoint,
        "fixture-credential",
        fixture.link.clone(),
        &fixture.descriptor,
    )
    .await?;

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, subject.to_string());
    assert!(rows[0].remote_name.is_none());
    assert!(rows[0].pullable);
    server.join().unwrap();
    Ok(())
}

#[tokio::test]
async fn legacy_list_propagates_fetch_failures_without_hiding_malformed_blobs() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (_, backup) = fixture
        .backup(88, None, Some("https://access.example/ucan/"))
        .await?;
    let (endpoint, server) = legacy_capability_server(serde_json::to_vec(&backup)?, true);
    tonk_cli::account::attach_for_integration_test(
        &fixture.profile,
        &TonkSite::open_with(
            fixture.tmp.path().join(".tonk").as_path(),
            fixture.config.clone(),
        )
        .await?
        .operator,
        fixture.config.clone(),
        &endpoint,
        "fixture-credential",
        fixture.link.clone(),
        &fixture.descriptor,
    )
    .await?;

    let error = account_spots::list(&fixture.profile, &fixture.store)
        .await
        .expect_err("a partial old-service inventory must not look empty");
    assert!(error.to_string().contains("503"), "{error:#}");
    server.join().unwrap();
    Ok(())
}

#[tokio::test]
async fn pull_requires_an_explicit_name_before_local_mutation() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (unnamed_subject, unnamed) = fixture
        .backup(83, None, Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture.put(&unnamed).await?;
    let error = account_spots::pull(
        &fixture.profile,
        &fixture.store,
        unnamed_subject.as_ref(),
        None,
    )
    .await
    .expect_err("legacy names require --name");
    assert!(error.to_string().contains("pass --name"), "{error:#}");
    assert!(!fixture.store.canonical_site("garden").exists());
    assert!(fixture.store.load()?.spots.is_empty());

    let (invalid_subject, invalid) = fixture
        .backup(84, Some("My Garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture.put(&invalid).await?;
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

    let (colliding_subject, colliding) = fixture
        .backup(89, Some("occupied"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture.put(&colliding).await?;
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
async fn pull_retains_an_unbound_canonical_spot_when_initial_sync_is_offline() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (subject, backup) = fixture
        .backup(85, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture.put(&backup).await?;

    let outcome =
        account_spots::pull(&fixture.profile, &fixture.store, subject.as_ref(), None).await?;
    assert!(!outcome.already_local);
    assert!(
        outcome
            .warning
            .as_deref()
            .is_some_and(|warning| { warning.contains("run `tonk pull`") })
    );
    assert_eq!(
        outcome.site,
        fixture.store.canonical_site("garden").canonicalize()?
    );
    let registry = fixture.store.load()?;
    assert_eq!(registry.spots["garden"].site, outcome.site);
    assert!(registry.bindings.is_empty());

    let second =
        account_spots::pull(&fixture.profile, &fixture.store, subject.as_ref(), None).await?;
    assert!(second.already_local);
    assert_eq!(second.name, "garden");

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows[0].local_name.as_deref(), Some("garden"));
    Ok(())
}

#[tokio::test]
async fn pull_requires_a_backed_inventory_row_and_returns_an_adopted_site() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;

    let adopted_root = fixture.tmp.path().join("adopted-site");
    let adopted = TonkSite::init_at_with(&adopted_root, fixture.config.clone()).await?;
    let adopted_subject = adopted.repository.did();
    let adopted_prefix =
        tonk_cli::site::account_root_prefix(&adopted, fixture.link.issuer()).await?;
    fixture
        .put(&tonk_account::backup::AccountSpotBackup {
            chain_hex: hex::encode(adopted_prefix.to_bytes()?),
            remote_url: Some("http://127.0.0.1:9/ucan/".to_string()),
            revocation_url: None,
            name: Some("adopted".to_string()),
        })
        .await?;
    let mut registry = fixture.store.load()?;
    registry.spots.insert(
        "adopted".to_string(),
        SpotEntry {
            site: adopted.root.clone(),
        },
    );
    fixture.store.save(&registry)?;

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

    let local_root = fixture.tmp.path().join("unbacked-local-site");
    let local = TonkSite::init_at_with(&local_root, fixture.config.clone()).await?;
    let mut registry = fixture.store.load()?;
    registry.spots.insert(
        "unbacked".to_string(),
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
    .expect_err("an unbacked local subject is not an account spot");
    assert!(
        error.to_string().contains("no account spot is backed up"),
        "{error:#}"
    );
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

    let prefix = tonk_cli::site::account_root_prefix(&source, fixture.link.issuer()).await?;
    fixture
        .put(&tonk_account::backup::AccountSpotBackup {
            chain_hex: hex::encode(prefix.to_bytes()?),
            remote_url: Some(env.access_service_url.clone()),
            revocation_url: None,
            name: Some("garden".to_string()),
        })
        .await?;

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

#[tokio::test]
async fn backup_reconciles_owned_joined_recovered_and_newly_remote_sites() -> Result<()> {
    use tonk_account::backup::SPACE_ROOT_SITE_PREFIX;
    use tonk_cli::account_spots::BackupOutcome;

    let fixture = common::AccountFixture::new().await?;
    let dead_remote = "http://127.0.0.1:9/ucan/";

    let owned = TonkSite::init_at_with(
        &fixture.store.canonical_site("owned-alias"),
        fixture.config.clone(),
    )
    .await?;
    name_repository(&owned, "synced-owned").await?;
    configure_upstream(&owned, dead_remote).await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "owned-alias", &owned.root)?;
    assert!(matches!(
        account_spots::back_up_site("owned-alias", &owned).await?,
        BackupOutcome::Uploaded { .. }
    ));
    assert_eq!(
        account_spots::back_up_site("owned-alias", &owned).await?,
        BackupOutcome::Unchanged
    );

    let (joined_subject, joined_artifact) = fixture.backup(90, None, Some(dead_remote)).await?;
    let joined_chain = joined_artifact
        .validate_for(fixture.link.issuer())
        .await?
        .chain;
    let joined = tonk_cli::site::mount_delegated_at(
        &fixture.store.canonical_site("joined-alias"),
        joined_chain,
        fixture.config.clone(),
    )
    .await?;
    configure_upstream(&joined, dead_remote).await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "joined-alias", &joined.root)?;
    assert!(matches!(
        account_spots::back_up_site("joined-alias", &joined).await?,
        BackupOutcome::Uploaded { .. }
    ));

    let recovered = TonkSite::init_at_with(
        &fixture.store.canonical_site("recovered-alias"),
        fixture.config.clone(),
    )
    .await?;
    configure_upstream(&recovered, dead_remote).await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "recovered-alias", &recovered.root)?;
    let recovered_subject = recovered.repository.did();
    recovered
        .profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{recovered_subject}"))
        .save(Vec::<u8>::new())
        .perform(&recovered.operator)
        .await?;
    assert!(matches!(
        account_spots::back_up_site("recovered-alias", &recovered).await?,
        BackupOutcome::Uploaded { .. }
    ));
    let recovered_prefix = recovered
        .profile
        .credential()
        .site(format!("{SPACE_ROOT_SITE_PREFIX}{recovered_subject}"))
        .load::<Vec<u8>>()
        .perform(&recovered.operator)
        .await?;
    assert!(!recovered_prefix.is_empty());

    let local_only = TonkSite::init_at_with(
        &fixture.store.canonical_site("local-fallback"),
        fixture.config.clone(),
    )
    .await?;
    tonk_cli::spot::register_existing_unbound(&fixture.store, "local-fallback", &local_only.root)?;
    assert_eq!(
        account_spots::back_up_site("local-fallback", &local_only).await?,
        BackupOutcome::NoUpstream
    );
    let before = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert!(
        !before
            .iter()
            .any(|row| row.subject == local_only.repository.did().to_string())
    );
    configure_upstream(&local_only, dead_remote).await?;
    assert!(matches!(
        account_spots::back_up_site("local-fallback", &local_only).await?,
        BackupOutcome::Uploaded { .. }
    ));

    let rows = account_spots::list(&fixture.profile, &fixture.store).await?;
    assert_eq!(rows.len(), 4);
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
            .find(|row| row.subject == joined_subject.to_string())
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

    // A failed best-effort sweep cannot roll back an already-committed primary
    // operation. Point only the account transport at a dead service after the
    // content commit, then observe the warning and retained name.
    name_repository(&owned, "primary-still-succeeded").await?;
    let profile_site = TonkSite::open_with(
        fixture.tmp.path().join(".tonk").as_path(),
        fixture.config.clone(),
    )
    .await?;
    tonk_cli::account::attach_for_integration_test(
        &fixture.profile,
        &profile_site.operator,
        fixture.config.clone(),
        "http://127.0.0.1:9",
        "fixture-credential",
        fixture.link.clone(),
        &fixture.descriptor,
    )
    .await?;
    let warnings = account_spots::back_up_registered(&fixture.profile, &fixture.store).await;
    assert!(!warnings.is_empty());
    let names: Vec<RepositoryName> = owned
        .branch()
        .await?
        .handle()
        .query()
        .select(Query::<RepositoryName> {
            this: Term::from(owned.repository.did().this()),
            name: Term::var("name"),
        })
        .perform(&owned.operator)
        .try_vec()
        .await?;
    assert_eq!(names[0].name.0, "primary-still-succeeded");
    Ok(())
}

#[tokio::test]
async fn backup_marks_unchanged_payloads_and_preserves_primary_state() -> Result<()> {
    let fixture = common::AccountFixture::new().await?;
    let (subject, backup) = fixture
        .backup(86, Some("garden"), Some("http://127.0.0.1:9/ucan/"))
        .await?;
    fixture.put(&backup).await?;
    account_spots::pull(&fixture.profile, &fixture.store, subject.as_ref(), None).await?;

    let warnings = account_spots::back_up_registered(&fixture.profile, &fixture.store).await;
    assert!(warnings.is_empty(), "{warnings:?}");
    let second = account_spots::back_up_registered(&fixture.profile, &fixture.store).await;
    assert!(second.is_empty(), "{second:?}");
    Ok(())
}
