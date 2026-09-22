//! Executable scoped connection import, restart, and acknowledgement contracts.
mod common;

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_query::the;
use dialog_ucan_core::{
    DelegationBuilder, DelegationChain,
    time::{Duration, SystemTime, Timestamp},
};
use dialog_varsig::Principal as _;
use std::collections::BTreeMap;
use std::path::Path;
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};

fn cli(home: &Path, cwd: &Path) -> std::process::Command {
    let binary = std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into());
    let mut command = std::process::Command::new(binary);
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPACES_STATE", home.join("state"))
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_CONNECTION_ORIGIN")
        .env_remove("TONK_TEST_CONNECTION_CHECKPOINT")
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT")
        .env_remove("TONK_SPACE");
    command
}

async fn run(mut command: std::process::Command) -> Result<std::process::Output> {
    Ok(tokio::task::spawn_blocking(move || command.output()).await??)
}

#[tokio::test]
async fn connection_command_rejects_account_flags_and_implicit_resume_without_mutation()
-> Result<()> {
    let home = tempfile::tempdir()?;
    let secret = "https://example.test/join#tonk-agent-v1=never-print-this-secret";
    for flags in [
        vec!["--no-open"],
        vec!["--via", "https://example.test"],
        vec!["--switch-account", "did:key:unrelated"],
    ] {
        let mut command = cli(home.path(), home.path());
        command.args(["join", secret]).args(flags);
        let output = run(command).await?;
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unexpected argument"), "{stderr}");
        assert!(!stderr.contains(secret) && !stderr.contains("never-print-this-secret"));
        assert!(!home.path().join("state").exists());
    }
    let mut command = cli(home.path(), home.path());
    command.arg("join").env("TONK_SPACE", "ambient");
    let output = run(command).await?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--space NAME join"));
    assert!(!home.path().join("state").exists());
    Ok(())
}

#[tokio::test]
async fn removed_workflows_preserve_existing_local_state() -> Result<()> {
    fn snapshot(root: &Path) -> Result<BTreeMap<std::path::PathBuf, Vec<u8>>> {
        fn visit(
            root: &Path,
            dir: &Path,
            files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) -> Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    visit(root, &path, files)?;
                } else if entry.file_type()?.is_file() {
                    files.insert(path.strip_prefix(root)?.to_owned(), std::fs::read(path)?);
                }
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files)?;
        Ok(files)
    }
    let home = tempfile::tempdir()?;
    let mut create = cli(home.path(), home.path());
    create.args(["space", "new", "retained"]);
    let output = run(create).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Include credentials at the platform's actual data path, as well as the
    // registry and replicas. macOS does not use the XDG data directory.
    let before = snapshot(home.path())?;
    for (args, expected) in [
        (
            vec![
                "join",
                "https://example.test/join#never-print-secret",
                "--name",
                "new",
            ],
            "invalid invite",
        ),
        (vec!["join", "--agent"], "unexpected argument"),
        (vec!["connect"], "unrecognized subcommand"),
        (vec!["link"], "unrecognized subcommand"),
        (vec!["account", "login"], "unrecognized subcommand"),
        (vec!["account", "logout"], "unrecognized subcommand"),
        (vec!["account", "delete"], "unrecognized subcommand"),
        (vec!["account", "devices"], "unrecognized subcommand"),
        (vec!["account", "space"], "unrecognized subcommand"),
        (
            vec![
                "space",
                "link",
                "retained",
                "--via",
                "file:///invalid/settings/link",
            ],
            "account approval page must use HTTP or HTTPS",
        ),
        (vec!["migrate", "account"], "unrecognized subcommand"),
        (
            vec!["join", "https://example.test/join#never-print-secret"],
            "invalid invite",
        ),
        (
            vec![
                "join",
                "https://example.test/join?access=old-account-proof#never-print-secret",
            ],
            "invalid invite",
        ),
        (
            vec!["--space", "retained", "join"],
            "unsupported_connection_resume",
        ),
    ] {
        let mut command = cli(home.path(), home.path());
        command.args(args);
        let output = run(command).await?;
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!stderr.contains("never-print-secret") && !stderr.contains("old-account-proof"));
        assert!(
            !output
                .stdout
                .windows(b"Open this URL".len())
                .any(|part| part == b"Open this URL")
        );
        assert!(
            before == snapshot(home.path())?,
            "refusal changed local state or credentials"
        );
    }
    Ok(())
}

