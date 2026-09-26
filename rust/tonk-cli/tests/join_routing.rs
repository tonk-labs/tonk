//! Tool-only invitation routing before `tonk join` mutates local state.

mod common;

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_effects::storage::Directory;
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};

const WRONG_KIND: &str = "This link invites a person to the space.\nTo connect the CLI, ask for a link from \"connect agent\" in Tonk.";

fn cli(home: &std::path::Path, cwd: &std::path::Path) -> std::process::Command {
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
        .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT")
        .env_remove("TONK_SPACE");
    command
}

async fn run(mut command: std::process::Command) -> Result<std::process::Output> {
    Ok(tokio::task::spawn_blocking(move || command.output()).await??)
}

async fn agent_fixture(base: &str, seed: [u8; 32], expired: bool) -> Result<AgentInvite> {
    let owner = Signer::from(Ed25519Signer::import(&[71; 32]).await?);
    let recipient = Ed25519Signer::import(&seed).await?;
    let remote = url::Url::parse("https://access.example.test/ucan/")?;
    let scopes = candidate_build_scopes(&owner.did());
    let now = Timestamp::now();
    let expiration = if expired {
        Timestamp::try_from((now.to_unix() - 60) as i128)?
    } else {
        Timestamp::new(SystemTime::now() + Duration::from_secs(3600))?
    };
    let validation_time = if expired {
        Timestamp::try_from((now.to_unix() - 120) as i128)?
    } else {
        now
    };
    let mut chains = Vec::new();
    for scope in &scopes {
        chains.push(DelegationChain::new(
            DelegationBuilder::new()
                .issuer(owner.clone())
                .audience(&recipient.did())
                .subject(scope.subject.clone())
                .command(scope.command.0.clone())
                .policy(scope.policy())
                .expiration(expiration)
                .meta(tonk_invite::home_address_meta(&remote))
                .try_build()
                .await?,
        ));
    }
    let invite = AgentInvite::new(seed, chains.clone(), &scopes, &remote, validation_time).await?;
    let _ = invite.to_url(base)?;
    Ok(invite)
}

fn secret_free(error: &anyhow::Error, secrets: &[&str]) {
    let message = format!("{error:#}");
    for secret in secrets {
        assert!(
            !message.contains(secret),
            "error leaked a bearer secret: {message}"
        );
    }
}

