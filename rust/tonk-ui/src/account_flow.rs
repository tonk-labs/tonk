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

    /// Wait until `selector`'s text no longer contains `gone` — the shape
    /// a retraction takes in the DOM.
    async fn wait_for_text_without(driver: &WebDriver, selector: &str, gone: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && let Ok(text) = found.text().await
                && !text.contains(gone)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to stop containing {gone:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The latest activation link the access service captured for `email`.
    async fn activation_link(env: &TestEnvironment, email: &str) -> Result<String> {
        let endpoint = env.access_service.join("_test/emails")?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
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

    /// Wait until the service worker controls the page.
    ///
    /// Polled from the test side in small steps. The previous shape — one
    /// `execute_async` that resolved on `controllerchange` — was bounded
    /// by chromedriver's script timeout, and a cold CI runner spends
    /// longer than that installing the worker (compiling its wasm is the
    /// long pole), which surfaced as "script timeout" flakes. A poll has
    /// no long-running script to time out. No reload is needed: the
    /// page's boot path nudges an already-active worker to claim it.
    async fn wait_for_service_worker(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        loop {
            let controlled = driver
                .execute(
                    "return !!(navigator.serviceWorker && navigator.serviceWorker.controller);",
                    vec![],
                )
                .await
                .ok()
                .and_then(|ret| ret.json().as_bool());
            if controlled == Some(true) {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the service worker never took control of the page"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait until the dashboard reports the deferred account work done:
    /// the custody cell published and nothing left in the queue.
    ///
    /// The signal is DOM state the page itself settles —
    /// `tonk-account[data-backup]` — not a REST probe re-deriving it:
    /// the dashboard is the one place that can run the queued publish
    /// (it takes a passkey assertion), so it is also the authority on
    /// whether the publish happened. "done" is the settled state.
    /// "stuck" means this load's attempt left the queue non-empty and
    /// the next load retries, so that is the one case worth a reload.
    ///
    /// Call with the dashboard as the current page.
    async fn wait_for_backup_done(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            // An absent panel or attribute is "not yet": the load that
            // settles it may still be running.
            let backup = match driver.find(By::Css("tonk-account")).await {
                Ok(host) => host.attr("data-backup").await.ok().flatten(),
                Err(_) => None,
            };
            match backup.as_deref() {
                Some("done") => return Ok(()),
                Some("stuck") => driver.refresh().await?,
                _ => {}
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the dashboard never reported the account backup done \
                 (last state: {backup:?})"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
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

    /// Create an account and confirm its email, leaving it able to host
    /// spaces. Most callers want this.
    pub(crate) async fn sign_up(
        driver: &WebDriver,
        env: &TestEnvironment,
        email: &str,
    ) -> Result<()> {
        enroll_only(driver, env, email).await?;
        // The access service provisions nothing and serves nothing for a
        // customer that has not confirmed its email, so a signed-up
        // account cannot host a space until this happens.
        activate(driver, env, email).await?;
        Ok(())
    }

    /// Create an account and stop, leaving the customer `Registered`
    /// with its activation email unopened — the window in which the
    /// service refuses everything and the client queues it.
    pub(crate) async fn enroll_only(
        driver: &WebDriver,
        env: &TestEnvironment,
        email: &str,
    ) -> Result<()> {
        wait_for_service_worker(driver).await?;
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

    /// Follow the emailed activation link and accept, leaving the
    /// customer `Active` and its queued work drained.
    pub(crate) async fn activate(
        driver: &WebDriver,
        env: &TestEnvironment,
        email: &str,
    ) -> Result<()> {
        let link = activation_link(env, email).await?;
        let account = driver.current_url().await?;
        driver.goto(&link).await?;
        element(driver, "#activate-accept").await?.click().await?;
        element(driver, "#activate-done").await?;
        // Activation is what unblocks the deferred account work, and the
        // custody-cell publish in that queue is what every later ceremony
        // (unlock, CLI approval, legacy link) resolves. The dashboard
        // publishes it in the background of its load, so returning as
        // soon as the page renders hands a race to whatever the caller
        // does next: navigating away kills the in-flight publish, and a
        // profile rotation orphans it — both of which surfaced in CI as
        // "no account custody is published for this passkey". Stay on
        // the dashboard until it says the backup settled.
        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        wait_for_backup_done(driver).await?;
        // Back to where the caller was: activation is a detour, not a
        // navigation the caller asked for.
        driver.goto(account.as_str()).await?;
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
        // sign_up already followed the emailed link, so the account is
        // past activation: the registration row says so, and the
        // pending-activation banner is gone.
        wait_for_text(&driver, "#account-registration-value", "Active").await?;
        if let Ok(notice) = driver.find(By::Css("#account-activation-notice")).await {
            let text = notice.text().await.unwrap_or_default();
            assert!(
                !text.contains("activation pending"),
                "an activated account must not still nag about activation, got {text:?}"
            );
        }

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_back_into_the_same_account_after_signing_out(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        click(&driver, "#account-unlink").await?;
        driver.accept_alert().await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;

        click(&driver, "#account-choose-link").await?;
        click(&driver, "#account-link-submit").await?;
        if let Err(wait_error) = element(&driver, "tonk-account[data-mode=\"success\"]").await {
            let host = element(&driver, "tonk-account").await?;
            let mode = host.attr("data-mode").await?.unwrap_or_default();
            let error = element(&driver, "#account-error").await?.text().await?;
            return Err(wait_error).context(format!(
                "same-account re-login stopped in mode {mode:?}: {error:?}"
            ));
        }

        let summary = get_json(&driver, "/api/account/summary").await?;
        assert_eq!(
            successful_body("account summary after re-login", &summary)["email"],
            EMAIL
        );
        let devices = get_json(&driver, "/api/account/devices").await?;
        let devices = successful_body("device list after re-login", &devices)
            .as_array()
            .context("device list was not an array")?;
        assert_eq!(devices.len(), 1, "re-login must not duplicate the device");

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
        wait_for_service_worker(&driver).await?;
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

    fn tonk_command_in(env: &TestEnvironment, profile: &TempDir) -> Command {
        let mut command = tonk_command(profile);
        // Trust this harness's Caddy root specifically. A process-wide
        // SSL_CERT_FILE would be whichever concurrent harness wrote it
        // last, leaving this child unable to reach its own origin.
        if let Some(ca) = &env.ca_certificate {
            command.env("SSL_CERT_FILE", ca);
        }
        command
    }

    fn tonk_command(profile: &TempDir) -> Command {
        let mut command = Command::new(tonk_bin());
        command
            .current_dir(profile.path())
            .env("HOME", profile.path())
            .env("XDG_DATA_HOME", profile.path().join("data"))
            .env("TONK_SPACES_STATE", profile.path().join("spaces"))
            .env("TONK_TELEMETRY_STATE", profile.path().join("telemetry"))
            .env("TONK_UPDATE_STATE", profile.path().join("update"))
            .env("TONK_NO_UPDATE_CHECK", "1")
            .env("DO_NOT_TRACK", "1")
            .env("NO_PROXY", "127.0.0.1,localhost,tonk.network")
            .env_remove("TONK_TELEMETRY")
            .env_remove("TONK_SPACE")
            .env_remove("TONK_UNSAFE_ALLOW_DEVICE_ROOT");
        command
    }

    struct CliOutput {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct JsonRows<T> {
        schema_version: String,
        rows: Vec<T>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CliDeviceRow {
        status: String,
        name: String,
        did: String,
        this_device: bool,
    }

    #[derive(Debug, serde::Deserialize)]
    struct CliAccountSpaceRow {
        subject: String,
    }

    async fn run_cli(
        env: &TestEnvironment,
        profile: &TempDir,
        args: &[String],
    ) -> Result<CliOutput> {
        // Bounded like `finish_link`: a CLI that hangs must fail the
        // test that ran it, not hold the suite until the job timeout.
        // `kill_on_drop` is what actually reaps the child when the
        // timeout drops the future — `output()` alone would leave it
        // running.
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            tonk_command_in(env, profile)
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| anyhow!("timed out waiting for `tonk {}`", args.join(" ")))??;
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
                Err(anyhow!("timed out waiting for `tonk account login`"))
            }
        }
    }

    struct LinkedCli {
        profile: TempDir,
        link: CliOutput,
    }

    /// Where the CLI learns which account service the link belongs to.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum AccountService {
        /// Named on the command line, as staging and local development do.
        Named,
        /// Left to the ceremony page. The hidden flag then keeps its
        /// production default, so anything matched against it instead of
        /// against what the page delivered names the wrong deployment.
        FromThePage,
    }

    async fn link_cli(driver: &WebDriver, env: &TestEnvironment) -> Result<LinkedCli> {
        link_cli_with(driver, env, false).await
    }

    async fn link_cli_with(
        driver: &WebDriver,
        env: &TestEnvironment,
        register_first: bool,
    ) -> Result<LinkedCli> {
        link_cli_using(driver, env, register_first, AccountService::Named).await
    }

    async fn link_cli_using(
        driver: &WebDriver,
        env: &TestEnvironment,
        register_first: bool,
        service: AccountService,
    ) -> Result<LinkedCli> {
        let profile = tempfile::tempdir()?;
        let mut command = tonk_command_in(env, &profile);
        command.args([
            "account",
            "login",
            "--name",
            "e2e terminal",
            "--no-open",
            "--via",
            env.tonk_web.join("account/link")?.as_str(),
        ]);
        if service == AccountService::Named {
            command.args(["--service-url", env.account_service.as_str()]);
        }
        command
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
            // Let the creation ceremony finish before navigating
            // anywhere: it lands back on the approval it interrupted,
            // and leaving mid-flight loses whatever it had not yet
            // persisted.
            element(driver, "tonk-account[data-mode=\"handoff\"]").await?;
            // Approving unlocks the account, which reads the custody
            // cell — and that cell cannot be published until the
            // customer confirms its email. Do it now, then come back to
            // the approval.
            activate(driver, env, EMAIL).await?;
            driver.goto(approval_url.as_str()).await?;
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
        if let Err(wait_error) = element(driver, "tonk-account[data-mode=\"success\"]").await {
            // Say WHERE the approval stopped, not just that it did: the
            // panel's mode, its error line, and its status line are what
            // separate a ceremony that failed from a callback that never
            // redirected.
            let host = element(driver, "tonk-account").await?;
            let mode = host.attr("data-mode").await?.unwrap_or_default();
            let error = match driver.find(By::Css("#account-error")).await {
                Ok(element) => element.text().await.unwrap_or_default(),
                Err(_) => String::new(),
            };
            let working = match driver.find(By::Css("#account-working")).await {
                Ok(element) => element.text().await.unwrap_or_default(),
                Err(_) => String::new(),
            };
            let url = driver.current_url().await?;
            return Err(wait_error).context(format!(
                "approval stopped in mode {mode:?} at {url}; error={error:?}; status={working:?}"
            ));
        }
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
                assert_eq!(outcome_line.trim_end(), "signed in");
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
        assert!(link.stdout.contains("signed in\naccount: did:key:"));
        assert!(link.stdout.contains("device: did:key:"));
        assert!(
            link.stdout.contains("status: synced")
                || link.stdout.contains("status: waiting for first sync")
        );

        Ok(LinkedCli { profile, link })
    }

    /// The one CLI call the suite has seen stall in CI: it runs traced,
    /// so a failure's stderr carries every connection, request, and
    /// response with timestamps instead of a bounded "did not answer".
    async fn devices(profile: &TempDir, env: &TestEnvironment) -> Result<CliOutput> {
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            tonk_command_in(env, profile)
                .args([
                    "account",
                    "devices",
                    "--service-url",
                    env.account_service.as_str(),
                    "--json",
                ])
                .env("TONK_TRACE", "1")
                .env(
                    "RUST_LOG",
                    "debug,hyper=trace,hyper_util=trace,reqwest=debug,rustls=info,h2=info,dialog_remote_ucan_s3=trace,dialog_remote_s3=trace,dialog_operator=debug",
                )
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| anyhow!("timed out waiting for `tonk account devices`"))??;
        Ok(CliOutput {
            status: output.status,
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
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

    /// POST a YAML document, the way `/evaluate` takes source.
    ///
    /// The JSON routes cannot make a replica write real content, and a
    /// replica with nothing to write never presigns — which is what made
    /// every status-code assertion in this file vacuous.
    async fn post_yaml(driver: &WebDriver, path: &str, body: &str) -> Result<serde_json::Value> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                fetch(arguments[0], {
                    method: "POST",
                    headers: { "content-type": "application/yaml" },
                    body: arguments[1],
                }).then(async response => done({
                    status: response.status,
                    body: await response.text(),
                })).catch(error => done({ error: String(error) }));
                "#,
                vec![serde_json::json!(path), serde_json::json!(body)],
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

    /// Whether `driver`'s replica of `key` can see a bookmark named
    /// `bookmark` on the content branch.
    ///
    /// This is the only honest oracle for revocation. A status code from
    /// the guest's own worker reports what the worker did locally, which
    /// is decoupled from whether the access service served the upload —
    /// so only the OTHER party's view distinguishes a revoked invite from
    /// a working one.
    async fn owner_sees(driver: &WebDriver, key: &str, bookmark: &str) -> Result<bool> {
        // The `Name` concept's wire shape, inlined: `tonk_worker::helpers`
        // is feature-gated off in this build. `this` is the name entity,
        // derived by prefixing `id:` — the row carries that, never the
        // bare name string, so the match is on `id:<bookmark>`.
        let query = serde_json::json!({
            "terms": {
                "this": { "?": { "name": "this", "type": { "primitive": { "bits": 64 } } } },
                "entity": { "?": { "name": "entity", "type": { "primitive": { "bits": 64 } } } }
            },
            "predicate": {
                "with": {
                    "entity": {
                        "the": "db.name/referent",
                        "cardinality": "one",
                        "as": "Entity"
                    }
                }
            }
        });
        let response = post_json(
            driver,
            &format!("/api/repository/{key}/branch/main/query"),
            query,
        )
        .await?;
        let wanted = format!("id:{bookmark}");
        let rows = response["body"].as_array().cloned().unwrap_or_default();
        Ok(rows.iter().any(|row| {
            row["fields"]["this"].as_str() == Some(wanted.as_str())
                || row["this"].as_str() == Some(wanted.as_str())
        }))
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

    fn device_rows(output: &str) -> Result<Vec<CliDeviceRow>> {
        let report: JsonRows<CliDeviceRow> =
            serde_json::from_str(output).context("account devices output was not valid JSON")?;
        if report.schema_version != "tonk.account-devices.v1" {
            return Err(anyhow!(
                "account devices returned unsupported schema {}",
                report.schema_version
            ));
        }
        Ok(report.rows)
    }

    fn account_space_subjects(output: &str) -> Result<Vec<String>> {
        let report: JsonRows<CliAccountSpaceRow> =
            serde_json::from_str(output).context("account space output was not valid JSON")?;
        if report.schema_version != "tonk.account-spaces.v1" {
            return Err(anyhow!(
                "account space returned unsupported schema {}",
                report.schema_version
            ));
        }
        Ok(report.rows.into_iter().map(|row| row.subject).collect())
    }

    fn did_for_device<'a>(rows: &'a [CliDeviceRow], name: &str) -> Option<&'a str> {
        rows.iter()
            .find(|row| row.name == name)
            .map(|row| row.did.as_str())
    }

    /// A space created before activation stays LOCAL: no remote, no
    /// provisioning, and therefore no refused presign.
    ///
    /// A device has an account from first boot, so "an account exists"
    /// says nothing about whether the access service will serve a
    /// space. Until the emailed link is confirmed the service refuses
    /// both provisioning and presign, so wiring an upstream would
    /// produce a space that syncs to `subject is provisioned by an
    /// active customer (the subject is not provisioned)` on every
    /// attempt. The space works locally and the share button attaches
    /// sync later, once there is a provider to attach to.
    ///
    /// This replaces an earlier contract where the create queued its
    /// provisioning and replayed it at activation. That left the space
    /// wired to a remote it could not use for the whole waiting period,
    /// which is the 403 this gate exists to prevent.
    #[dialog_common::test]
    async fn it_creates_a_space_local_only_before_activation(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "queued@example.com";
        // Stop at Registered: the activation email is sent but unopened.
        enroll_only(&driver, &env, email).await?;
        wait_for_text_containing(&driver, "#account-activation-notice", "activation pending")
            .await?;

        // No remote in the request: the worker decides, the way the
        // create wizard now leaves it to.
        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({
                "name": "Made While Waiting",
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create space before activation", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();

        // The space exists and is remote-less: nothing was wired, so
        // nothing can fail against the service.
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the space configuration", &info);
        // `RepositoryInfo::remote` skips serializing an empty map, so
        // "no remotes" is an ABSENT key rather than an empty object.
        assert!(
            info["remote"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "a space created before activation must wire no remote, got {}",
            info["remote"],
        );
        let upstream = &info["branch"]["main"]["upstream"];
        assert!(
            upstream.is_null(),
            "main must track nothing before activation, got {upstream}",
        );

        driver.quit().await?;
        Ok(())
    }

    /// Activation records the provider, and a space created after it
    /// attaches to that provider without the page naming one.
    ///
    /// The service decides which provider serves its customers and says
    /// so in the activation receipt; the client records it as a fact on
    /// profile main. Every attach path reads that one answer, so the
    /// page no longer derives `https://{origin}/ucan/` for itself.
    #[dialog_common::test]
    async fn it_attaches_the_recorded_provider_after_activation(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "provided@example.com").await?;
        wait_for_text(&driver, "#account-registration-value", "Active").await?;

        // Again no remote: if the worker did not read the recorded
        // provider, this space would come up local-only.
        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({
                "name": "Made After Activation",
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create space after activation", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();

        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the space configuration", &info);
        // Report what the worker actually knows, so a failure here says
        // whether the provider fact was recorded or merely unread.
        let customer = get_json(&driver, "/api/customer").await?;
        assert!(
            info["remote"]["origin"].is_object(),
            "an activated account's space must wire the origin remote, got {}; \
             customer state was {customer}",
            info["remote"],
        );
        let upstream = &info["branch"]["main"]["upstream"];
        assert_eq!(
            upstream["remote"].as_str(),
            Some("origin"),
            "main must track the attached remote, got {upstream}",
        );

        driver.quit().await?;
        Ok(())
    }

    /// A space created before activation becomes syncable once the user
    /// asks for it, and not before.
    ///
    /// The opt-in half of the local-only gate: creation withholds the
    /// remote, and an explicit enable-sync is what provisions the space
    /// and attaches one. Provisioning at attach time is what makes this
    /// work — before, `enable_sync` attached without ever calling
    /// `/provider/add`, so the upstream pointed at a subject the service
    /// refused.
    #[dialog_common::test]
    async fn it_syncs_a_local_only_space_once_sync_is_enabled(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "optin@example.com";
        enroll_only(&driver, &env, email).await?;
        wait_for_text_containing(&driver, "#account-activation-notice", "activation pending")
            .await?;

        let created = post_json(
            &driver,
            "/api/spaces",
            serde_json::json!({ "name": "Opted In", "template": "blank" }),
        )
        .await?;
        let key = successful_body("create space before activation", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();

        // Confirm the email, so a provider exists to attach to.
        activate(&driver, &env, email).await?;
        wait_for_text(&driver, "#account-registration-value", "Active").await?;

        // Still local: activation does not retroactively sync spaces
        // created before it. The user opts in per space.
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the space configuration", &info);
        assert!(
            info["remote"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "activation must not retroactively attach a remote, got {}",
            info["remote"],
        );

        // Opting in attaches and provisions, so the space can now push.
        let attached = post_json(
            &driver,
            &format!("/api/repository/{key}/remote"),
            serde_json::json!({
                "remote": { "origin": { "address": { "Ucan": { "endpoint": env.tonk_web.join("ucan/")? } } } },
                "branch": { "main": { "upstream": { "remote": "origin", "branch": "main" } } },
            }),
        )
        .await?;
        successful_body("attach the remote", &attached);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let pushed = post_json(
                &driver,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?;
            if pushed["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status))
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "an opted-in space must be provisioned and pushable: {pushed}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        driver.quit().await?;
        Ok(())
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
        let key = successful_body("create synced space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();
        let pushed = post_json(
            &creator,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push synced space", &pushed);
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
            "/api/profile/join",
            serde_json::json!({ "url": invite_url }),
        )
        .await?;
        successful_body("join shared space", &visited);

        // The account directory is the backup now: link a CLI as a
        // second device of the claimer's account and read the claimed
        // space back out of the synced account DB — the real
        // cross-device path, not a service-side artifact store.
        let second_device = link_cli_with(&claimer, &env, false).await?;
        // Two things have to land before this reads: the freshly linked
        // device's first account sync (until then `spaces` exits non-zero
        // with "not yet hydrated"), and the browser's push of the
        // directory facts, which happens on its next sync drain. Both
        // are timing, not behaviour, so poll on the outcome under test —
        // that promotion recorded the space — rather than asserting on
        // whichever intermediate state the first run happened to catch.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut last_seen = String::from("<account space never completed a run>");
        let recorded = loop {
            // Drive the browser's sync drain rather than waiting for
            // incidental traffic to trigger one. Promotion writes the
            // directory facts locally; publishing them to the account
            // remote happens on a drain, and once the test stops
            // touching the page nothing else schedules one.
            let _ = post_json(&claimer, "/api/sync", serde_json::json!({})).await;
            let run = run_cli(
                &env,
                &second_device.profile,
                &[
                    "account".to_string(),
                    "space".to_string(),
                    "--json".to_string(),
                ],
            )
            .await?;
            if run.status.success() {
                if account_space_subjects(&run.stdout)?
                    .iter()
                    .any(|subject| subject == &key)
                {
                    break true;
                }
                last_seen = run.stdout;
            } else if run.stderr.contains("first sync has not succeeded yet") {
                // Hydration maps every underlying error to Unhydrated,
                // so this one message covers both a first sync that has
                // genuinely not landed yet and one that cannot land at
                // all. Only the former is worth waiting on: a transport
                // failure means the harness origin is unreachable from
                // this CLI child, which no amount of retrying fixes.
                if run.stderr.contains("Transport error") {
                    return Err(anyhow!(
                        "the linked CLI cannot reach the harness origin, so the account \
                         can never sync: {}",
                        run.stderr.trim()
                    ));
                }
                last_seen = format!("not hydrated yet: {}", run.stderr.trim());
            } else {
                // Any other non-zero exit is a real error; failing here
                // beats burning the deadline on it.
                return Err(anyhow!("account space failed: {}", run.stderr));
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        assert!(
            recorded,
            "promotion completed without recording the claimed space in the account \
             directory; last `account space` output was: {last_seen}"
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
        wait_for_service_worker(&claimer).await?;
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
        let restored = successful_body("load claimed space on second device", &restored);
        assert_eq!(restored["subject"], key);

        let pulled = post_json(
            &claimer,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull claimed space on second device", &pulled);
        let hydrated = get_json(&claimer, &format!("/api/repository/{key}")).await?;
        assert_eq!(
            successful_body("load pulled space on second device", &hydrated)["label"],
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
    async fn it_deletes_the_account_and_releases_its_email_and_profile(
        env: TestEnvironment,
    ) -> Result<()> {
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
        let key = successful_body("create synced space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();
        let pushed = post_json(
            &driver,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push synced space", &pushed);

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

        // Permanent deletion retires this account's local profile rather
        // than rebinding its retained authority to another root. The browser
        // must already be on a fresh profile so the released email can create
        // a genuinely new account without the user finding the hidden
        // "different account" root conflict.
        sign_up(&driver, &env, email).await?;
        let recreated = get_json(&driver, "/api/account/summary").await?;
        assert_eq!(
            successful_body("load the recreated account", &recreated)["email"],
            email
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

        // First account creates a space; its Hub lists it.
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
        let key = successful_body("create first account's space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
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

        // The second account sees none of the first account's spaces.
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("list second account's spaces", &listed)).is_empty(),
            "a fresh account must not see the other account's spaces"
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
            "switching back must restore the first account's space list"
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
            &env,
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
        assert!(linked.link.stdout.contains("signed in"));

        let devices = devices(&linked.profile, &env).await?;
        assert!(
            devices.status.success(),
            "devices failed: {}",
            devices.stderr
        );
        let device_rows = device_rows(&devices.stdout)?;
        assert!(
            device_rows
                .iter()
                .any(|row| row.status == "active" && row.name.starts_with("Chrome on ")),
            "{}",
            devices.stdout
        );
        assert!(
            device_rows.iter().any(|row| {
                row.status == "active" && row.name == "e2e terminal" && row.this_device
            }),
            "the linked terminal must be the row marked as this device: {}",
            devices.stdout
        );

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
            &env,
            &linked.profile,
            &["account".to_string(), "status".to_string()],
        )
        .await?;
        assert!(status.status.success(), "status failed: {}", status.stderr);
        assert!(status.stdout.contains("signed in: yes"));
        // `login` prints the sign-in itself; the registration the signup
        // performed is reported by `status`, which is the one command that
        // reads the access service.
        assert!(
            status.stdout.contains("access service:"),
            "status reports the registration the signup performed: {}",
            status.stdout
        );

        driver.quit().await?;
        Ok(())
    }

    /// Linking without naming the account service records the deployment
    /// the ceremony actually ran on.
    ///
    /// The page delivers its own service URL and the CLI attaches to that,
    /// so the endpoints it discovers must be matched against the same
    /// value. Matched against the flag instead, every ceremony outside
    /// production disagrees with its own deployment, because the flag is
    /// hidden and defaults to production.
    #[dialog_common::test]
    async fn it_links_without_being_told_the_account_service(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let linked = link_cli_using(&driver, &env, false, AccountService::FromThePage).await?;

        let status = run_cli(
            &env,
            &linked.profile,
            &["account".to_string(), "status".to_string()],
        )
        .await?;
        let provider = status
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("account service: "))
            .context("status output omitted the account service")?;
        assert_eq!(url::Url::parse(provider)?, env.account_service);

        // The endpoints are what `space new` and `space link` need; the
        // registry is where they are read from, and status does not print
        // them.
        let registry: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            linked.profile.path().join("spaces").join("spaces.json"),
        )?)?;
        let account = registry
            .get("account")
            .context("the registry recorded no account")?;
        let endpoint = |field: &str| -> Result<url::Url> {
            let value = account
                .get(field)
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("the account record omitted {field}"))?;
            Ok(url::Url::parse(value)?)
        };
        let origin = url::Url::parse(&format!("{}/", env.tonk_web.origin().ascii_serialization()))?;
        assert_eq!(endpoint("ceremonyOrigin")?, origin);
        assert_eq!(endpoint("accessRemote")?, origin.join("/ucan/")?);

        driver.quit().await?;
        Ok(())
    }

    /// A listener standing in for a waiting `tonk account login --via`.
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

    /// The browser half of `tonk account login --via`: the page reads the
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

        // Generous: approving runs a passkey assertion, the unlock, and
        // the device registration before the callback POST, and a loaded
        // CI runner stretches each of them.
        let (field, value) = tokio::time::timeout(Duration::from_secs(60), delivered)
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

        let (field, _) = tokio::time::timeout(Duration::from_secs(60), delivered)
            .await
            .context("cancelling never reached the waiting process")??;
        assert_eq!(
            field, "deny",
            "cancelling must report a denial, not leave the CLI waiting"
        );
        Ok(())
    }

    /// Revocation as the user experiences it: a guest who claimed an
    /// invite loses access to the space when that invite is withdrawn.
    ///
    /// The property under test is the one that matters, and the one the
    /// account-service device list cannot show: after revocation the
    /// claimed credential no longer reaches storage. That runs the whole
    /// path, from minting the artifact through `/ucan/revoke` recording
    /// it to the chain walk refusing a chain that rests on it.
    #[dialog_common::test]
    async fn it_cuts_off_storage_access_when_an_invite_is_revoked(
        env: TestEnvironment,
    ) -> Result<()> {
        let owner = driver_with_prf(&env).await?;
        sign_up(&owner, &env, "owner@example.com").await?;

        let created = post_json(
            &owner,
            "/api/spaces",
            serde_json::json!({
                "name": "Revocable Garden",
                "remote": env.tonk_web.join("ucan/")?,
                "template": "blank",
            }),
        )
        .await?;
        let key = successful_body("create space", &created)["key"]
            .as_str()
            .context("create response omitted the space key")?
            .to_string();
        successful_body(
            "push space",
            &post_json(
                &owner,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?,
        );

        let invited = post_json(
            &owner,
            &format!("/api/repository/{key}/invite"),
            serde_json::json!({ "baseUrl": env.tonk_web.join("join")? }),
        )
        .await?;
        let invite_url = successful_body("mint invite", &invited)["url"]
            .as_str()
            .context("invite response omitted its URL")?
            .to_string();
        // The mint answers a URL; the revocation target comes from the
        // invitation listing, which is where its CID is recorded.
        let listed = get_json(&owner, &format!("/api/repository/{key}/invites")).await?;
        let body = successful_body("list invites", &listed);
        let invite_cid = body
            .as_array()
            .and_then(|invites| invites.first())
            .and_then(|invite| invite["targetCid"].as_str())
            .with_context(|| {
                format!("invitation listing carried no target CID; listing was: {body}")
            })?
            .to_string();

        // A guest claims it and can reach the space.
        let guest = driver_with_prf(&env).await?;
        sign_up(&guest, &env, "guest@example.com").await?;
        successful_body(
            "visit invite",
            &post_json(
                &guest,
                "/api/profile/join",
                serde_json::json!({ "url": invite_url }),
            )
            .await?,
        );
        successful_body(
            "guest pulls before revocation",
            &post_json(
                &guest,
                &format!("/api/repository/{key}/branch/main/sync/pull"),
                serde_json::json!({}),
            )
            .await?,
        );

        // The guest writes something the owner can look for, and syncs it
        // up. This half proves the write path WORKS before revocation, so
        // its absence afterwards means something.
        //
        // Every earlier version of this test asserted on a status code
        // from `sync/pull` or `sync/push`. Both are vacuous: a replica
        // with nothing to fetch never presigns, and a replica with nothing
        // to send never uploads, so both answer 200 whether or not the
        // invite was revoked. The guest in those versions never wrote
        // anything at all, so the write path this test exists to check was
        // never exercised.
        let before_marker = "xyz.tonk.e2e/before-revocation";
        let after_marker = "xyz.tonk.e2e/after-revocation";
        // Distinct bookmark names, so the owner can tell the two writes
        // apart in the Name index.
        let declare = |bookmark: &str, attribute: &str| {
            format!(
                r#"attribute!: &{bookmark}
  the:         {attribute}
  as:          text
  cardinality: one
  description: revocation e2e marker
"#
            )
        };

        let wrote = post_yaml(
            &guest,
            &format!("/api/repository/{key}/branch/main/evaluate"),
            &declare("before-revocation", before_marker),
        )
        .await?;
        assert_eq!(
            wrote["status"].as_u64(),
            Some(200),
            "the guest must be able to write before revocation: {wrote}"
        );
        successful_body(
            "guest pushes its pre-revocation write",
            &post_json(
                &guest,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?,
        );
        successful_body(
            "owner pulls the guest's pre-revocation write",
            &post_json(
                &owner,
                &format!("/api/repository/{key}/branch/main/sync/pull"),
                serde_json::json!({}),
            )
            .await?,
        );
        let owner_sees_before = owner_sees(&owner, &key, "before-revocation").await?;
        assert!(
            owner_sees_before,
            "the guest's pre-revocation write must reach the owner, or the \
             post-revocation assertion below proves nothing"
        );

        // The owner withdraws the invite.
        successful_body(
            "revoke invite",
            &post_json(
                &owner,
                &format!("/api/repository/{key}/invites/{invite_cid}/revoke"),
                serde_json::json!({}),
            )
            .await?,
        );

        // Now the same sequence must NOT reach the owner. Asserted on
        // CONTENT, not on a status code: the guest's worker may report a
        // successful push for an upload the access service refused, so
        // only the owner's view distinguishes a revoked invite from a
        // working one.
        let wrote_after = post_yaml(
            &guest,
            &format!("/api/repository/{key}/branch/main/evaluate"),
            &declare("after-revocation", after_marker),
        )
        .await?;
        assert_eq!(
            wrote_after["status"].as_u64(),
            Some(200),
            "the guest still writes locally; revocation cuts off storage, \
             not the local branch: {wrote_after}"
        );

        // Polled: the index is eventually consistent by design, so give
        // the guest every chance to get its write through.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let _ = post_json(
                &guest,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?;
            let _ = post_json(
                &owner,
                &format!("/api/repository/{key}/branch/main/sync/pull"),
                serde_json::json!({}),
            )
            .await?;
            assert!(
                !owner_sees(&owner, &key, "after-revocation").await?,
                "a revoked invite still reached storage: the owner can see \
                 the guest's post-revocation write"
            );
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // The owner is unaffected: revoking one invite withdraws that
        // delegation, not the space.
        successful_body(
            "owner still reaches the space",
            &post_json(
                &owner,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?,
        );

        guest.quit().await?;
        owner.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_revokes_the_cli_device_from_the_browser(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let linked = link_cli(&driver, &env).await?;
        let listed = devices(&linked.profile, &env).await?;
        assert!(listed.status.success(), "devices failed: {}", listed.stderr);
        let listed_rows = device_rows(&listed.stdout)?;
        let cli_did = did_for_device(&listed_rows, "e2e terminal")
            .context("CLI device was absent from the account device list")?
            .to_string();

        driver.goto(env.tonk_web.join("account")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text_containing(&driver, "#account-device-list", "e2e terminal").await?;
        let selector = format!("#account-device-list button[data-revoke=\"{cli_did}\"]");
        click(&driver, &selector).await?;
        driver.accept_alert().await?;
        wait_for_text_containing(&driver, "#account-working", "Access removed").await?;

        // The row leaves with the authority: revoking retracted the
        // link's facts from the account space, and the refreshed list no
        // longer shows the device. Storage enforcement of the published
        // revocation is pinned by the native access-service tests.
        wait_for_text_without(&driver, "#account-device-list", "e2e terminal").await?;

        // The revoked CLI still answers locally — the list is facts, not
        // a service round trip — but it can no longer pull the account,
        // so its own stale row is all it has left of the retraction.
        let listed = devices(&linked.profile, &env).await?;
        assert!(listed.status.success(), "devices failed: {}", listed.stderr);
        let listed_rows = device_rows(&listed.stdout)?;
        assert!(
            listed_rows
                .iter()
                .any(|row| row.name.starts_with("Chrome on ")),
            "{}",
            listed.stdout
        );

        driver.quit().await?;
        Ok(())
    }

    /// Link a second browser to an existing account the way a page from
    /// before the encryption key existed did: the same unlock ceremony and
    /// the same two saves, but the root is stored WITHOUT the key. The
    /// account's virtual authenticator must already hold the passkey.
    async fn legacy_link(driver: &WebDriver, env: &TestEnvironment) -> Result<()> {
        let identify = get_json(driver, "/api/identify").await?;
        let device_did = successful_body("identify", &identify)["did"]
            .as_str()
            .context("identify omitted the device DID")?
            .to_string();
        let ceremony = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const [deviceDid, service] = arguments;
                window.tonkIdentity.unlockWithPasskey({
                    deviceDid,
                    deviceName: "Legacy Chrome",
                    endpoint: `${window.location.origin}/ucan/`,
                }).then(async ceremony => {
                    const bytes = Uint8Array.from(
                        ceremony.invocationHex.match(/../g).map(pair => parseInt(pair, 16)),
                    );
                    const response = await fetch(`${service}devices/link`, {
                        method: "POST",
                        headers: { "content-type": "application/cbor" },
                        body: bytes,
                    });
                    const linked = await response.json();
                    done({ status: response.status, ceremony, linked });
                }).catch(error => done({ error: String(error) }));
                "#,
                vec![
                    serde_json::json!(device_did),
                    serde_json::json!(env.account_service.as_str()),
                ],
            )
            .await?
            .json()
            .clone();
        anyhow::ensure!(
            ceremony.get("error").is_none() && ceremony["status"] == 200,
            "legacy link ceremony failed: {ceremony}"
        );
        anyhow::ensure!(
            ceremony["ceremony"]["encryptionKey"].is_string(),
            "the unlock ceremony derives the key; the legacy page just never saved it: {ceremony}"
        );
        let saved = post_json(
            driver,
            "/api/identity/root",
            serde_json::json!({
                "credentialId": ceremony["ceremony"]["credentialId"],
                "delegationHex": ceremony["ceremony"]["delegationHex"],
            }),
        )
        .await?;
        successful_body("legacy root save", &saved);
        let attached = post_json(
            driver,
            "/api/account/attach",
            serde_json::json!({
                "provider": env.account_service.as_str(),
                "rootDid": ceremony["ceremony"]["rootDid"],
                "credentialId": ceremony["ceremony"]["credentialId"],
                "delegationHex": ceremony["ceremony"]["delegationHex"],
                "descriptorHex": ceremony["linked"]["descriptorHex"],
                "initializeName": false,
            }),
        )
        .await?;
        successful_body("legacy attach", &attached);
        Ok(())
    }

    /// Poll a JSON GET until `accept` says the body is what we wait for.
    async fn poll_json(
        driver: &WebDriver,
        path: &str,
        what: &str,
        accept: impl Fn(&serde_json::Value) -> bool,
    ) -> Result<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let response = get_json(driver, path).await?;
            if response.get("error").is_none() && accept(&response["body"]) {
                return Ok(response["body"].clone());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for {what}: {response}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// A device linked before the account's encryption key existed has
    /// nothing to seal a new space's seed to. Creating one from a page makes
    /// the worker ask that page for a passkey assertion; the page answers by
    /// saving the key with the root, and the create resumes. The space ends
    /// up custodied under the account, and the device now carries the key.
    #[dialog_common::test]
    async fn it_asks_the_page_for_a_passkey_assertion_when_custody_needs_the_key(
        env: TestEnvironment,
    ) -> Result<()> {
        let creator = driver_with_prf(&env).await?;
        sign_up(&creator, &env, EMAIL).await?;
        // A second device on the same account, in the same session so the
        // virtual authenticator still holds the passkey: "Add account"
        // rotates the worker onto a fresh profile with no root of its own,
        // which is exactly what a new browser is.
        let added = post_json(&creator, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add profile", &added);
        creator.goto(env.tonk_web.as_str()).await?;
        legacy_link(&creator, &env).await?;

        let root = get_json(&creator, "/api/identity/root").await?;
        let root = successful_body("root status", &root);
        assert_eq!(root["status"], "ready");
        assert!(
            root.get("encryptionKey").is_none(),
            "a legacy link records no key: {root}"
        );

        // Create through the profile branch, the way the FAB does: a
        // transient the worker runs post-commit, with this page as the
        // originating client the worker can ask.
        let created = post_json(
            &creator,
            "/api/profile/branch/main/transact",
            serde_json::json!({
                "claims": [{
                    "op": "assert",
                    "application": {
                        "predicate": {
                            "kind": "transient",
                            "concept": {
                                "description": "A request to create a new space from the wizard form.",
                                "with": {
                                    "name":     { "the": "dom.event.current-target.elements.name/value", "as": "Text" },
                                    "remote":   { "the": "dom.event.current-target.elements.remote/value", "as": "Text" },
                                    "template": { "the": "dom.event.current-target.elements.template/value", "as": "Text" }
                                }
                            }
                        },
                        "parameters": {
                            "name": "Custodied After Assertion",
                            "remote": env.tonk_web.join("ucan/")?,
                            "template": "blank"
                        }
                    }
                }]
            }),
        )
        .await?;
        successful_body("create space command", &created);

        // Two correct endings race from here. Either the worker needs
        // this page's assertion — it raises the consent card and waits
        // on its button — or the account pull has already delivered the
        // encryption key the first profile published, and the worker
        // seals straight to it without asking. Which side wins is
        // timing, not behaviour, so drive whichever happens: click the
        // card whenever it shows, and wait on the durable outcome — the
        // space exists and its seed is custodied.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut asserted_through_the_card = false;
        let key = loop {
            if let Ok(button) = creator.find(By::Css("#tonk-custody-continue")).await {
                // A card that re-renders between find and click is "not
                // yet", the same staleness `click` absorbs elsewhere.
                if button.click().await.is_ok() {
                    asserted_through_the_card = true;
                }
            }
            if let Ok(profile) = get_json(&creator, "/api/profile").await
                && profile.get("error").is_none()
                && let Some(key) = profile["body"]["space"]
                    .as_array()
                    .and_then(|spaces| spaces.first())
                    .and_then(|space| space["key"].as_str())
            {
                break key.to_string();
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the create finished neither way: no consent card appeared \
                 and no space was recorded"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };

        // Whichever path ran, the new space's seed ends up sealed to the
        // account's X25519 recipient. The custody fact follows the seal,
        // so poll for it rather than assert on the first read.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let recipient = loop {
            let rows = post_json(
                &creator,
                "/api/profile/branch/main/query",
                serde_json::json!({
                    "terms": {
                        "this": { "?": { "name": "this" } },
                        "subject": { "?": { "name": "subject" } },
                        "recipient": { "?": { "name": "recipient" } }
                    },
                    "predicate": {
                        "with": {
                            "subject": { "the": "xyz.tonk.custody/subject", "cardinality": "one", "as": "Entity" },
                            "recipient": { "the": "xyz.tonk.custody/recipient", "cardinality": "one", "as": "Entity" }
                        }
                    }
                }),
            )
            .await?;
            let rows = rows["body"].as_array().cloned().unwrap_or_default();
            if let Some(sealed_to) = rows.iter().find_map(|row| {
                let subject = row["fields"]["subject"].as_str().unwrap_or_default();
                let sealed_to = row["fields"]["recipient"].as_str().unwrap_or_default();
                (subject.ends_with(&key) && !sealed_to.is_empty()).then(|| sealed_to.to_string())
            }) {
                break sealed_to;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the new space's seed was never custodied: {rows:?}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        assert!(recipient.starts_with("did:key:z6LS"), "{recipient}");

        // The card path exists to record the key on the device root —
        // that is what the assertion was for — and it must be the same
        // recipient the seed was sealed to. The direct-seal path leaves
        // the legacy root record keyless by design.
        if asserted_through_the_card {
            let root = poll_json(
                &creator,
                "/api/identity/root",
                "the assertion to record the key",
                |body| body.get("encryptionKey").is_some(),
            )
            .await?;
            assert_eq!(
                root["encryptionKey"].as_str(),
                Some(recipient.as_str()),
                "the assertion's key and the seed's recipient must be the \
                 same account key: {root}"
            );
        }

        creator.quit().await?;
        Ok(())
    }
}