#[dialog_common::test]
async fn connection_receipts_are_grant_set_specific_and_failed_pull_writes_none() -> Result<()> {
    let site = common::TestSite::new().await?;
    let first = "1".repeat(64);
    let second = "2".repeat(64);
    assert!(
        tonk_cli::handoff::confirm_scoped_connection(&site.site, &first)
            .await
            .is_err()
    );
    let query = |id: &str| {
        format!("agent-connection:\n  this: id:tonk:agent-connection:{id}\n  status: ?status\n")
    };
    assert!(
        site.eval_inline(&query(&first))
            .await?
            .response
            .matches_after[0]
            .results
            .is_empty()
    );
    tonk_cli::handoff::record_scoped_connection(&site.site, &first).await?;
    tonk_cli::handoff::record_scoped_connection(&site.site, &second).await?;
    for id in [&first, &second] {
        assert_eq!(
            site.eval_inline(&query(id)).await?.response.matches_after[0]
                .results
                .len(),
            1
        );
    }
    assert!(
        site.eval_inline(
            "agent-connection:\n  this: id:tonk:agent-connection\n  status: ?status\n"
        )
        .await?
        .response
        .matches_after[0]
            .results
            .is_empty()
    );
    let route = tonk_cli::render::RenderRoute::parse("tonk:agent-connection")?;
    let html = tonk_cli::render::render(&site.site, &route).await?;
    for id in [&first, &second] {
        assert!(
            html.contains(&format!("id:tonk:agent-connection:{id}")),
            "{html}"
        );
    }
    assert_eq!(html.matches("agent setup confirmed").count(), 2);
    assert!(tonk_cli::handoff::scoped_connection_entity("invalid\nnotation").is_err());
    Ok(())
}

#[test]
fn connection_directory_resume_record_is_public_and_exact() -> Result<()> {
    let root = tempfile::tempdir()?;
    let directory = tempfile::tempdir()?;
    let id = "3".repeat(64);
    tonk_cli::handoff::remember_scoped_directory(root.path(), &id, directory.path())?;
    assert_eq!(
        tonk_cli::handoff::pending_scoped_directory(root.path(), &id)?,
        Some(directory.path().canonicalize()?)
    );
    assert!(tonk_cli::handoff::pending_scoped_directory(root.path(), &"4".repeat(64)).is_err());
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.path().join("agent-directory.json"))?)?;
    assert_eq!(value.as_object().unwrap().len(), 3);
    Ok(())
}

