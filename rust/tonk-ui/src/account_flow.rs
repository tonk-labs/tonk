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
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::prelude::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio::process::{Child, Command};

    use crate::helpers::{TestEnvironment, driver_with_prf, driver_with_prf_authenticator};

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

    /// Click the element `selector` names, re-finding it if the DOM
    /// replaced it in between.
    ///
    /// `element` retries the *find*, but a list that re-renders between
    /// resolving the handle and clicking it invalidates the handle —
    /// WebDriver answers "stale element reference". The profile
    /// switcher does exactly that: activating a profile re-renders the
    /// list the button lives in. Re-finding on staleness is the fix;
    /// sleeping before the click would only make the race rarer.
    async fn click(driver: &WebDriver, selector: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let found = element(driver, selector).await?;
            match found.click().await {
                Ok(()) => return Ok(()),
                Err(error) if tokio::time::Instant::now() < deadline => {
                    let _ = error;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("timed out clicking `{selector}`"));
                }
            }
        }
    }

    async fn click_in_guest(driver: &WebDriver, selector: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let mut paths = vec![Vec::<u16>::new()];
            for _depth in 0..5 {
                let current = std::mem::take(&mut paths);
                for path in current {
                    driver.enter_default_frame().await?;
                    let mut reachable = true;
                    for index in &path {
                        if driver.enter_frame(*index).await.is_err() {
                            reachable = false;
                            break;
                        }
                    }
                    if !reachable {
                        continue;
                    }
                    if let Ok(element) = driver.find(By::Css(selector.to_string())).await {
                        element.click().await?;
                        driver.enter_default_frame().await?;
                        return Ok(());
                    }
                    let count = driver.find_all(By::Css("iframe".to_string())).await?.len();
                    for index in 0..count.min(u16::MAX as usize) {
                        let mut child = path.clone();
                        child.push(index as u16);
                        paths.push(child);
                    }
                }
            }
            driver.enter_default_frame().await?;
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for {selector} in guest frames"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
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
            // The text read is fallible for the same reason the find is:
            // a list that re-renders between the two invalidates the
            // handle ("stale element reference"). Treat that as "not yet"
            // and go round again — propagating it aborts the wait on a
            // race the wait exists to absorb.
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && let Ok(text) = found.text().await
                && text.contains(expected)
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

    /// The latest activation link the access service captured for `email`.
    async fn activation_link(env: &TestEnvironment, email: &str) -> Result<String> {
        let endpoint = env.access_service.join("_test/emails")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let inbox: Vec<(String, String)> = reqwest::get(endpoint.clone()).await?.json().await?;
            if let Some((_, link)) = inbox.iter().rev().find(|(to, _)| to == email) {
                return Ok(link.clone());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for an activation email for {email}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn credential_count(driver: &WebDriver, authenticator_id: &str) -> Result<usize> {
        let devtools = ChromeDevTools::new(driver.handle.clone());
        let result = devtools
            .execute_cdp_with_params(
                "WebAuthn.getCredentials",
                serde_json::json!({ "authenticatorId": authenticator_id }),
            )
            .await?;
        result["credentials"]
            .as_array()
            .map(Vec::len)
            .ok_or_else(|| anyhow!("Chrome omitted the virtual authenticator credentials"))
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

        wait_for_text_containing(&driver, "#account-email-value", EMAIL).await?;
        // Creation mints the first custody passkey in the same ceremony
        // that generates and seals the secret, so the dashboard
        // describes it immediately.
        wait_for_text_containing(&driver, "#account-passkey-device-value", "Chrome on ").await?;
        wait_for_text_containing(&driver, "#account-device-list", "Chrome on ").await?;
        let first = get_json(&driver, "/api/account/summary").await?;
        let again = get_json(&driver, "/api/account/summary").await?;
        let first = successful_body("account summary", &first);
        let again = successful_body("account summary", &again);
        assert!(
            !first["passkey"].is_null(),
            "creation records passkey facts: {first}"
        );
        assert_eq!(
            first["passkey"], again["passkey"],
            "a second read must not rewrite the recorded creation facts"
        );

        // Signup enrolled the account as a customer: the dashboard names
        // the pending activation, and the emailed link completes it from
        // this (or any) device without a key.
        wait_for_text_containing(&driver, "#account-activation-notice", "activation pending")
            .await?;
        let link = activation_link(&env, EMAIL).await?;
        driver.goto(&link).await?;
        element(&driver, "#activate-accept").await?.click().await?;
        element(&driver, "#activate-done").await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_reports_an_existing_email_and_recovers_with_another_address(
        env: TestEnvironment,
    ) -> Result<()> {
        let existing_email = "existing@example.com";
        let available_email = "available@example.com";

        let creator = driver_with_prf(&env).await?;
        sign_up(&creator, &env, existing_email).await?;
        creator.quit().await?;

        let (driver, authenticator_id) = driver_with_prf_authenticator(&env).await?;
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
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        element(&driver, "#account-choose-create")
            .await?
            .click()
            .await?;
        element(&driver, "#account-email")
            .await?
            .send_keys(existing_email)
            .await?;
        element(&driver, "#account-create-submit")
            .await?
            .click()
            .await?;

        // The conflict surfaces at signed account creation — after the
        // custody passkey exists. That ordering is deliberate: an
        // availability probe without a verified code would let anyone
        // enumerate registered emails, so the failed attempt's cost is
        // one orphaned passkey in the authenticator.
        wait_for_text(
            &driver,
            "#account-error",
            "an account already exists for this email address",
        )
        .await?;
        assert_eq!(credential_count(&driver, &authenticator_id).await?, 1);

        let email = element(&driver, "#account-email").await?;
        email.clear().await?;
        email.send_keys(available_email).await?;
        element(&driver, "#account-create-submit")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        assert_eq!(
            credential_count(&driver, &authenticator_id).await?,
            2,
            "each creation attempt mints exactly one custody passkey"
        );

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
            .env("NO_PROXY", "127.0.0.1,localhost,tonk.network")
            .env_remove("TONK_TELEMETRY")
            .env_remove("TONK_SPACE")
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
        link_cli_with(driver, env, false).await
    }

    async fn link_cli_with(
        driver: &WebDriver,
        env: &TestEnvironment,
        register_first: bool,
    ) -> Result<LinkedCli> {
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
                "--via",
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
        let query: std::collections::HashMap<String, String> =
            approval_url.query_pairs().into_owned().collect();
        let audience = query
            .get("audience")
            .context("approval URL names no audience")?
            .clone();
        assert!(audience.starts_with("did:key:"));
        assert!(
            query
                .get("callback")
                .is_some_and(|callback| callback.starts_with("http://127.0.0.1")),
            "approval URL must carry the loopback callback"
        );

        driver.goto(approval_url.as_str()).await?;
        if register_first {
            // A browser with no account yet registers before approving:
            // the link page opens on the signup panels, and the ceremony
            // that creates and enrolls the account flows straight into
            // the approval it was interrupted by.
            element(driver, "tonk-account[data-mode=\"choice\"]").await?;
            element(driver, "#account-choose-create")
                .await?
                .click()
                .await?;
            element(driver, "#account-email")
                .await?
                .send_keys(EMAIL)
                .await?;
            element(driver, "#account-create-submit")
                .await?
                .click()
                .await?;
        }
        element(driver, "tonk-account[data-mode=\"handoff\"]").await?;
        wait_for_text(driver, "#account-handoff-name", "e2e terminal").await?;
        let handoff_did = element(driver, "#account-handoff-did")
            .await?
            .text()
            .await?;
        assert_eq!(handoff_did, audience);
        element(driver, "#account-handoff-submit")
            .await?
            .click()
            .await?;
        // The callback answers the form POST with a redirect back to the
        // account page, which renders the outcome in its own styling.
        element(driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text(
            driver,
            "#account-success-message",
            "Command-line device linked.",
        )
        .await?;

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
                    "CLI received the grant but did not finish account-state hydration"
                ));
            }
        }
        let prefix = format!("{heading}{url_line}{outcome_line}");
        let link = finish_link(&mut child, &mut stdout, &mut stderr, prefix).await?;
        assert!(link.status.success(), "link failed: {}", link.stderr);
        assert!(link.stdout.contains("linked\naccount: did:key:"));
        assert!(link.stdout.contains("device: did:key:"));
        assert!(
            link.stdout.contains("status: synced")
                || link.stdout.contains("status: waiting for first sync")
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

    async fn post_json(
        driver: &WebDriver,
        path: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                fetch(arguments[0], {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify(arguments[1]),
                }).then(async response => done({
                    status: response.status,
                    body: await response.json(),
                })).catch(error => done({ error: String(error) }));
                "#,
                vec![serde_json::json!(path), body],
            )
            .await?;
        Ok(result.json().clone())
    }

    async fn get_json(driver: &WebDriver, path: &str) -> Result<serde_json::Value> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                fetch(arguments[0]).then(async response => done({
                    status: response.status,
                    body: await response.json(),
                })).catch(error => done({ error: String(error) }));
                "#,
                vec![serde_json::json!(path)],
            )
            .await?;
        Ok(result.json().clone())
    }

    fn successful_body<'a>(
        operation: &str,
        result: &'a serde_json::Value,
    ) -> &'a serde_json::Value {
        assert!(
            result.get("error").is_none(),
            "{operation} transport failed: {result}"
        );
        assert!(
            result["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status)),
            "{operation} failed: {result}"
        );
        &result["body"]
    }

    fn profile_space_keys(body: &serde_json::Value) -> Vec<String> {
        body["space"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry["key"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn wait_for_account_space(
        driver: &WebDriver,
        subject: &str,
        membership: &str,
        visibility: Option<&str>,
    ) -> Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let response = get_json(driver, "/api/account/spaces").await?;
            if response["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status))
                && let Some(row) = response["body"].as_array().and_then(|rows| {
                    rows.iter().find(|row| {
                        row["subject"] == subject
                            && row["membership"] == membership
                            && visibility.is_none_or(|expected| row["visibility"] == expected)
                    })
                })
            {
                return Ok(row.clone());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for account space {subject} membership={membership} visibility={visibility:?}: {response}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn create_connected_space(
        driver: &WebDriver,
        env: &TestEnvironment,
        name: &str,
    ) -> Result<String> {
        let created = post_json(
            driver,
            "/api/spaces",
            serde_json::json!({
                "name": name,
                "remote": env.tonk_web.join("ucan/")?,
                "revocation_url": env.account_service.join("revocations")?,
                "template": "blank",
            }),
        )
        .await?;
        let subject = successful_body("create connected space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();
        let pushed = post_json(
            driver,
            &format!("/api/repository/{subject}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push connected space", &pushed);
        let row = wait_for_account_space(driver, &subject, "active", Some("visible")).await?;
        assert_eq!(row["enrollment"], "connected");
        assert!(row["confirmedRevision"].is_string());
        Ok(subject)
    }

    fn did_for_device<'a>(output: &'a str, name: &str) -> Option<&'a str> {
        output.lines().find_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields.len() == 3 && fields[1] == name)
                .then(|| fields[2].trim_end_matches(" (this device)"))
        })
    }

    #[dialog_common::test]
    async fn it_backs_up_a_claimed_spot_for_another_account_device(
        env: TestEnvironment,
    ) -> Result<()> {
        let creator = driver_with_prf(&env).await?;
        sign_up(&creator, &env, "creator@example.com").await?;

        let created = post_json(
            &creator,
            "/api/spaces",
            serde_json::json!({
                "name": "Shared Garden",
                "remote": env.tonk_web.join("ucan/")?,
                "revocation_url": env.account_service.join("revocations")?,
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create synced spot", &created)["key"]
            .as_str()
            .context("create response omitted the spot key")?
            .to_string();
        let pushed = post_json(
            &creator,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push synced spot", &pushed);
        let invited = post_json(
            &creator,
            &format!("/api/repository/{key}/invite"),
            serde_json::json!({ "baseUrl": env.tonk_web.join("join")? }),
        )
        .await?;
        let invite_url = successful_body("mint invite", &invited)["url"]
            .as_str()
            .context("invite response omitted its URL")?
            .to_string();
        creator.quit().await?;

        let claimer = driver_with_prf(&env).await?;
        sign_up(&claimer, &env, "claimer@example.com").await?;
        let visited = post_json(
            &claimer,
            "/api/profile/visit",
            serde_json::json!({ "url": invite_url }),
        )
        .await?;
        successful_body("visit shared spot", &visited);
        let promoted = post_json(
            &claimer,
            &format!("/api/repository/{key}/membership"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("promote guest membership", &promoted);

        // The account directory is the backup now: link a CLI as a
        // second device of the claimer's account and read the claimed
        // spot back out of the synced account DB — the real
        // cross-device path, not a service-side artifact store.
        let second_device = link_cli_with(&claimer, &env, false).await?;
        // The browser pushes the directory facts on its next sync
        // drain, so a freshly linked device may pull before they land.
        // `spots` pulls best-effort on every run; poll until the
        // recording arrives — the assertion is that promotion recorded
        // the spot, not that it won a push race.
        let mut spots = run_cli(
            &second_device.profile,
            &["account".to_string(), "spots".to_string()],
        )
        .await?;
        assert!(spots.status.success(), "spots failed: {}", spots.stderr);
        for _ in 0..30 {
            if spots.stdout.contains(&key) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            spots = run_cli(
                &second_device.profile,
                &["account".to_string(), "spots".to_string()],
            )
            .await?;
            assert!(spots.status.success(), "spots failed: {}", spots.stderr);
        }
        assert!(
            spots.stdout.contains(&key),
            "promotion completed without recording the claimed spot in the account directory: {}",
            spots.stdout
        );

        let devtools = ChromeDevTools::new(claimer.handle.clone());
        devtools
            .execute_cdp_with_params(
                "Storage.clearDataForOrigin",
                serde_json::json!({
                    "origin": env.tonk_web.origin().ascii_serialization(),
                    "storageTypes": "all",
                }),
            )
            .await?;
        claimer.goto(env.tonk_web.as_str()).await?;
        claimer
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
        claimer.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&claimer, "tonk-account[data-mode=\"choice\"]").await?;
        element(&claimer, "#account-choose-link")
            .await?
            .click()
            .await?;
        element(&claimer, "#account-link-submit")
            .await?
            .click()
            .await?;
        if let Err(wait_error) = element(&claimer, "tonk-account[data-mode=\"success\"]").await {
            let host = element(&claimer, "tonk-account").await?;
            let mode = host.attr("data-mode").await?.unwrap_or_default();
            let error = element(&claimer, "#account-error").await?.text().await?;
            return Err(wait_error).context(format!(
                "second-device sign-in stopped in mode {mode:?}: {error:?}"
            ));
        }

        // Sign-in success precedes the account content pull that
        // carries the directory rows, and the on-demand mount needs
        // those rows. The Hub renders from a live subscription, so
        // arrival is eventually consistent by design; poll the load
        // the same way a page would re-render.
        let mut restored = get_json(&claimer, &format!("/api/repository/{key}")).await?;
        for _ in 0..30 {
            if restored["status"].as_u64().is_some_and(|s| s == 200) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            restored = get_json(&claimer, &format!("/api/repository/{key}")).await?;
        }
        let restored = successful_body("load claimed spot on second device", &restored);
        assert_eq!(restored["subject"], key);

        let pulled = post_json(
            &claimer,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull claimed spot on second device", &pulled);
        let hydrated = get_json(&claimer, &format!("/api/repository/{key}")).await?;
        assert_eq!(
            successful_body("load pulled spot on second device", &hydrated)["label"],
            "Shared Garden"
        );

        claimer.quit().await?;
        Ok(())
    }

    /// The full deletion stack under the button: plan review, email
    /// confirmation, device-signed deprovisioning of every owned
    /// hosted space, and both account-level finalizations. Only the
    /// passkey user-verification gesture is UI-side and out of scope;
    /// everything destructive runs here exactly as in production.
    #[dialog_common::test]
    async fn it_deletes_the_account_and_its_hosted_spaces(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "goner@example.com";
        sign_up(&driver, &env, email).await?;

        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({
                "name": "Doomed Garden",
                "remote": env.tonk_web.join("ucan/")?,
                "revocation_url": env.account_service.join("revocations")?,
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create synced spot", &created)["key"]
            .as_str()
            .context("create response omitted the spot key")?
            .to_string();
        let pushed = post_json(
            &driver,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push synced spot", &pushed);

        let plan = get_json(&driver, "/api/account/deletion/plan").await?;
        let plan = successful_body("review the deletion plan", &plan);
        assert_eq!(plan["email"], email, "plan reveals the verified email");
        let spaces = plan["spaces"]
            .as_array()
            .context("plan omitted the owned spaces")?;
        assert_eq!(spaces.len(), 1, "one owned hosted space: {plan}");
        let subject = spaces[0]["subject"]
            .as_str()
            .context("plan space omitted its subject")?
            .to_string();

        // A mistyped confirmation email refuses before anything burns.
        let refused = post_json(
            &driver,
            "/api/account/delete",
            serde_json::json!({
                "spaces": [{ "subject": subject }],
                "confirmedEmail": "someone-else@example.com",
            }),
        )
        .await?;
        assert_eq!(
            refused["status"], 403,
            "wrong confirmation email must refuse: {refused}"
        );

        let deleted = post_json(
            &driver,
            "/api/account/delete",
            serde_json::json!({
                "spaces": [{ "subject": subject }],
                "confirmedEmail": email,
            }),
        )
        .await?;
        let receipt = successful_body("delete the account", &deleted);
        assert_eq!(receipt["deletedSpaces"], 1, "one hosted space purged");
        assert_eq!(receipt["retainedJoinedSpaces"], 0);

        // The profile is unlinked: the deletion plan is no longer
        // reviewable because there is no account to review.
        let after = get_json(&driver, "/api/account/deletion/plan").await?;
        assert_eq!(
            after["status"], 404,
            "a deleted account leaves nothing to plan against: {after}"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_shares_inventory_between_cli_and_browser(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "inventory@example.com").await?;

        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({
                "name": "Cross-client Garden",
                "remote": env.tonk_web.join("ucan/")?,
                "revocation_url": env.account_service.join("revocations")?,
                "template": "blank",
            }),
        )
        .await?;
        let subject = successful_body("create cross-client space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();
        let pushed = post_json(
            &driver,
            &format!("/api/repository/{subject}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push cross-client space", &pushed);

        // Reconciliation is deliberately asynchronous. Provider projection is
        // its final step, so an inventory row with an exact confirmed revision
        // proves all earlier canonical writes completed too.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let browser_row = loop {
            let response = get_json(&driver, "/api/account/spaces").await?;
            if response["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status))
                && let Some(row) = response["body"].as_array().and_then(|rows| {
                    rows.iter().find(|row| {
                        row["subject"] == subject
                            && row["membership"] == "active"
                            && row["enrollment"] == "connected"
                            && row["confirmedRevision"].is_string()
                    })
                })
            {
                break row.clone();
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for connected browser account inventory: {response}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        };
        assert_eq!(browser_row["local"], true);
        assert_eq!(browser_row["pullable"], false);
        let confirmed = browser_row["confirmedRevision"]
            .as_str()
            .context("browser inventory omitted the confirmed revision")?
            .to_string();
        let remote = browser_row["remoteUrl"]
            .as_str()
            .context("browser inventory omitted the content remote")?
            .parse::<url::Url>()?;
        assert_eq!(remote.scheme(), "http");
        assert_eq!(remote.host_str(), Some("127.0.0.1"));
        assert_eq!(remote.path(), "/ucan/");

        let account = get_json(&driver, "/api/account").await?;
        let root = successful_body("read inventory account", &account)["rootDid"]
            .as_str()
            .context("inventory account status omitted its root DID")?
            .to_string();
        let provider_snapshot: Vec<serde_json::Value> = reqwest::Client::new()
            .get(env.account_service.join("_test/spots")?)
            .header("X-Test-Root", &root)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(
            provider_snapshot
                .iter()
                .any(|row| row["subject"] == subject && row["key"].is_string()),
            "connected browser inventory has no selected provider artifact: {provider_snapshot:?}"
        );

        let linked = link_cli(&driver, &env).await?;
        assert!(linked.link.status.success());
        let synced = run_cli(
            &linked.profile,
            &["account".to_string(), "sync".to_string()],
        )
        .await?;
        assert!(
            synced.status.success(),
            "account sync failed after link: stdout={} stderr={}",
            synced.stdout,
            synced.stderr
        );
        let status = run_cli(
            &linked.profile,
            &["account".to_string(), "status".to_string()],
        )
        .await?;
        assert!(
            status.status.success(),
            "account status failed: {}",
            status.stderr
        );
        assert!(
            status.stdout.contains(&root),
            "CLI linked to a different account root; browser={root} status={}",
            status.stdout
        );
        let provider = status
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("provider: "))
            .context("account status omitted the provider")?;
        assert_eq!(url::Url::parse(provider)?, env.account_service);
        let listed = run_cli(
            &linked.profile,
            &[
                "space".to_string(),
                "list".to_string(),
                "--all".to_string(),
                "--refresh".to_string(),
                "--json".to_string(),
            ],
        )
        .await?;
        assert!(
            listed.status.success(),
            "space list failed: {}",
            listed.stderr
        );
        let rows: Vec<serde_json::Value> = serde_json::from_str(&listed.stdout)?;
        let cli_row = rows
            .iter()
            .find(|row| row["subject"] == subject)
            .with_context(|| {
                format!(
                    "CLI inventory omitted the browser-created subject; link={} rows={}",
                    linked.link.stdout, listed.stdout
                )
            })?;
        assert_eq!(cli_row["accountMembership"], "active");
        assert_eq!(cli_row["localPresence"], "absent");
        assert_eq!(cli_row["transport"], "configured");
        assert_eq!(
            cli_row["authority"], "retained",
            "refreshed CLI inventory rejected saved authority: {}",
            listed.stderr
        );
        assert_eq!(cli_row["confirmedRevision"], confirmed);
        assert_eq!(cli_row["pullable"], true);

        let pulled = run_cli(
            &linked.profile,
            &[
                "account".to_string(),
                "spaces".to_string(),
                "pull".to_string(),
                subject.clone(),
                "--name".to_string(),
                "cross-client-garden".to_string(),
            ],
        )
        .await?;
        assert!(
            pulled.status.success(),
            "space pull failed: {}",
            pulled.stderr
        );
        assert!(pulled.stdout.contains("pulled\tcross-client-garden\t"));

        let listed = run_cli(
            &linked.profile,
            &[
                "space".to_string(),
                "list".to_string(),
                "--all".to_string(),
                "--json".to_string(),
            ],
        )
        .await?;
        assert!(
            listed.status.success(),
            "space relist failed: {}",
            listed.stderr
        );
        let rows: Vec<serde_json::Value> = serde_json::from_str(&listed.stdout)?;
        let cli_row = rows
            .iter()
            .find(|row| row["subject"] == subject)
            .context("CLI inventory lost the pulled subject")?;
        assert_eq!(cli_row["localPresence"], "registered");
        assert_eq!(cli_row["pullable"], false);

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_device_local_removal_hidden_across_reload_and_profile_switch(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "hidden-space@example.com").await?;
        let subject = create_connected_space(&driver, &env, "Hidden Garden").await?;
        let linked = link_cli(&driver, &env).await?;
        let synced = run_cli(
            &linked.profile,
            &["account".to_string(), "sync".to_string()],
        )
        .await?;
        assert!(
            synced.status.success(),
            "account sync failed: {}",
            synced.stderr
        );

        driver.goto(env.tonk_web.as_str()).await?;
        click_in_guest(&driver, &format!("label[for=\"rm-{subject}\"]")).await?;
        click_in_guest(
            &driver,
            &format!("form[data-remove=\"{subject}\"] wa-button[type=\"submit\"]"),
        )
        .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let profile = get_json(&driver, "/api/profile").await?;
            if !profile_space_keys(successful_body("read profile after removal", &profile))
                .contains(&subject)
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("device-local removal did not retract {subject}"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let hidden =
            wait_for_account_space(&driver, &subject, "active", Some("hiddenOnThisDevice")).await?;
        assert_eq!(hidden["local"], false);

        driver.refresh().await?;
        element(&driver, "tonk-site").await?;
        let reloaded = get_json(&driver, "/api/profile").await?;
        assert!(
            !profile_space_keys(successful_body("read reloaded profile", &reloaded))
                .contains(&subject)
        );
        wait_for_account_space(&driver, &subject, "active", Some("hiddenOnThisDevice")).await?;

        let profiles = get_json(&driver, "/api/profiles").await?;
        let first_profile = successful_body("read first profile", &profiles)["active"]
            .as_str()
            .context("profiles response omitted active profile")?
            .to_string();
        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        click(&driver, "#account-add-profile").await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        sign_up(&driver, &env, "other-profile@example.com").await?;
        click(
            &driver,
            &format!("#account-profile-list button[data-activate=\"{first_profile}\"]"),
        )
        .await?;
        wait_for_text_containing(&driver, "#account-email-value", "hidden-space@example.com")
            .await?;
        let switched_back = get_json(&driver, "/api/profile").await?;
        assert!(
            !profile_space_keys(successful_body("read switched profile", &switched_back))
                .contains(&subject)
        );
        wait_for_account_space(&driver, &subject, "active", Some("hiddenOnThisDevice")).await?;

        let listed = run_cli(
            &linked.profile,
            &[
                "space".to_string(),
                "list".to_string(),
                "--all".to_string(),
                "--refresh".to_string(),
                "--json".to_string(),
            ],
        )
        .await?;
        assert!(
            listed.status.success(),
            "CLI list failed: {}",
            listed.stderr
        );
        let rows: Vec<serde_json::Value> = serde_json::from_str(&listed.stdout)?;
        let cli_row = rows
            .iter()
            .find(|row| row["subject"] == subject)
            .context("another device lost active account membership")?;
        assert_eq!(cli_row["accountMembership"], "active");
        assert_eq!(cli_row["localPresence"], "absent");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_archives_account_membership_without_claiming_remote_or_peer_deletion(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "archive-space@example.com").await?;
        let subject = create_connected_space(&driver, &env, "Archive Garden").await?;
        let linked = link_cli(&driver, &env).await?;
        let synced = run_cli(
            &linked.profile,
            &["account".to_string(), "sync".to_string()],
        )
        .await?;
        assert!(
            synced.status.success(),
            "account sync failed: {}",
            synced.stderr
        );
        let devices_before = devices(&linked.profile, &env).await?;
        assert!(devices_before.status.success(), "{}", devices_before.stderr);

        let archived = run_cli(
            &linked.profile,
            &[
                "space".to_string(),
                "archive".to_string(),
                subject.clone(),
                "--yes".to_string(),
            ],
        )
        .await?;
        assert!(
            archived.status.success(),
            "CLI archive failed: stdout={} stderr={}",
            archived.stdout,
            archived.stderr
        );
        assert!(archived.stdout.contains("Archived account space"));

        let row = wait_for_account_space(&driver, &subject, "archived", None).await?;
        assert_eq!(row["local"], true, "archive must not unmount the peer copy");
        assert_eq!(row["pullable"], false);
        let readable = get_json(&driver, &format!("/api/repository/{subject}")).await?;
        successful_body("read already-mounted archived space", &readable);

        let account = get_json(&driver, "/api/account").await?;
        let root = successful_body("read archive account", &account)["rootDid"]
            .as_str()
            .context("account response omitted root DID")?;
        let active_provider: Vec<serde_json::Value> = reqwest::Client::new()
            .get(env.account_service.join("_test/spots")?)
            .header("X-Test-Root", root)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(
            active_provider.iter().all(|row| row["subject"] != subject),
            "the archived tombstone must replace the active provider head: {active_provider:?}"
        );
        let devices_after = devices(&linked.profile, &env).await?;
        assert!(devices_after.status.success(), "{}", devices_after.stderr);
        assert_eq!(
            devices_after.stdout, devices_before.stdout,
            "archive must not revoke devices or authority"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_adds_a_second_account_and_switches_between_disjoint_space_lists(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "first@example.com").await?;

        // First account creates a spot; its Hub lists it.
        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({
                "name": "First Garden",
                "remote": env.tonk_web.join("ucan/")?,
                "revocation_url": env.account_service.join("revocations")?,
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create first account's spot", &created)["key"]
            .as_str()
            .context("create response omitted the spot key")?
            .to_string();
        let listed = get_json(&driver, "/api/profile").await?;
        let space_keys = |body: &serde_json::Value| -> Vec<String> {
            body["space"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| entry["key"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert!(space_keys(successful_body("list first account's spaces", &listed)).contains(&key));
        let profiles = get_json(&driver, "/api/profiles").await?;
        let first_profile = successful_body("list profiles", &profiles)["active"]
            .as_str()
            .context("profiles response omitted the active name")?
            .to_string();

        // Add account: a fresh profile lands on the normal Choice flow,
        // where the second sign-up runs unchanged.
        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        element(&driver, "#account-add-profile")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        sign_up(&driver, &env, "second@example.com").await?;

        // The second account sees none of the first account's spots.
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("list second account's spaces", &listed)).is_empty(),
            "a fresh account must not see the other account's spots"
        );
        wait_for_text_containing(&driver, "#account-profile-list", "first@example.com").await?;

        // Switch back through the switcher; the first Hub returns.
        let selector = format!("#account-profile-list button[data-activate=\"{first_profile}\"]");
        click(&driver, &selector).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text_containing(&driver, "#account-email-value", "first@example.com").await?;
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("relist first account's spaces", &listed)).contains(&key),
            "switching back must restore the first account's spot list"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_links_the_cli_through_the_browser_callback(env: TestEnvironment) -> Result<()> {
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
            .find_map(|line| line.strip_prefix("account service: "))
            .context("status output omitted the account service")?;
        assert_eq!(url::Url::parse(provider)?, env.account_service);
        assert!(linked.link.stdout.contains("linked"));

        let devices = devices(&linked.profile, &env).await?;
        assert!(
            devices.status.success(),
            "devices failed: {}",
            devices.stderr
        );
        assert!(
            devices.stdout.contains("active\tChrome on "),
            "{}",
            devices.stdout
        );
        assert!(devices.stdout.contains("active\te2e terminal\t"));
        assert!(devices.stdout.contains(" (this device)"));

        driver.quit().await?;
        Ok(())
    }

    /// A CLI approved from a browser with no account yet: the link page
    /// runs the signup ceremony first — creating and registering the
    /// account is what makes there be something to delegate — then flows
    /// straight into the approval panel.
    #[dialog_common::test]
    async fn it_registers_before_linking_a_cli_from_a_fresh_browser(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let linked = link_cli_with(&driver, &env, true).await?;

        let status = run_cli(
            &linked.profile,
            &["account".to_string(), "status".to_string()],
        )
        .await?;
        assert!(status.status.success(), "status failed: {}", status.stderr);
        assert!(status.stdout.contains("signed in: yes"));
        assert!(
            linked.link.stdout.contains("access service:"),
            "the link reports the registration the signup performed: {}",
            linked.link.stdout
        );

        driver.quit().await?;
        Ok(())
    }

    /// A listener standing in for a waiting `tonk account link --via`.
    ///
    /// The CLI's half is a loopback server that accepts one form POST; a test
    /// needs no CLI process to play that part, only the same contract. It
    /// hands back whatever the page delivered.
    /// The one-shot slot a delivered authorization lands in.
    type Delivery =
        std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<(String, String)>>>>;

    async fn waiting_cli() -> Result<(String, tokio::sync::oneshot::Receiver<(String, String)>)> {
        use axum::extract::{Form, State};
        use std::collections::HashMap;

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(sender)));

        async fn deliver(
            State(slot): State<Delivery>,
            Form(form): Form<HashMap<String, String>>,
        ) -> &'static str {
            // The page posts the outcome alongside a `redirect` field; the
            // outcome field is the one under test.
            let (field, value) = ["authorize", "deny"]
                .into_iter()
                .find_map(|key| form.get(key).map(|value| (key.to_owned(), value.clone())))
                .unwrap_or_else(|| ("none".to_owned(), String::new()));
            if let Ok(mut slot) = slot.lock()
                && let Some(sender) = slot.take()
            {
                let _ = sender.send((field, value));
            }
            "received"
        }

        let app = axum::Router::new()
            .route("/", axum::routing::post(deliver))
            .with_state(slot);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((url, receiver))
    }

    /// The browser half of `tonk account link --via`: the page reads the
    /// waiting profile's DID and callback out of the URL, runs a real passkey
    /// ceremony, and posts the grant back.
    ///
    /// No CLI process is involved — a listener plays its part, since what the
    /// CLI contributes is one loopback endpoint and a contract. What this
    /// proves is the half the CLI tests cannot: that the ceremony runs and
    /// the page delivers something the CLI would accept.
    #[dialog_common::test]
    async fn it_authorizes_a_waiting_cli_from_the_browser(env: TestEnvironment) -> Result<()> {
        use base64::Engine as _;

        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        let (callback, delivered) = waiting_cli().await?;
        let audience = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let mut url = env.tonk_web.join("account/link")?;
        url.query_pairs_mut()
            .append_pair("audience", audience)
            .append_pair("callback", &callback);
        driver.goto(url.as_str()).await?;

        // The panel names the profile that is waiting, so the user knows what
        // they are approving.
        element(&driver, "tonk-account[data-mode=\"handoff\"]").await?;
        let shown = element(&driver, "#account-handoff-did")
            .await?
            .text()
            .await?;
        assert_eq!(shown, audience, "the page must name the waiting profile");

        element(&driver, "#account-handoff-submit")
            .await?
            .click()
            .await?;

        let (field, value) = tokio::time::timeout(Duration::from_secs(30), delivered)
            .await
            .context("the page never delivered an authorization")??;
        assert_eq!(field, "authorize", "approving must deliver a grant");

        // What arrived is what the CLI decodes: base64 over a payload
        // carrying both the delegation and the descriptor.
        let decoded = base64::engine::general_purpose::STANDARD.decode(&value)?;
        let payload: serde_json::Value = serde_json::from_slice(&decoded)?;
        for field in ["delegationHex", "descriptorHex"] {
            assert!(
                payload
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                "the authorization must carry {field}: {payload}"
            );
        }
        Ok(())
    }

    /// Declining tells the waiting process, rather than leaving it to time
    /// out on a decision the user already made.
    #[dialog_common::test]
    async fn it_declines_a_waiting_cli_from_the_browser(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        let (callback, delivered) = waiting_cli().await?;
        let mut url = env.tonk_web.join("account/link")?;
        url.query_pairs_mut()
            .append_pair(
                "audience",
                "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
            )
            .append_pair("callback", &callback);
        driver.goto(url.as_str()).await?;

        element(&driver, "tonk-account[data-mode=\"handoff\"]").await?;
        element(&driver, "#account-handoff-cancel")
            .await?
            .click()
            .await?;

        let (field, _) = tokio::time::timeout(Duration::from_secs(30), delivered)
            .await
            .context("cancelling never reached the waiting process")??;
        assert_eq!(
            field, "deny",
            "cancelling must report a denial, not leave the CLI waiting"
        );
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
        wait_for_text_containing(&driver, "#account-device-list", "e2e terminal").await?;
        let selector = format!("#account-device-list button[data-revoke=\"{cli_did}\"]");
        click(&driver, &selector).await?;
        driver.accept_alert().await?;
        wait_for_text_containing(&driver, "#account-working", "Access removed").await?;

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
