//! Typed invitation routing before `tonk join` mutates local state.

mod common;

use anyhow::Result;
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::time::{Duration, SystemTime, Timestamp};
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tonk_cli::join::PreparedInvitation;
use tonk_invite::connection::{AgentInvite, candidate_build_scopes};
use tonk_schema::prelude::DidExt as _;

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

async fn agent_link(base: &str, expired: bool) -> Result<String> {
    let owner = Signer::from(Ed25519Signer::import(&[71; 32]).await?);
    let seed = [72; 32];
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
        let grant = DelegationBuilder::new()
            .issuer(owner.clone())
            .audience(&recipient.did())
            .subject(scope.subject.clone())
            .command(scope.command.0.clone())
            .policy(scope.policy())
            .expiration(expiration)
            .meta(tonk_invite::home_address_meta(&remote))
            .try_build()
            .await?;
        chains.push(DelegationChain::new(grant));
    }
    Ok(
        AgentInvite::new(seed, chains, &scopes, &remote, validation_time)
            .await?
            .to_url(base)?,
    )
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

#[dialog_common::test]
async fn it_routes_and_validates_full_links_without_fallback() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let ordinary = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
    )
    .await?;
    match tonk_cli::join::prepare(&ordinary.url).await? {
        PreparedInvitation::Ordinary(prepared) => {
            assert_eq!(prepared.invitation().subject.0, ordinary.subject.this());
        }
        PreparedInvitation::Agent(_) => panic!("ordinary invitation reached the agent parser"),
    }

    let agent = agent_link("https://carrier.example.test/join", false).await?;
    match tonk_cli::join::prepare(&agent).await? {
        PreparedInvitation::Agent(prepared) => {
            assert_eq!(
                prepared.hint().remote.as_str(),
                "https://access.example.test/ucan/"
            );
        }
        PreparedInvitation::Ordinary(_) => panic!("agent invitation reached the ordinary parser"),
    }

    let mut mixed = url::Url::parse(&agent)?;
    mixed
        .query_pairs_mut()
        .append_pair("access", "ordinary-proof");
    let error = tonk_cli::join::prepare(mixed.as_str()).await.unwrap_err();
    assert!(format!("{error:#}").contains("ambiguous_invitation"));
    secret_free(&error, &["ordinary-proof", "tonk-agent-v"]);

    let unsupported = agent.replace("tonk-agent-v2=", "tonk-agent-v99=");
    let error = tonk_cli::join::prepare(&unsupported).await.unwrap_err();
    assert!(format!("{error:#}").contains("connection_unsupported_version"));
    secret_free(&error, &["tonk-agent-v99"]);

    let malformed = "https://carrier.example.test/join?access=not-base58#never-print-secret";
    let error = tonk_cli::join::prepare(malformed).await.unwrap_err();
    assert!(format!("{error:#}").contains("invalid invite"));
    secret_free(&error, &["not-base58", "never-print-secret"]);

    let expired = agent_link("https://carrier.example.test/join", true).await?;
    let error = tonk_cli::join::prepare(&expired).await.unwrap_err();
    assert!(format!("{error:#}").contains("Expired at"), "{error:#}");
    secret_free(&error, &["tonk-agent-v2"]);
    Ok(())
}

#[dialog_common::test]
async fn shortcuts_resolve_once_before_routing_both_formats() -> Result<()> {
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
                let location = location.read().await.clone();
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, HeaderValue::from_str(&location).unwrap())],
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
    let ordinary_fragment = ordinary_url.fragment().unwrap();
    let mut ordinary_location = ordinary_url.clone();
    ordinary_location.set_fragment(None);
    *location.write().await = ordinary_location.to_string();
    let short = format!("{base}/@/{}#{ordinary_fragment}", "a".repeat(64));
    assert!(matches!(
        tonk_cli::join::prepare(&short).await?,
        PreparedInvitation::Ordinary(_)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    let agent = agent_link(&format!("{base}/join"), false).await?;
    let agent_url = url::Url::parse(&agent)?;
    let agent_fragment = agent_url.fragment().unwrap();
    let mut agent_location = agent_url.clone();
    agent_location.set_fragment(None);
    *location.write().await = agent_location.to_string();
    let short = format!("{base}/@/{}#{agent_fragment}", "b".repeat(64));
    assert!(matches!(
        tonk_cli::join::prepare(&short).await?,
        PreparedInvitation::Agent(_)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 2);

    server.abort();
    Ok(())
}

#[dialog_common::test]
async fn ordinary_local_join_uses_normal_storage_and_no_agent_marker() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let invite = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
    )
    .await?;
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;
    let mut command = cli(&home, &project);
    command.args(["join", &invite.url, "--name", "shared"]);
    let output = run(command).await?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Joined space 'shared'"), "{stdout}");
    assert!(stdout.contains("no sync remote"), "{stdout}");
    assert!(!stdout.contains("Agent connection confirmed"));
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let registry = store.load()?;
    let entry = &registry.spaces["shared"];
    assert!(entry.connection.is_none());
    assert!(!entry.site.join(tonk_cli::connections::MARKER_FILE).exists());
    assert_eq!(
        registry.bindings.get(&project.canonicalize()?),
        Some(&"shared".to_owned())
    );
    let recovery = std::fs::read_to_string(entry.site.join("ordinary-join.json"))?;
    assert!(!recovery.contains(&invite.url));
    assert!(!recovery.contains(invite.url.split('#').next_back().unwrap()));
    Ok(())
}

