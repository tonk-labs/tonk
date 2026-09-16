//! Real-process request persistence, authenticated polling, timeout and cancellation.
use anyhow::{Context, Result};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

fn cli(home: &Path) -> Command {
    let binary = std::env::var_os("NEXTEST_BIN_EXE_tonk")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_tonk").into());
    let mut command = Command::new(binary);
    command
        .current_dir(home)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("TONK_SPACES_STATE", home.join("state"))
        .env("TONK_TELEMETRY_STATE", home.join("telemetry"))
        .env("TONK_UPDATE_STATE", home.join("update"))
        .env("TONK_NO_UPDATE_CHECK", "1")
        .env("DO_NOT_TRACK", "1")
        .env_remove("TONK_SPACE")
        .env_remove("TONK_CONNECTION_ORIGIN");
    command
}
async fn server() -> Result<tonk_access_service::helpers::AccessServer> {
    let s3 =
        dialog_remote_s3::helpers::LocalS3::start_with_auth("test", "test", &["terminals"]).await?;
    tonk_access_service::helpers::AccessServer::start(
        s3,
        "terminals",
        "test",
        "test",
        Some(tonk_worker_api::DeploymentConfig::default()),
        None,
        None,
    )
    .await
}
fn record(home: &Path) -> Result<(std::path::PathBuf, serde_json::Value)> {
    let root = std::fs::read_dir(home.join("state/terminal-links"))?
        .next()
        .context("no terminal request persisted")??
        .path();
    let value = serde_json::from_slice(&std::fs::read(root.join("request.json"))?)?;
    Ok((root, value))
}
async fn output(mut command: Command) -> Result<std::process::Output> {
    Ok(tokio::time::timeout(
        Duration::from_secs(20),
        tokio::task::spawn_blocking(move || command.output()),
    )
    .await???)
}

#[tokio::test]
async fn terminal_timeout_polls_real_service_and_preserves_unrelated_account() -> Result<()> {
    let server = server().await?;
    let home = tempfile::tempdir()?;
    let store = tonk_cli::space::SpaceStore::at(home.path().join("state"));
    store.set_account(Some(tonk_cli::space::AccountRecord::new(
        "unrelated-account",
    )))?;
    let previous = std::fs::read(store.registry_path())?;
    let mut command = cli(home.path());
    command.args([
        "link",
        "--no-open",
        "--via",
        &server.endpoint,
        "--label",
        "Headless terminal",
        "--timeout",
        "1",
    ]);
    let result = output(command).await?;
    assert!(!result.status.success());
    let stdout = String::from_utf8_lossy(&result.stdout);
    let url = stdout
        .lines()
        .find(|line| line.starts_with("http"))
        .context("approval URL not printed alone")?;
    let request = tonk_invite::terminal::LinkRequest::from_url(
        url,
        dialog_ucan_core::time::Timestamp::now().to_unix(),
    )
    .await?;
    let (root, journal) = record(home.path())?;
    assert_eq!(journal["state"], "expired");
    assert_eq!(
        hex::decode(journal["request"].as_str().unwrap())?,
        request.bytes()
    );
    assert!(!stdout.contains(&hex::encode(std::fs::read(root.join("recipient.key"))?)));
    assert_eq!(std::fs::read(store.registry_path())?, previous);
    let mut resume = cli(home.path());
    resume.args(["link", "--resume", &request.id(), "--no-open"]);
    let result = output(resume).await?;
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Expired"));
    assert_eq!(std::fs::read(store.registry_path())?, previous);
    Ok(())
}

#[tokio::test]
async fn terminal_killed_request_reuses_key_then_cancels_without_remote_write() -> Result<()> {
    let server = server().await?;
    let home = tempfile::tempdir()?;
    let mut command = cli(home.path());
    command
        .args(["link", "--no-open", "--via", &server.endpoint])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let request_url = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::task::spawn_blocking(move || -> Result<String> {
            for line in BufReader::new(stdout).lines() {
                let line = line?;
                if line.starts_with("http") {
                    return Ok(line);
                }
            }
            anyhow::bail!("CLI closed before printing request URL")
        }),
    )
    .await???;
    child.kill()?;
    child.wait()?;
    let request = tonk_invite::terminal::LinkRequest::from_url(
        &request_url,
        dialog_ucan_core::time::Timestamp::now().to_unix(),
    )
    .await?;
    let (root, before) = record(home.path())?;
    let key = std::fs::read(root.join("recipient.key"))?;
    assert_eq!(before["state"], "pending");
    let mut cancel = cli(home.path());
    cancel.args(["link", "--cancel", &request.id()]);
    let result = output(cancel).await?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let (_, after) = record(home.path())?;
    assert_eq!(after["state"], "cancelled");
    assert_eq!(after["request"], before["request"]);
    assert_eq!(std::fs::read(root.join("recipient.key"))?, key);
    assert!(!home.path().join("state/spaces.json").exists());
    Ok(())
}