fn snapshot(root: &std::path::Path) -> Result<BTreeMap<std::path::PathBuf, Vec<u8>>> {
    fn visit(
        root: &std::path::Path,
        at: &std::path::Path,
        files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) -> Result<()> {
        if !at.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(at)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(root, &path, files)?;
            } else {
                files.insert(path.strip_prefix(root)?.to_owned(), std::fs::read(path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

#[dialog_common::test]
async fn it_accepts_supported_tool_links_and_rejects_other_kinds_without_fallback() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let ordinary = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
    )
    .await?;
    let error = tonk_cli::join::prepare(&ordinary.url).await.unwrap_err();
    assert_eq!(format!("{error:#}"), WRONG_KIND);
    secret_free(&error, &[ordinary.url.split('#').next_back().unwrap()]);

    let targeted = common::TestSite::new().await?;
    let target = tonk_cli::site::member_did(&targeted.site).await?;
    let targeted = tonk_cli::invite::mint_targeted(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
        target.as_str(),
    )
    .await?;
    assert_eq!(
        format!(
            "{:#}",
            tonk_cli::join::prepare(&targeted.url).await.unwrap_err()
        ),
        WRONG_KIND
    );

    let seed = [72; 32];
    let agent = agent_fixture("https://carrier.example.test/join", seed, false).await?;
    for path in ["join", "agent/"] {
        let prepared = tonk_cli::join::prepare(
            &agent.to_url(&format!("https://carrier.example.test/{path}"))?,
        )
        .await?;
        assert_eq!(
            prepared.hint().remote.as_str(),
            "https://access.example.test/ucan/"
        );
    }

    let mut mixed = url::Url::parse(&agent.to_url("https://carrier.example.test/join")?)?;
    mixed
        .query_pairs_mut()
        .append_pair("access", "ordinary-proof");
    let error = tonk_cli::join::prepare(mixed.as_str()).await.unwrap_err();
    assert!(format!("{error:#}").contains("ambiguous_invitation"));
    secret_free(&error, &["ordinary-proof", "tonk-agent-v"]);

    let unsupported = agent
        .to_url("https://carrier.example.test/join")?
        .replace("tonk-agent-v2=", "tonk-agent-v99=");
    let error = tonk_cli::join::prepare(&unsupported).await.unwrap_err();
    assert!(format!("{error:#}").contains("connection_unsupported_version"));
    secret_free(&error, &["tonk-agent-v99"]);

    let malformed = "https://carrier.example.test/join?access=not-base58#never-print-secret";
    let error = tonk_cli::join::prepare(malformed).await.unwrap_err();
    assert!(format!("{error:#}").contains("invalid invite"));
    secret_free(&error, &["not-base58", "never-print-secret"]);

    let expired = agent_fixture("https://carrier.example.test/join", [73; 32], true).await?;
    let expired = expired.to_url("https://carrier.example.test/join")?;
    let error = tonk_cli::join::prepare(&expired).await.unwrap_err();
    assert!(format!("{error:#}").contains("Expired at"), "{error:#}");
    secret_free(&error, &["tonk-agent-v2"]);
    Ok(())
}

#[dialog_common::test]
async fn shortcuts_resolve_once_before_tool_only_routing() -> Result<()> {
    use axum::Router;
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;

    let location = Arc::new(RwLock::new(String::new()));
    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new().fallback({
        let location = location.clone();
        let requests = requests.clone();
        move || {
            let location = location.clone();
            let requests = requests.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(
                        header::LOCATION,
                        HeaderValue::from_str(&location.read().await).unwrap(),
                    )],
                )
                    .into_response()
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let issuer = common::TestSite::new().await?;
    let ordinary =
        tonk_cli::invite::mint(&issuer.site, Some(&format!("{base}/join")), None).await?;
    let ordinary_url = url::Url::parse(&ordinary.url)?;
    let mut ordinary_location = ordinary_url.clone();
    ordinary_location.set_fragment(None);
    *location.write().await = ordinary_location.to_string();
    let short = format!(
        "{base}/@/{}#{}",
        "a".repeat(64),
        ordinary_url.fragment().unwrap()
    );
    assert_eq!(
        format!("{:#}", tonk_cli::join::prepare(&short).await.unwrap_err()),
        WRONG_KIND
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    let agent = agent_fixture(&format!("{base}/join"), [74; 32], false).await?;
    let agent_url = url::Url::parse(&agent.to_url(&format!("{base}/agent/"))?)?;
    let mut agent_location = agent_url.clone();
    agent_location.set_fragment(None);
    *location.write().await = agent_location.to_string();
    let short = format!(
        "{base}/@/{}#{}",
        "b".repeat(64),
        agent_url.fragment().unwrap()
    );
    tonk_cli::join::prepare(&short).await?;
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    server.abort();
    Ok(())
}

#[dialog_common::test]
async fn ordinary_links_fail_before_any_cli_state_or_binding_write() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let recipient = common::TestSite::new().await?;
    let recipient_root = tonk_cli::site::member_did(&recipient.site).await?;
    let links = [
        tonk_cli::invite::mint(
            &issuer.site,
            Some("https://carrier.example.test/join"),
            None,
        )
        .await?
        .url,
        tonk_cli::invite::mint_targeted(
            &issuer.site,
            Some("https://carrier.example.test/join"),
            None,
            recipient_root.as_str(),
        )
        .await?
        .url,
    ];
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let account = tonk_cli::space::AccountRecord::new("did:key:unchanged");
    store.set_account(Some(account.clone()))?;
    std::fs::write(
        home.join("legacy-account-sentinel"),
        b"malformed-but-untouched",
    )?;
    let before = snapshot(&home)?;
    for link in links {
        let mut command = cli(&home, &project);
        command.args(["join", &link, "--name", "must-not-exist"]);
        let output = run(command).await?;
        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            format!("error: {WRONG_KIND}")
        );
        assert!(output.stdout.is_empty());
        assert_eq!(snapshot(&home)?, before);
        assert_eq!(store.account()?, Some(account.clone()));
        assert!(store.load()?.spaces.is_empty());
        assert!(store.load()?.bindings.is_empty());
    }
    Ok(())
}

#[dialog_common::test]
async fn an_already_persisted_ordinary_import_can_resume_only_by_space() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let invite = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
    )
    .await?;
    let preflight = tonk_cli::invite::preflight(&invite.url).await?;
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;
    #[cfg(target_os = "macos")]
    let profile_parent = home.join("Library/Application Support/dialog");
    #[cfg(not(target_os = "macos"))]
    let profile_parent = home.join("data/dialog");
    std::fs::create_dir_all(&profile_parent)?;
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let root = store.canonical_site("legacy");
    let config = tonk_cli::site::SiteConfig {
        profile_name: tonk_cli::site::PROFILE_NAME.into(),
        profile_directory: Directory::At(profile_parent.to_string_lossy().into_owned()),
        require_account: true,
        provision_account_spaces: true,
        account_store: store.clone(),
    };
    tonk_cli::invite::claim(&root, &invite.url, config).await?;
    tonk_cli::join::OrdinaryState::legacy(
        &preflight.invitation,
        &project,
        true,
        None,
        false,
        tonk_cli::join::OrdinaryPhase::Ready,
    )?
    .save(&root)?;
    tonk_cli::space::register_existing_bound(&store, "legacy", &root, &project)?;

    let mut rejected = cli(&home, &project);
    rejected.args(["join", &invite.url]);
    let rejected = run(rejected).await?;
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains(WRONG_KIND));

    let mut resume = cli(&home, &home);
    resume.args(["--space", "legacy", "join"]);
    let resumed = run(resume).await?;
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("Joined space 'legacy'"));
    assert_eq!(
        store
            .load()?
            .bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([project.canonicalize()?])
    );
    Ok(())
}
