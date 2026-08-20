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

        // The conflict surfaces at account creation, and creation is
        // secret-rooted: no credential is minted on the failed attempt
        // or on the successful retry.
        wait_for_text(
            &driver,
            "#account-error",
            "an account already exists for this email address",
        )
        .await?;
        assert_eq!(credential_count(&driver, &authenticator_id).await?, 0);

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
            0,
            "secret-rooted creation must mint no credentials at all"
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

        let account = get_json(&claimer, "/api/account").await?;
        let root = successful_body("read claiming account", &account)["rootDid"]
            .as_str()
            .context("claiming account status omitted its root DID")?;
        let snapshot: Vec<serde_json::Value> = reqwest::Client::new()
            .get(env.account_service.join("_test/spots")?)
            .header("X-Test-Root", root)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert!(
            snapshot.iter().any(|spot| spot["subject"] == key),
            "promotion completed without uploading the claimed spot: {snapshot:?}"
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

        let restored = get_json(&claimer, &format!("/api/repository/{key}")).await?;
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
            linked.link.stdout.contains("sync service:"),
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