#[dialog_common::test]
async fn ordinary_targeted_join_requires_an_available_exact_recipient() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let recipient = common::TestSite::new().await?;
    let recipient_root = tonk_cli::site::member_did(&recipient.site).await?;
    let invite = tonk_cli::invite::mint_targeted(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        None,
        recipient_root.as_str(),
    )
    .await?;
    let prepared = match tonk_cli::join::prepare(&invite.url).await? {
        PreparedInvitation::Ordinary(prepared) => prepared,
        PreparedInvitation::Agent(_) => panic!("targeted ordinary invite reached agent parsing"),
    };
    tonk_cli::join::ensure_ordinary_recipient(&prepared, &recipient.config).await?;

    let unrelated = common::TestSite::new().await?;
    let error = tonk_cli::join::ensure_ordinary_recipient(&prepared, &unrelated.config)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("invitation_recipient_mismatch"));

    let empty = tempfile::tempdir()?;
    let project = empty.path().join("project");
    std::fs::create_dir_all(&project)?;
    let mut command = cli(empty.path(), &project);
    command.args(["join", &invite.url, "--name", "must-not-exist"]);
    let output = run(command).await?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invitation_recipient_mismatch"));
    assert!(!empty.path().join("state/spaces").exists());
    Ok(())
}

#[dialog_common::test]
async fn failed_hosted_join_retains_alias_offline_edits_and_resume_state() -> Result<()> {
    let issuer = common::TestSite::new().await?;
    let invite = tonk_cli::invite::mint(
        &issuer.site,
        Some("https://carrier.example.test/join"),
        Some("http://127.0.0.1:9/ucan/"),
    )
    .await?;
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&project)?;
    let mut command = cli(&home, &project);
    command.args(["join", &invite.url, "--name", "pending"]);
    let output = run(command).await?;
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Joined space"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Resume with `tonk --space pending join`"),
        "{stderr}"
    );
    assert!(!stderr.contains(&invite.url));
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let registry = store.load()?;
    assert_eq!(registry.spaces.len(), 1);
    let root = registry.spaces["pending"].site.clone();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("ordinary-join.json"))?)?;
    assert_eq!(state["phase"], "pull_pending");

    let document = "attribute!: &offline-note\n  description: Offline note\n  the: test.pending/offline-note\n  as: text\n  cardinality: one\n";
    let mut edit = cli(&home, &project);
    edit.args(["--space", "pending", "eval", "-c", document, "--no-sync"]);
    let edited = run(edit).await?;
    assert!(
        edited.status.success(),
        "{}",
        String::from_utf8_lossy(&edited.stderr)
    );
    let mut resume = cli(&home, &project);
    resume.args(["--space", "pending", "join"]);
    let resumed = run(resume).await?;
    assert!(!resumed.status.success());
    assert!(!String::from_utf8_lossy(&resumed.stdout).contains("Joined space"));
    assert_eq!(store.load()?.spaces.len(), 1);
    let mut show = cli(&home, &project);
    show.args(["--space", "pending", "show", "offline-note", "--json"]);
    let shown = run(show).await?;
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    Ok(())
}

#[dialog_common::test]
async fn ordinary_advisory_names_get_collision_safe_local_aliases() -> Result<()> {
    async fn named_invite() -> Result<tonk_cli::invite::InviteOutcome> {
        let issuer = common::TestSite::new().await?;
        issuer
            .site
            .branch()
            .await?
            .handle()
            .transaction()
            .assert(tonk_schema::RepositoryName {
                this: issuer.site.repository.did().this(),
                name: tonk_schema::domain::repo::Name("Shared Garden".into()),
            })
            .commit()
            .publish()
            .perform(&issuer.site.operator)
            .await?;
        Ok(tonk_cli::invite::mint(
            &issuer.site,
            Some("https://carrier.example.test/join"),
            None,
        )
        .await?)
    }

    let first = named_invite().await?;
    let second = named_invite().await?;
    let temp = tempfile::tempdir()?;
    let home = temp.path().join("home");
    let first_project = temp.path().join("first-project");
    let second_project = temp.path().join("second-project");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&first_project)?;
    std::fs::create_dir_all(&second_project)?;
    for (project, invite, expected) in [
        (&first_project, &first.url, "shared-garden"),
        (&second_project, &second.url, "shared-garden-2"),
    ] {
        let mut command = cli(&home, project);
        command.args(["join", invite]);
        let output = run(command).await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("Joined space '{expected}'"))
        );
    }
    let store = tonk_cli::space::SpaceStore::at(home.join("state"));
    let registry = store.load()?;
    assert!(registry.spaces.contains_key("shared-garden"));
    assert!(registry.spaces.contains_key("shared-garden-2"));
    assert_eq!(
        registry.bindings[&first_project.canonicalize()?],
        "shared-garden"
    );
    assert_eq!(
        registry.bindings[&second_project.canonicalize()?],
        "shared-garden-2"
    );
    Ok(())
}