#[tokio::test]
async fn connection_command_imports_bearer_restarts_and_keeps_account_state() -> Result<()> {
    use tonk_schema::prelude::DidExt as _;
    let s3 =
        dialog_remote_s3::helpers::LocalS3::start_with_auth("test", "test", &["commands"]).await?;
    let server = tonk_access_service::helpers::AccessServer::start(
        s3,
        "commands",
        "test",
        "test",
        Some(tonk_worker_api::DeploymentConfig::default()),
        None,
        None,
    )
    .await?;
    let owner = Ed25519Signer::generate().await?;
    let address = tonk_access_service::helpers::AccessServiceAddress {
        access_service_url: server.endpoint.clone(),
        s3_endpoint: server.s3_server.endpoint.clone(),
        bucket: "commands".into(),
        access_key_id: "test".into(),
        secret_access_key: "test".into(),
        service_did: server.service_did.clone(),
        service_seed: server.service_seed.clone(),
    };
    address.provision_subject(owner.did().as_ref()).await?;
    let remote: url::Url = format!("{}/ucan/", server.endpoint).parse()?;
    let seed = [59; 32];
    let recipient = Ed25519Signer::import(&seed).await?;
    let scopes = candidate_build_scopes(&owner.did());
    let mut chains = Vec::new();
    for scope in &scopes {
        let grant = DelegationBuilder::new()
            .issuer(Signer::from(owner.clone()))
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(Timestamp::new(
                SystemTime::now() + Duration::from_secs(90 * 86400),
            )?)
            .meta(BTreeMap::from([(
                tonk_invite::HOME_ADDRESS.into(),
                ipld_core::ipld::Ipld::String(remote.to_string()),
            )]))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    let invite = AgentInvite::new(seed, chains, &scopes, &remote, Timestamp::now()).await?;
    // Carrier and ambient selection cannot choose the service or space.
    let link = invite.to_url("https://untrusted-carrier.example/join")?;
    let prepared = tonk_cli::connections::validate_link(&link, &remote).await?;
    let temp = tempfile::tempdir()?;
    let producer_root = temp.path().join("producer");
    let producer_store = tonk_cli::space::SpaceStore::at(temp.path().join("producer-state"));
    let binding =
        tonk_cli::connections::import_at(&producer_root, &prepared, producer_store.clone()).await?;
    let producer =
        tonk_cli::connections::open_bound(&producer_root, &binding, producer_store).await?;
    let entity: dialog_artifacts::Entity = "id:test:before-cli".parse()?;
    producer
        .branch()
        .await?
        .handle()
        .transaction()
        .assert(tonk_schema::RepositoryName {
            this: producer.repository.did().this(),
            name: tonk_schema::domain::repo::Name("Shared Garden".into()),
        })
        .assert(
            the!("test.connection/value")
                .of(entity)
                .is("retained remote content".to_owned()),
        )
        .commit()
        .publish()
        .perform(&producer.operator)
        .await?;
    tonk_cli::sync::push(&producer).await?;

    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let unrelated = tonk_cli::space::AccountRecord::new(owner.did().to_string());
    store.set_account(Some(unrelated.clone()))?;
    let mut command = cli(&home, &project);
    command
        .args(["join", &link, "--name", "agent"])
        .env("TONK_CONNECTION_ORIGIN", &server.endpoint)
        .env("TONK_SPACE", "must-not-select-this");
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Agent connection confirmed"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&link));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&link));
    assert_eq!(store.account()?, Some(unrelated.clone()));
    let registry = store.load()?;
    let entry = &registry.spaces["agent"];
    assert_eq!(entry.connection.as_ref(), Some(&binding));
    assert_eq!(
        registry.bindings.get(&project.canonicalize()?),
        Some(&"agent".to_owned())
    );
    assert!(
        !entry.site.join("main").exists(),
        "old CLI must not see a conventional repository at the outer root"
    );
    let resumed = tonk_cli::connections::open_bound(&entry.site, &binding, store.clone()).await?;
    let local = resumed.branch().await?.handle().revision().unwrap().tree;
    tonk_cli::sync::pull(&producer).await?;
    assert_eq!(
        producer.branch().await?.handle().revision().unwrap().tree,
        local
    );

    // A new process uses retained credentials without the URL or discovery.
    let mut command = cli(&home, &project);
    command
        .args(["--space", "agent", "join"])
        .env("TONK_CONNECTION_ORIGIN", "invalid-unused-on-resume");
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut command = cli(&home, &project);
    command.args(["--space", "agent", "status", "--json"]);
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        status["authority"]["recipient"],
        recipient.did().to_string()
    );
    assert_eq!(status["authority"]["kind"], "invitation");
    assert_eq!(status["schemaVersion"], "tonk.status.v3");
    assert!(status.get("account").is_none(), "{status}");
    assert!(status.get("signedIn").is_none(), "{status}");
    assert_eq!(store.account()?, Some(unrelated));
    assert!(
        !home.join("data").exists(),
        "scoped commands must not open a default profile"
    );
    for file in ["connection.json", "agent-directory.json"] {
        let text = std::fs::read_to_string(entry.site.join(file))?;
        assert!(!text.contains(&link));
        assert!(!text.contains(&hex::encode(seed)));
    }
    // A default import reads the hub's name and survives a fresh-process retry.
    let default_home = temp.path().join("default-home");
    std::fs::create_dir(&default_home)?;
    for _ in 0..2 {
        let mut command = cli(&default_home, &default_home);
        command
            .args(["join", &link])
            .env("TONK_CONNECTION_ORIGIN", &server.endpoint);
        let output = run(command).await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("space: shared-garden"), "{stdout}");
        assert!(stdout.contains("Agent connection confirmed"), "{stdout}");
    }
    let default_store = tonk_cli::space::SpaceStore::at(default_home.join("state"));
    let default_registry = default_store.load()?;
    assert_eq!(default_registry.spaces.len(), 1);
    assert_eq!(
        default_registry.spaces["shared-garden"].connection.as_ref(),
        Some(&binding)
    );
    assert_eq!(
        default_registry.bindings[&default_home.canonicalize()?],
        "shared-garden"
    );

    // Kill a real import after it persisted Ready but before registry publication.
    // The held write guard is an explicit barrier, not an assumed delay.
    let interrupted_directory = temp.path().join("interrupted-project");
    std::fs::create_dir(&interrupted_directory)?;
    let guard = store.write_guard()?;
    let mut command = cli(&home, &interrupted_directory);
    command
        .args(["join", &link, "--name", "interrupted"])
        .env("TONK_CONNECTION_ORIGIN", &server.endpoint)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn()?;
    let marker = store.canonical_site("interrupted").join("connection.json");
    let ready = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let phase = std::fs::read(&marker)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if phase
                .as_ref()
                .is_some_and(|value| value["phase"] == "ready")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    let _ = child.kill();
    let interrupted_output =
        tokio::task::spawn_blocking(move || child.wait_with_output()).await??;
    drop(guard);
    assert!(
        ready.is_ok(),
        "import never reached publication barrier: {}",
        String::from_utf8_lossy(&interrupted_output.stderr)
    );
    assert!(!store.load()?.spaces.contains_key("interrupted"));
    assert!(
        !String::from_utf8_lossy(&interrupted_output.stdout).contains("Agent connection confirmed")
    );
    let mut command = cli(&home, &home);
    command
        .args(["--space", "interrupted", "join"])
        .env("TONK_SPACE", "agent");
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        store
            .load()?
            .bindings
            .get(&interrupted_directory.canonicalize()?),
        Some(&"interrupted".to_owned())
    );
    assert!(!store.load()?.bindings.contains_key(&home.canonicalize()?));

    // These deterministic process exits are compiled out of production builds.
    #[cfg(feature = "integration-tests")]
    for phase in ["credentials", "mounted"] {
        let directory = temp.path().join(format!("{phase}-project"));
        std::fs::create_dir(&directory)?;
        let mut command = cli(&home, &directory);
        command
            .args(["join", &link, "--name", phase])
            .env("TONK_CONNECTION_ORIGIN", &server.endpoint)
            .env("TONK_TEST_CONNECTION_CHECKPOINT", phase);
        let output = run(command).await?;
        assert_eq!(
            output.status.code(),
            Some(86),
            "{phase}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!store.load()?.spaces.contains_key(phase));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Agent connection confirmed"));
        let mut command = cli(&home, &home);
        command.args(["--space", phase, "join"]);
        let output = run(command).await?;
        assert!(
            output.status.success(),
            "{phase}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            store.load()?.bindings.get(&directory.canonicalize()?),
            Some(&phase.to_owned())
        );
    }
    // A different default profile retains a live, broader grant to the same
    // subject. Scoped selection must never replace the revoked invitation with it.
    use dialog_effects::storage::Directory;
    use dialog_operator::{DeriveOperator, Profile};
    use dialog_storage::provider::storage::{NativeSpace, Storage};
    use dialog_ucan::UcanDelegation;
    #[cfg(target_os = "macos")]
    let profile_parent = home.join("Library/Application Support/dialog");
    #[cfg(not(target_os = "macos"))]
    let profile_parent = home.join("data/dialog");
    std::fs::create_dir_all(&profile_parent)?;
    let storage = Storage::<NativeSpace>::default();
    let ambient_profile = Profile::create(tonk_cli::site::PROFILE_NAME)
        .at(Directory::At(profile_parent.to_string_lossy().into_owned()))
        .perform(&storage)
        .await?;
    let ambient_base = home.join("ambient-data");
    std::fs::create_dir(&ambient_base)?;
    let ambient_operator = ambient_profile
        .derive("ambient-authority")
        .base(Directory::At(ambient_base.to_string_lossy().into_owned()))
        .build(storage)
        .await?;
    let ambient_grant = DelegationBuilder::new()
        .issuer(Signer::from(owner.clone()))
        .audience(&ambient_profile.did())
        .subject(dialog_ucan_core::subject::Subject::Specific(owner.did()))
        .command(vec!["use".into()])
        .expiration(Timestamp::new(
            SystemTime::now() + Duration::from_secs(90 * 86400),
        )?)
        .try_build()
        .await?;
    ambient_profile
        .save(UcanDelegation(DelegationChain::new(ambient_grant)))
        .perform(&ambient_operator)
        .await?;
    let mut command = cli(&home, &home);
    command.arg("identity");
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("device: {}", ambient_profile.did()))
    );

    // The acknowledged standard revocations are the barrier: every following
    // command is a new process and cannot reuse a cached S3 descriptor.
    for chain in invite.grants().chains() {
        let bytes = tonk_identity::revocation::mint_root_revocation(
            owner.clone(),
            chain,
            chain.proof_cids().last().unwrap(),
        )
        .await?;
        reqwest::Client::new()
            .post(remote.clone())
            .header("Content-Type", "application/cbor")
            .body(bytes)
            .send()
            .await?
            .error_for_status()?;
    }
    let before = tonk_cli::connections::open_bound(&entry.site, &binding, store.clone())
        .await?
        .branch()
        .await?
        .handle()
        .revision()
        .unwrap()
        .tree;
    let document = "attribute!: &after-revoke\n  description: Offline edit after revocation\n  the: test.connection/offline-after-revoke\n  as: text\n  cardinality: one\n";
    let mut command = cli(&home, &project);
    command.args(["--space", "agent", "eval", "-c", document, "--no-sync"]);
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let edited = tonk_cli::connections::open_bound(&entry.site, &binding, store.clone())
        .await?
        .branch()
        .await?
        .handle()
        .revision()
        .unwrap()
        .tree;
    assert_ne!(before, edited, "offline eval must persist a real edit");
    for operation in ["pull", "push", "join"] {
        let mut command = cli(&home, &project);
        command
            .args(["--space", "agent", operation])
            .env("TONK_UNSAFE_ALLOW_DEVICE_ROOT", "1");
        let output = run(command).await?;
        assert!(
            !output.status.success(),
            "revoked {operation} unexpectedly succeeded"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("has been revoked"), "{operation}: {error}");
        assert!(!error.contains("account login"), "{operation}: {error}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Agent connection confirmed"));
    }
    let retained = tonk_cli::connections::open_bound(&entry.site, &binding, store.clone()).await?;
    assert_eq!(retained.profile.did(), recipient.did());
    assert_eq!(
        retained.branch().await?.handle().revision().unwrap().tree,
        edited
    );
    Ok(())
}
