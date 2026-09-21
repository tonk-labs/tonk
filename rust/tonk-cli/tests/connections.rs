//! Milestone-one executable coverage of the candidate scoped build preset.
//! This fixture does not implement production invitation import or migration.
mod common;

use anyhow::Result;
use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier, Signer};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_effects::storage::Directory;
use dialog_operator::{DeriveOperator, Profile};
use dialog_repository::SiteAddress;
use dialog_storage::provider::storage::{NativeSpace, Storage};
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal;
use tonk_cli::site::TonkSite;

async fn scoped_site(
    root: &std::path::Path,
    owner: &Ed25519Signer,
    endpoint: &str,
) -> Result<TonkSite> {
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let config = common::isolated_config(&root)?;
    let unrelated = TonkSite::init_at_with(&root.join("unrelated"), config.clone()).await?;
    let unrelated_subject = unrelated.repository.did();
    let storage = Storage::<NativeSpace>::default();
    let profile = Profile::load(config.profile_name.clone())
        .at(config.profile_directory.clone())
        .perform(&storage)
        .await?;
    let replica = root.join("replica");
    std::fs::create_dir_all(&replica)?;
    let operator = profile
        .derive("connection-mount")
        .base(Directory::At(replica.to_string_lossy().into_owned()))
        .build(storage)
        .await?;
    let expiry = Timestamp::new(SystemTime::now() + Duration::from_secs(90 * 86400))?;
    // The profile already owns an unrelated local space. No target-space signer
    // or wider target-space grant is installed.
    for scope in tonk_invite::connection::candidate_build_scopes(&owner.did()) {
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(&profile.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(expiry)
            .try_build()
            .await?;
        profile
            .save(UcanDelegation(DelegationChain::new(grant)))
            .perform(&operator)
            .await?;
    }
    let verifier: Ed25519Verifier = owner
        .did()
        .to_string()
        .parse()
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    Subject::from(profile.did())
        .attenuate(Space::new("main"))
        .create(Credential::from(verifier))
        .perform(&operator)
        .await?;
    let site = TonkSite::open_with(&replica, config).await?;
    assert_ne!(site.repository.did(), unrelated_subject);
    // Deliberately wire only content. The legacy remote helper also syncs meta,
    // which is outside these grants and cannot silently be added to the preset.
    site.repository
        .remote("origin")
        .create(SiteAddress::from(dialog_remote_ucan::UcanAddress::new(
            format!("{endpoint}/ucan/"),
        )))
        .perform(&site.operator)
        .await?;
    let remote = site
        .repository
        .remote("origin")
        .load()
        .perform(&site.operator)
        .await?;
    let upstream = remote.branch("main").open().perform(&site.operator).await?;
    site.branch()
        .await?
        .handle()
        .set_upstream(&upstream)
        .perform(&site.operator)
        .await?;
    Ok(site)
}

#[dialog_common::test]
async fn connection_candidate_grants_cover_cli_authoring_and_remote_sync() -> Result<()> {
    let s3 =
        dialog_remote_s3::helpers::LocalS3::start_with_auth("test", "test", &["build"]).await?;
    let server = tonk_access_service::helpers::AccessServer::start(
        s3, "build", "test", "test", None, None, None,
    )
    .await?;
    let temp = tempfile::tempdir()?;
    let owner = Ed25519Signer::generate().await?;
    let address = tonk_access_service::helpers::AccessServiceAddress {
        access_service_url: server.endpoint.clone(),
        s3_endpoint: server.s3_server.endpoint.clone(),
        bucket: "build".into(),
        access_key_id: "test".into(),
        secret_access_key: "test".into(),
        service_did: server.service_did.clone(),
        service_seed: server.service_seed.clone(),
    };
    address.provision_subject(owner.did().as_ref()).await?;
    let writer = scoped_site(&temp.path().join("writer"), &owner, &server.endpoint).await?;
    tonk_cli::eval::run_against_site(
        &writer,
        tonk_cli::eval::Source::Inline(format!(
            "{}\n{}\n{}",
            common::ATTRIBUTE_DECL,
            common::CONCEPT_DECL,
            common::VIEW_DECL,
        )),
        tonk_cli::eval::Options::default(),
    )
    .await?;
    tonk_cli::eval::run_against_site(
        &writer,
        tonk_cli::eval::Source::Inline(
            "task!: &first-task\n  title: Scoped build\n  done: false\n\npage!: &agent-page\n  body: '<h1>Scoped build</h1>'\n".into(),
        ),
        tonk_cli::eval::Options::default(),
    ).await?;
    let blob_bytes = b"scoped connection blob roundtrip";
    let blob_file = temp.path().join("asset.txt");
    std::fs::write(&blob_file, blob_bytes)?;
    let blob = tonk_cli::blob::add(&writer, &blob_file, None).await?;
    tonk_cli::sync::push(&writer).await?;
    let expected = writer.branch().await?.handle().revision().unwrap().tree;
    let reader = scoped_site(&temp.path().join("reader"), &owner, &server.endpoint).await?;
    tonk_cli::sync::pull(&reader).await?;
    assert_eq!(
        reader.branch().await?.handle().revision().unwrap().tree,
        expected
    );
    let views = tonk_cli::views::list(&reader).await?;
    assert!(
        views
            .iter()
            .any(|view| view.name.as_deref() == Some("agent-page"))
    );
    let mut readback = Vec::new();
    tonk_cli::blob::cat(&reader, &blob.entity.to_string(), &mut readback).await?;
    assert_eq!(readback, blob_bytes);
    assert!(writer.account_store.account()?.is_none());
    assert!(reader.account_store.account()?.is_none());
    Ok(())
}

async fn agent_link(seed: [u8; 32], remote: &url::Url) -> Result<(String, String)> {
    let owner = Signer::from(Ed25519Signer::import(&[82; 32]).await?);
    let recipient = Ed25519Signer::import(&seed).await?;
    let scopes = tonk_invite::connection::candidate_build_scopes(&owner.did());
    let expiry = Timestamp::try_from((Timestamp::now().to_unix() + 90 * 86400) as i128)?;
    let mut chains = Vec::new();
    for scope in &scopes {
        let leaf = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(expiry)
            .meta(tonk_invite::home_address_meta(remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(leaf));
    }
    let invite =
        tonk_invite::connection::AgentInvite::new(seed, chains, &scopes, remote, Timestamp::now())
            .await?;
    Ok((
        invite.to_url("https://tonk.network/connect")?,
        recipient.did().to_string(),
    ))
}

#[dialog_common::test]
async fn connection_import_preserves_identity_private_credentials_and_offline_edits() -> Result<()>
{
    use std::os::unix::fs::PermissionsExt as _;
    use tonk_cli::connections;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("scoped");
    let ambient = tonk_cli::space::SpaceStore::at(temp.path().join("ambient"));
    let remote = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let (link, recipient) = agent_link([84; 32], &remote).await?;
    let validated = connections::validate_link(&link, &remote).await?;
    let binding = connections::import_at(&root, &validated, ambient.clone()).await?;
    assert_eq!(binding.recipient, recipient);
    assert!(!root.join("main").exists());
    assert!(root.join("data/main").is_dir());
    let key = root.join("credentials/invitation/credential/key/self");
    assert_eq!(std::fs::metadata(&key)?.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        std::fs::metadata(root.join("credentials"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let journal = std::fs::read_to_string(root.join(connections::MARKER_FILE))?;
    assert!(!journal.contains(&link));
    assert!(!journal.contains(&hex::encode([84; 32])));
    let site = connections::open_bound(&root, &binding, ambient.clone()).await?;
    assert!(site.is_scoped());
    assert_eq!(site.profile.did().to_string(), recipient);
    assert!(tonk_cli::custody::site_seed(&site).await?.is_none());
    assert!(
        tonk_cli::site::Identity::of(&site)
            .await?
            .account()
            .is_none()
    );
    tonk_cli::eval::run_against_site(
        &site,
        tonk_cli::eval::Source::Inline(format!(
            "{}\n{}",
            common::ATTRIBUTE_DECL,
            common::CONCEPT_DECL
        )),
        tonk_cli::eval::Options::default(),
    )
    .await?;
    let tree = site.branch().await?.handle().revision().unwrap().tree;
    drop(site);
    // Repeated import must not rewrite the replica or replace the retained key.
    assert_eq!(
        connections::import_at(&root, &validated, ambient.clone()).await?,
        binding
    );
    let reopened = connections::open_bound(&root, &binding, ambient.clone()).await?;
    assert_eq!(
        reopened.branch().await?.handle().revision().unwrap().tree,
        tree
    );
    assert_eq!(reopened.profile.did().to_string(), recipient);
    assert!(
        !ambient.root().exists(),
        "scoped work must not initialize ambient account state"
    );
    let report = tonk_cli::sync::rejection_report(&reopened, "agent", "expired").await;
    assert!(!report.contains("account login"));
    assert!(report.contains("local edits are retained"));
    Ok(())
}

#[dialog_common::test]
async fn connection_rejects_legacy_opens_binding_loss_and_missing_credentials() -> Result<()> {
    use tonk_cli::connections;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("scoped");
    let config = common::isolated_config(temp.path())?;
    let remote = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let (link, _) = agent_link([85; 32], &remote).await?;
    let validated = connections::validate_link(&link, &remote).await?;
    let binding = connections::import_at(&root, &validated, config.account_store.clone()).await?;
    for directory in [&root, &root.join("data")] {
        assert!(
            TonkSite::open_with(directory, config.clone())
                .await
                .is_err()
        );
        assert!(
            TonkSite::init_at_with(directory, config.clone())
                .await
                .is_err()
        );
        assert!(
            tonk_cli::site::transplant_at_with(directory, "wrong", config.clone())
                .await
                .is_err()
        );
    }
    assert!(!root.join("main").exists());
    let marker = std::fs::read(root.join(connections::MARKER_FILE))?;
    std::fs::remove_file(root.join(connections::MARKER_FILE))?;
    assert!(connections::binding_at(&root).is_err());
    assert!(TonkSite::init_at_with(&root, config.clone()).await.is_err());
    std::fs::write(root.join(connections::MARKER_FILE), marker)?;
    let mut altered = binding.clone();
    altered.subject = altered.recipient.clone();
    assert!(
        connections::open_bound(&root, &altered, config.account_store.clone())
            .await
            .is_err()
    );
    let sentinel = root.join("data").join(connections::DATA_MARKER_FILE);
    std::fs::write(&sentinel, "wrong-binding")?;
    assert!(
        connections::import_at(&root, &validated, config.account_store.clone())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&sentinel)?, "wrong-binding");
    std::fs::write(&sentinel, &binding.id)?;
    std::fs::create_dir(root.join("main"))?;
    assert!(
        connections::import_at(&root, &validated, config.account_store.clone())
            .await
            .is_err()
    );
    assert!(
        connections::open_bound(&root, &binding, config.account_store.clone())
            .await
            .is_err()
    );
    std::fs::remove_dir(root.join("main"))?;
    let key = root.join("credentials/invitation/credential/key/self");
    std::fs::remove_file(&key)?;
    assert!(
        connections::open_bound(&root, &binding, config.account_store.clone())
            .await
            .is_err()
    );
    assert!(
        connections::import_at(&root, &validated, config.account_store)
            .await
            .is_err()
    );
    assert!(
        !key.exists(),
        "missing ready credentials must never be regenerated"
    );
    Ok(())
}

#[dialog_common::test]
async fn connection_resumes_saved_credentials_without_bearer_and_preserves_distinct_invites()
-> Result<()> {
    use tonk_cli::connections;
    let temp = tempfile::tempdir()?;
    let store = tonk_cli::space::SpaceStore::at(temp.path().join("state"));
    let remote = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let (link, _) = agent_link([86; 32], &remote).await?;
    let validated = connections::validate_link(&link, &remote).await?;
    let root = temp.path().join("scoped");
    let binding = connections::import_at(&root, &validated, store.clone()).await?;
    let file = root.join(connections::MARKER_FILE);
    let original: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
    for phase in ["preparing", "credentials", "mounted"] {
        let mut checkpoint = original.clone();
        checkpoint["phase"] = phase.into();
        std::fs::write(&file, serde_json::to_vec(&checkpoint)?)?;
        let reopened = connections::open_bound(&root, &binding, store.clone()).await?;
        assert_eq!(reopened.profile.did().to_string(), binding.recipient);
        drop(reopened);
        let completed: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
        assert_eq!(completed["phase"], "ready");
    }
    let (other_link, _) = agent_link([87; 32], &remote).await?;
    let other = connections::validate_link(&other_link, &remote).await?;
    assert_ne!(binding.id, other.binding().id);
    assert_eq!(binding.subject, other.binding().subject);
    assert!(
        connections::import_at(&root, &other, store.clone())
            .await
            .is_err()
    );
    let other_root = temp.path().join("other");
    let other_binding = connections::import_at(&other_root, &other, store.clone()).await?;
    assert_ne!(
        connections::open_bound(&other_root, &other_binding, store)
            .await?
            .profile
            .did()
            .to_string(),
        binding.recipient
    );
    Ok(())
}

#[dialog_common::test]
async fn connection_refuses_foreign_directories_and_symlinked_storage_before_writing() -> Result<()>
{
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    use tonk_cli::connections;
    let temp = tempfile::tempdir()?;
    let store = tonk_cli::space::SpaceStore::at(temp.path().join("state"));
    let remote = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let (link, _) = agent_link([88; 32], &remote).await?;
    let validated = connections::validate_link(&link, &remote).await?;
    let foreign = temp.path().join("foreign");
    std::fs::create_dir(&foreign)?;
    std::fs::set_permissions(&foreign, std::fs::Permissions::from_mode(0o755))?;
    std::fs::write(foreign.join("keep"), b"unrelated edits")?;
    assert!(
        connections::import_at(&foreign, &validated, store.clone())
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::metadata(&foreign)?.permissions().mode() & 0o777,
        0o755
    );
    assert!(
        connections::open_bound(&foreign, validated.binding(), store.clone())
            .await
            .is_err()
    );
    assert!(!foreign.join(".connection.lock").exists());
    let root = temp.path().join("scoped");
    let binding = connections::import_at(&root, &validated, store.clone()).await?;
    let credentials = root.join("credentials");
    let saved = root.join("saved-credentials");
    std::fs::rename(&credentials, &saved)?;
    symlink(&foreign, &credentials)?;
    assert!(
        connections::import_at(&root, &validated, store.clone())
            .await
            .is_err()
    );
    assert!(
        connections::open_bound(&root, &binding, store)
            .await
            .is_err()
    );
    assert!(!foreign.join("invitation").exists());
    assert_eq!(std::fs::read(foreign.join("keep"))?, b"unrelated edits");
    Ok(())
}

#[dialog_common::test]
async fn connection_rejects_local_upstream_without_silently_repairing_it() -> Result<()> {
    use tonk_cli::connections;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("scoped");
    let store = tonk_cli::space::SpaceStore::at(temp.path().join("state"));
    let remote = url::Url::parse("http://127.0.0.1:9/ucan/")?;
    let (link, _) = agent_link([89; 32], &remote).await?;
    let validated = connections::validate_link(&link, &remote).await?;
    let binding = connections::import_at(&root, &validated, store.clone()).await?;
    let site = connections::open_bound(&root, &binding, store.clone()).await?;
    let local = site
        .repository
        .branch("local-only")
        .open()
        .perform(&site.operator)
        .await?;
    site.branch()
        .await?
        .handle()
        .set_upstream(&local)
        .perform(&site.operator)
        .await?;
    drop(site);
    assert!(
        connections::open_bound(&root, &binding, store.clone())
            .await
            .is_err()
    );
    assert!(
        connections::import_at(&root, &validated, store)
            .await
            .is_err()
    );
    Ok(())
}
