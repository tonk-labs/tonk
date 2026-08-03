//! Real-browser account-panel and UI↔CLI roundtrip tests.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use std::path::PathBuf;
    use std::process::{ExitStatus, Stdio};
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow};
    use tempfile::TempDir;
    use thirtyfour::prelude::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::process::{Child, Command};

    use crate::helpers::{TestEnvironment, driver_with_prf};

    const EMAIL: &str = "person@example.com";

    async fn element(driver: &WebDriver, selector: &str) -> Result<WebElement> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(element) => return Ok(element),
                Err(error) if tokio::time::Instant::now() < deadline => {
                    let _ = error;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("timed out waiting for `{selector}`"));
                }
            }
        }
    }

    async fn wait_for_text(driver: &WebDriver, selector: &str, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && found.text().await.ok().as_deref() == Some(expected)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to equal {expected:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_for_text_containing(
        driver: &WebDriver,
        selector: &str,
        expected: &str,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && found.text().await?.contains(expected)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to contain {expected:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn captured_code(env: &TestEnvironment, email: &str) -> Result<String> {
        let endpoint = env.account_service.join("_test/emails")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let inbox: Vec<serde_json::Value> =
                reqwest::get(endpoint.clone()).await?.json().await?;
            if let Some(code) = inbox.iter().rev().find_map(|entry| {
                (entry["address"].as_str() == Some(email))
                    .then(|| entry["code"].as_str().map(str::to_owned))
                    .flatten()
            }) {
                return Ok(code);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for a code for {email}"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub(crate) async fn sign_up(
        driver: &WebDriver,
        env: &TestEnvironment,
        email: &str,
    ) -> Result<()> {
        driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                navigator.serviceWorker.ready.then(() => {
                    if (navigator.serviceWorker.controller) {
                        done(true);
                    } else {
                        navigator.serviceWorker.addEventListener(
                            "controllerchange",
                            () => done(true),
                            { once: true },
                        );
                    }
                }).catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(driver, "tonk-account[data-mode=\"choice\"]").await?;
        element(driver, "#account-choose-create")
            .await?
            .click()
            .await?;
        element(driver, "#account-email")
            .await?
            .send_keys(email)
            .await?;
        element(driver, "#account-send-code").await?.click().await?;
        element(driver, "tonk-account[data-mode=\"verify\"]").await?;

        let code = captured_code(env, email).await?;
        element(driver, "#account-code")
            .await?
            .send_keys(code)
            .await?;
        element(driver, "#account-create-submit")
            .await?
            .click()
            .await?;
        if let Err(wait_error) = element(driver, "tonk-account[data-mode=\"success\"]").await {
            let host = element(driver, "tonk-account").await?;
            let mode = host.attr("data-mode").await?.unwrap_or_default();
            let error = element(driver, "#account-error").await?.text().await?;
            let working = element(driver, "#account-working").await?.text().await?;
            return Err(wait_error).context(format!(
                "account creation stopped in mode {mode:?}; error={error:?}; status={working:?}"
            ));
        }
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_up_through_the_account_panels(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        element(&driver, "#account-manage-devices")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"devices\"]").await?;
        wait_for_text_containing(&driver, "#account-device-list", "This browser").await?;

        driver.quit().await?;
        Ok(())
    }

    fn tonk_bin() -> PathBuf {
        let path = std::env::var_os("TONK_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join("target/debug/tonk")
            });
        assert!(
            path.is_file(),
            "tonk binary not found at {}; build it with `cargo build -p tonk-cli` or set TONK_BIN",
            path.display()
        );
        path
    }

    fn tonk_command(profile: &TempDir) -> Command {
        let mut command = Command::new(tonk_bin());
        command
            .current_dir(profile.path())
            .env("HOME", profile.path())
            .env("XDG_DATA_HOME", profile.path().join("data"))
            .env("TONK_SPOTS_STATE", profile.path().join("spots"))
            .env("TONK_TELEMETRY_STATE", profile.path().join("telemetry"))
            .env("TONK_UPDATE_STATE", profile.path().join("update"))
            .env("TONK_NO_UPDATE_CHECK", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NO_PROXY", "127.0.0.1,localhost,tonk.spot")
            .env_remove("TONK_TELEMETRY")
            .env_remove("TONK_SPOT")
            .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT");
        command
    }

    struct CliOutput {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    }

    async fn run_cli(profile: &TempDir, args: &[String]) -> Result<CliOutput> {
        let output = tonk_command(profile).args(args).output().await?;
        Ok(CliOutput {
            status: output.status,
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }

    async fn finish_link(
        child: &mut Child,
        stdout: &mut BufReader<tokio::process::ChildStdout>,
        stderr: &mut tokio::process::ChildStderr,
        prefix: String,
    ) -> Result<CliOutput> {
        let completion = async {
            let mut stdout_rest = String::new();
            let mut stderr_text = String::new();
            let (status, _, _) = tokio::try_join!(
                child.wait(),
                stdout.read_to_string(&mut stdout_rest),
                stderr.read_to_string(&mut stderr_text),
            )?;
            Ok::<_, std::io::Error>((status, stdout_rest, stderr_text))
        };
        match tokio::time::timeout(Duration::from_secs(60), completion).await {
            Ok(result) => {
                let (status, stdout_rest, stderr) = result?;
                Ok(CliOutput {
                    status,
                    stdout: format!("{prefix}{stdout_rest}"),
                    stderr,
                })
            }
            Err(_) => {
                child.kill().await?;
                Err(anyhow!("timed out waiting for `tonk account link`"))
            }
        }
    }

    struct LinkedCli {
        profile: TempDir,
        link: CliOutput,
    }

    async fn link_cli(driver: &WebDriver, env: &TestEnvironment) -> Result<LinkedCli> {
        let profile = tempfile::tempdir()?;
        let mut command = tonk_command(&profile);
        command
            .args([
                "account",
                "link",
                "--name",
                "e2e terminal",
                "--no-open",
                "--service-url",
                env.account_service.as_str(),
                "--account-url",
                env.tonk_web.join("account/link")?.as_str(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().context("CLI stdout was not piped")?);
        let mut stderr = child.stderr.take().context("CLI stderr was not piped")?;

        let mut heading = String::new();
        let mut url_line = String::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            stdout.read_line(&mut heading).await?;
            stdout.read_line(&mut url_line).await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .context("timed out waiting for the CLI approval URL")??;
        assert_eq!(heading.trim_end(), "Open this URL to approve the device:");
        let approval_url = url::Url::parse(url_line.trim())?;
        assert_eq!(approval_url.path(), "/account/link");
        assert!(
            approval_url
                .fragment()
                .is_some_and(|secret| !secret.is_empty())
        );

        driver.goto(approval_url.as_str()).await?;
        element(driver, "tonk-account[data-mode=\"handoff\"]").await?;
        wait_for_text(driver, "#account-handoff-name", "e2e terminal").await?;
        let handoff_did = element(driver, "#account-handoff-did")
            .await?
            .text()
            .await?;
        assert!(!handoff_did.is_empty());
        element(driver, "#account-handoff-submit")
            .await?
            .click()
            .await?;
        element(driver, "tonk-account[data-mode=\"success\"]").await?;

        let mut outcome_line = String::new();
        match tokio::time::timeout(Duration::from_secs(20), stdout.read_line(&mut outcome_line))
            .await
        {
            Ok(result) => {
                result?;
                assert_eq!(outcome_line.trim_end(), "linked");
            }
            Err(_) => {
                child.kill().await?;
                return Err(anyhow!(
                    "CLI consumed the handoff but did not finish account-state hydration"
                ));
            }
        }
        let prefix = format!("{heading}{url_line}{outcome_line}");
        let link = finish_link(&mut child, &mut stdout, &mut stderr, prefix).await?;
        assert!(link.status.success(), "link failed: {}", link.stderr);
        assert!(link.stdout.contains("linked\nroot: did:key:"));
        assert!(link.stdout.contains("device: did:key:"));
        assert!(
            link.stdout.contains("account state: ready")
                || link.stdout.contains("account state: unhydrated")
        );

        Ok(LinkedCli { profile, link })
    }

    async fn devices(profile: &TempDir, env: &TestEnvironment) -> Result<CliOutput> {
        run_cli(
            profile,
            &[
                "account".to_string(),
                "devices".to_string(),
                "--service-url".to_string(),
                env.account_service.to_string(),
            ],
        )
        .await
    }

    fn did_for_device<'a>(output: &'a str, name: &str) -> Option<&'a str> {
        output.lines().find_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields.len() == 3 && fields[1] == name)
                .then(|| fields[2].trim_end_matches(" (this device)"))
        })
    }

    #[dialog_common::test]
    async fn it_links_the_cli_through_the_browser_handoff(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let linked = link_cli(&driver, &env).await?;

        let status = run_cli(
            &linked.profile,
            &["account".to_string(), "status".to_string()],
        )
        .await?;
        assert!(status.status.success(), "status failed: {}", status.stderr);
        assert!(status.stdout.contains("signed in: yes"));
        let provider = status
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("provider: "))
            .context("status output omitted the provider")?;
        assert_eq!(url::Url::parse(provider)?, env.account_service);
        assert!(linked.link.stdout.contains("linked"));

        let devices = devices(&linked.profile, &env).await?;
        assert!(
            devices.status.success(),
            "devices failed: {}",
            devices.stderr
        );
        assert!(devices.stdout.contains("active\tThis browser\t"));
        assert!(devices.stdout.contains("active\te2e terminal\t"));
        assert!(devices.stdout.contains(" (this device)"));

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_revokes_the_cli_device_from_the_browser(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let linked = link_cli(&driver, &env).await?;
        let listed = devices(&linked.profile, &env).await?;
        assert!(listed.status.success(), "devices failed: {}", listed.stderr);
        let cli_did = did_for_device(&listed.stdout, "e2e terminal")
            .context("CLI device was absent from the account device list")?
            .to_string();

        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        element(&driver, "#account-manage-devices")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"devices\"]").await?;
        let selector = format!("#account-device-list button[data-revoke=\"{cli_did}\"]");
        element(&driver, &selector).await?.click().await?;
        driver.accept_alert().await?;
        wait_for_text_containing(&driver, "#account-working", "Device revoked").await?;

        let rejected = devices(&linked.profile, &env).await?;
        assert_eq!(rejected.status.code(), Some(4), "{}", rejected.stderr);
        assert!(
            rejected.stderr.contains("403 Forbidden"),
            "{}",
            rejected.stderr
        );
        assert!(rejected.stderr.contains("\"code\":\"FORBIDDEN\""));
        assert!(
            rejected
                .stderr
                .contains("device is not an active member of this account")
        );

        driver.quit().await?;
        Ok(())
    }
}
