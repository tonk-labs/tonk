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

    async fn wait_for_value(driver: &WebDriver, selector: &str, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && found.prop("value").await?.as_deref() == Some(expected)
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for `{selector}` value to equal {expected:?}"
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

    /// Enter the opaque Hub frame after first restoring the top browsing
    /// context. Callers that navigate or inspect top-document account UI must
    /// call `enter_default_frame` again first.
    async fn enter_hub(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        let frame = element(driver, "tonk-site > iframe").await?;
        frame.enter_frame().await?;
        element(driver, ".hub-page").await?;
        Ok(())
    }

    async fn wait_for_displayed(driver: &WebDriver, selector: &str) -> Result<WebElement> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(found) = driver.find(By::Css(selector.to_string())).await
                && found.is_displayed().await.unwrap_or(false)
            {
                return Ok(found);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to be displayed"
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
        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
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
        // The ceremony pre-signed the custody publish and the worker
        // drains it on activation — no page, no click. All that is
        // left is to wait for the queue to empty.
        poll_json(
            driver,
            "/api/customer/pending",
            "the queued custody publish to drain",
            |body| body.as_array().is_some_and(|queue| queue.is_empty()),
        )
        .await?;
        // Back to where the caller was: activation is a detour, not a
        // navigation the caller asked for.
        driver.goto(account.as_str()).await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_redirects_legacy_account_routes_without_losing_the_query(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let mut legacy = env.tonk_web.join("account")?;
        legacy.set_query(Some("next=%2Fspace%2Fdid%3Akey%3AzOne&add=1"));
        driver.goto(legacy.as_str()).await?;
        element(&driver, "tonk-account").await?;
        let current = driver.current_url().await?;
        assert_eq!(current.path(), "/settings");
        assert_eq!(current.query(), legacy.query());

        let mut legacy_link = env.tonk_web.join("account/link")?;
        legacy_link.set_query(Some(
            "audience=did%3Akey%3AzCli&callback=http%3A%2F%2F127.0.0.1%3A9999&name=terminal",
        ));
        driver.goto(legacy_link.as_str()).await?;
        element(&driver, "tonk-account").await?;
        let current = driver.current_url().await?;
        assert_eq!(current.path(), "/settings/link");
        assert_eq!(current.query(), legacy_link.query());

        driver.quit().await?;
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

        let display_name = element(&driver, "#account-display-name").await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Meta + "a"
        } else {
            Key::Control + "a"
        };
        display_name.send_keys(select_all).await?;
        display_name.send_keys("Settings Name").await?;
        display_name.send_keys(Key::Enter).await?;
        wait_for_value(&driver, "#account-display-name", "Settings Name").await?;
        let settings = driver.current_url().await?;
        driver.goto(settings.as_str()).await?;
        wait_for_value(&driver, "#account-display-name", "Settings Name").await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_matches_hub_tokens_and_settings_geometry(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "geometry@example.com").await?;

        const TOKEN_SCRIPT: &str = r#"
            const dark = arguments[0];
            const root = document.documentElement;
            root.classList.toggle('wa-dark', dark);
            const surface = document.querySelector(arguments[1]);
            const keys = ['--page','--ink','--on-ink','--soft','--ring','--sep',
              '--frost-solid','--panel','--wash','--wash-2'];
            const probe = document.createElement('span');
            probe.style.display = 'none';
            surface.appendChild(probe);
            const values = Object.fromEntries(keys.map(key => {
              probe.style.color = `var(${key})`;
              return [key, getComputedStyle(probe).color];
            }));
            probe.remove();
            return values;
        "#;

        driver.enter_default_frame().await?;
        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        let settings_light = driver
            .execute(
                TOKEN_SCRIPT,
                vec![serde_json::json!(false), serde_json::json!("tonk-account")],
            )
            .await?
            .json()
            .clone();
        let settings_dark = driver
            .execute(
                TOKEN_SCRIPT,
                vec![serde_json::json!(true), serde_json::json!("tonk-account")],
            )
            .await?
            .json()
            .clone();

        driver.goto(env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        let hub_light = driver
            .execute(
                TOKEN_SCRIPT,
                vec![serde_json::json!(false), serde_json::json!(".hub-page")],
            )
            .await?
            .json()
            .clone();
        let hub_dark = driver
            .execute(
                TOKEN_SCRIPT,
                vec![serde_json::json!(true), serde_json::json!(".hub-page")],
            )
            .await?
            .json()
            .clone();
        assert_eq!(
            settings_light, hub_light,
            "light settings tokens drifted from Hub"
        );
        assert_eq!(
            settings_dark, hub_dark,
            "dark settings tokens drifted from Hub"
        );
        let hub_heights = driver
            .execute(
                r#"document.querySelector('[data-open-settings]').click();
                    const body = document.querySelector('.hub-settings__body');
                    document.querySelector('[data-settings-tab="account"]').click();
                    const account = Math.round(body.getBoundingClientRect().height);
                    document.querySelector('[data-settings-tab="devices"]').click();
                    const devices = Math.round(body.getBoundingClientRect().height);
                    return {account, devices};"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            hub_heights.json()["account"],
            hub_heights.json()["devices"],
            "Hub Account and Devices tabs must keep one panel height"
        );

        driver.enter_default_frame().await?;
        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        for (window_width, expected_total, expected_rail, expected_body) in
            [(1200, 576, 144, 432), (607, 432, 108, 324)]
        {
            driver.set_window_rect(0, 0, window_width, 900).await?;
            let geometry = driver
                .execute(
                    r#"const settings = document.querySelector('.account__settings').getBoundingClientRect();
                        const rail = document.querySelector('.account__rail').getBoundingClientRect();
                        const body = document.querySelector('.account__settings-body').getBoundingClientRect();
                        document.querySelector('#account-tab-account').click();
                        const accountHeight = Math.round(document.querySelector('.account__settings-body').getBoundingClientRect().height);
                        document.querySelector('#account-tab-devices').click();
                        const devicesHeight = Math.round(document.querySelector('.account__settings-body').getBoundingClientRect().height);
                        const error = document.querySelector('#account-error');
                        error.hidden = false;
                        const errorRight = Math.round(error.getBoundingClientRect().right);
                        error.hidden = true;
                        const logo = document.querySelector('.account__logo').getBoundingClientRect();
                        return {
                          settings: Math.round(settings.width),
                          rail: Math.round(rail.width),
                          body: Math.round(body.width),
                          accountHeight,
                          devicesHeight,
                          bodyRight: Math.round(body.right),
                          errorRight,
                          logoVisible: logo.width > 0 && logo.height > 0
                        };"#,
                    Vec::new(),
                )
                .await?;
            let geometry = geometry.json();
            assert_eq!(
                geometry["settings"], expected_total,
                "settings geometry drifted at {window_width}px"
            );
            assert_eq!(geometry["rail"], expected_rail);
            assert_eq!(geometry["body"], expected_body);
            assert_eq!(
                geometry["accountHeight"], geometry["devicesHeight"],
                "Account and Devices tabs must keep one panel height at {window_width}px"
            );
            assert_eq!(
                geometry["errorRight"], geometry["bodyRight"],
                "settings notices must align with the panel body at {window_width}px"
            );
            assert_eq!(geometry["logoVisible"], true);
        }

        driver.set_window_rect(0, 0, 390, 844).await?;
        let compact = driver
            .execute(
                r#"const settings = document.querySelector('.account__settings');
                    const rail = document.querySelector('.account__rail');
                    const body = document.querySelector('.account__settings-body');
                    const visible = [...document.querySelectorAll('button,a,input')]
                      .filter(el => el.offsetParent !== null);
                    return {
                      settings: Math.round(settings.getBoundingClientRect().width),
                      rail: Math.round(rail.getBoundingClientRect().width),
                      body: Math.round(body.getBoundingClientRect().width),
                      viewport: innerWidth,
                      overflow: document.documentElement.scrollWidth > innerWidth,
                      undersized: visible.filter(el => {
                        const rect = el.getBoundingClientRect();
                        return Math.max(rect.width, rect.height) < 44;
                      }).map(el => el.id || el.textContent.trim())
                    };"#,
                Vec::new(),
            )
            .await?;
        let compact = compact.json();
        let available = compact["viewport"].as_i64().unwrap_or_default() - 32;
        assert_eq!(compact["settings"], available);
        assert_eq!(compact["rail"], available);
        assert_eq!(compact["body"], available);
        assert_eq!(compact["overflow"], false);
        assert_eq!(compact["undersized"], serde_json::json!([]));

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_explains_email_verification_before_account_sync(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        enroll_only(&driver, &env, "verify-first@example.com").await?;

        let notice = element(&driver, "#account-error").await?.text().await?;
        assert!(
            notice.contains("verification link"),
            "pending setup should direct the person to the verification email: {notice:?}"
        );
        assert!(
            !notice.contains("hydration") && !notice.contains("could not be synchronized"),
            "pending setup should not expose account-state implementation terms: {notice:?}"
        );
        assert!(
            !notice.contains("reload /settings"),
            "email verification should be the only requested next step: {notice:?}"
        );

        let display_name = element(&driver, "#account-display-name").await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Meta + "a"
        } else {
            Key::Control + "a"
        };
        display_name.send_keys(select_all).await?;
        display_name.send_keys("Pending Name").await?;
        display_name.send_keys(Key::Enter).await?;

        wait_for_text_containing(&driver, "#account-display-name-error", "verification link")
            .await?;
        let error = element(&driver, "#account-display-name-error")
            .await?
            .text()
            .await?;
        assert!(
            error.contains("verify your email"),
            "display-name failure should explain the required account step: {error:?}"
        );
        for technical in [
            "Error from local API",
            "503 Service Unavailable",
            "account_state_unavailable",
        ] {
            assert!(
                !error.contains(technical),
                "display-name failure should not expose {technical:?}: {error:?}"
            );
        }

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_styles_account_activation_as_a_fabb_ceremony(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let mut activation = env.tonk_web.join("activate")?;
        activation.set_query(Some("ucan=AA"));
        driver.goto(activation.as_str()).await?;
        element(&driver, "tonk-activate #activate-confirm").await?;

        assert!(
            driver.find(By::Css(".account__brand")).await.is_err(),
            "activation should use the same unbadged Tonk wordmark as settings"
        );
        assert!(
            driver.find(By::Css(".account__badge")).await.is_err(),
            "activation should not retain the retired page badge"
        );

        driver.set_window_rect(0, 0, 1200, 900).await?;
        let desktop = driver
            .execute(
                r#"document.documentElement.classList.remove('wa-dark');
                    document.documentElement.classList.add('wa-light');
                    const host = document.querySelector('tonk-activate');
                    const main = document.querySelector('.account').getBoundingClientRect();
                    const ceremony = document.querySelector('.account__ceremony').getBoundingClientRect();
                    const logo = document.querySelector('.account__logo').getBoundingClientRect();
                    const action = document.querySelector('#activate-accept').getBoundingClientRect();
                    const styles = getComputedStyle(host);
                    return {
                      hostDisplay: styles.display,
                      hostHeight: Math.round(host.getBoundingClientRect().height),
                      viewportHeight: innerHeight,
                      page: styles.backgroundColor,
                      mainWidth: Math.round(main.width),
                      mainCenter: Math.round(main.left + main.width / 2),
                      viewportCenter: Math.round(innerWidth / 2),
                      ceremonyWidth: Math.round(ceremony.width),
                      logoWidth: Math.round(logo.width),
                      actionHeight: Math.round(action.height),
                      heading: document.querySelector('.account__ceremony-head')?.textContent.trim(),
                      overflow: document.documentElement.scrollWidth > innerWidth
                    };"#,
                Vec::new(),
            )
            .await?;
        let desktop = desktop.json();
        assert_eq!(desktop["hostDisplay"], "grid");
        assert_eq!(desktop["hostHeight"], desktop["viewportHeight"]);
        assert_eq!(desktop["page"], "rgb(236, 236, 236)");
        assert_eq!(desktop["mainWidth"], 576);
        assert_eq!(desktop["mainCenter"], desktop["viewportCenter"]);
        assert_eq!(desktop["ceremonyWidth"], 432);
        assert_eq!(desktop["logoWidth"], 132);
        assert_eq!(desktop["actionHeight"], 44);
        assert_eq!(desktop["heading"], "activate your account");
        assert_eq!(desktop["overflow"], false);

        let done = driver
            .execute(
                r#"document.querySelector('#activate-confirm').hidden = true;
                    document.querySelector('#activate-done').hidden = false;
                    const row = document.querySelector('#activate-done .account__row').getBoundingClientRect();
                    const action = document.querySelector('#activate-done .account__run').getBoundingClientRect();
                    return {
                      heading: document.querySelector('#activate-done-title').textContent.trim(),
                      rowWidth: Math.round(row.width),
                      actionHeight: Math.round(action.height)
                    };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(done.json()["heading"], "account activated");
        assert_eq!(done.json()["rowWidth"], 432);
        assert_eq!(done.json()["actionHeight"], 44);

        driver.set_window_rect(0, 0, 390, 844).await?;
        let compact = driver
            .execute(
                r#"const main = document.querySelector('.account').getBoundingClientRect();
                    const ceremony = document.querySelector('.account__ceremony').getBoundingClientRect();
                    const visible = [...document.querySelectorAll('button,a,input')]
                      .filter(el => el.offsetParent !== null);
                    return {
                      viewport: innerWidth,
                      mainWidth: Math.round(main.width),
                      ceremonyWidth: Math.round(ceremony.width),
                      logoWidth: Math.round(document.querySelector('.account__logo').getBoundingClientRect().width),
                      overflow: document.documentElement.scrollWidth > innerWidth,
                      undersized: visible.filter(el => {
                        const rect = el.getBoundingClientRect();
                        return Math.max(rect.width, rect.height) < 44;
                      }).map(el => el.id || el.textContent.trim())
                    };"#,
                Vec::new(),
            )
            .await?;
        let compact = compact.json();
        let available = compact["viewport"].as_i64().unwrap_or_default() - 32;
        assert_eq!(compact["mainWidth"], available);
        assert_eq!(compact["ceremonyWidth"], available);
        assert_eq!(compact["logoWidth"], 98);
        assert_eq!(compact["overflow"], false);
        assert_eq!(compact["undersized"], serde_json::json!([]));

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_back_into_the_same_account_after_signing_out(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        click(&driver, "#account-unlink").await?;
        element(&driver, "[role=alertdialog]").await?;
        click(&driver, "#account-delete-submit").await?;
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
        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
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
            env.tonk_web.join("settings/link")?.as_str(),
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
        assert_eq!(approval_url.path(), "/settings/link");
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

    /// Await a fact appearing on a branch, by SUBSCRIPTION rather than
    /// by polling.
    ///
    /// `POST .../query` with `Accept: text/event-stream` opens a live
    /// subscription: a `snapshot` frame with what already matches, then
    /// an `update` frame for every change. The browser holds it open and
    /// resolves as soon as a frame satisfies `predicate`, so the test
    /// waits on the same notification the app does instead of asking
    /// again on a timer.
    ///
    /// `timeout_ms` bounds the wait so a fact that never arrives fails
    /// with a message rather than hanging the suite. That is a deadline,
    /// not an interval: nothing re-checks on a clock.
    async fn await_subscription(
        driver: &WebDriver,
        path: &str,
        query: serde_json::Value,
        predicate: &str,
        timeout_ms: u64,
    ) -> Result<serde_json::Value> {
        let result = driver
            .execute_async(
                r#"
                const [path, query, predicateSource, timeoutMs, done] = [
                    arguments[0], arguments[1], arguments[2], arguments[3],
                    arguments[arguments.length - 1],
                ];
                const matches = new Function("frame", predicateSource);
                let settled = false;
                const settle = (value) => {
                    if (settled) return;
                    settled = true;
                    try { controller.abort(); } catch (_) {}
                    done(value);
                };
                const controller = new AbortController();
                const timer = setTimeout(
                    () => settle({ error: "timed out waiting for the subscription" }),
                    timeoutMs,
                );
                fetch(path, {
                    method: "POST",
                    headers: {
                        "content-type": "application/json",
                        accept: "text/event-stream",
                    },
                    body: JSON.stringify(query),
                    signal: controller.signal,
                }).then(async (response) => {
                    if (!response.ok) {
                        return settle({ error: "subscribe failed: " + response.status });
                    }
                    const reader = response.body.getReader();
                    const decoder = new TextDecoder();
                    let buffer = "";
                    for (;;) {
                        const { value, done: finished } = await reader.read();
                        if (finished) {
                            return settle({ error: "the subscription ended early" });
                        }
                        buffer += decoder.decode(value, { stream: true });
                        let cut;
                        while ((cut = buffer.indexOf("\n\n")) !== -1) {
                            const chunk = buffer.slice(0, cut);
                            buffer = buffer.slice(cut + 2);
                            const line = chunk
                                .split("\n")
                                .find((l) => l.startsWith("data:"));
                            if (!line) continue;
                            let frame;
                            try {
                                frame = JSON.parse(line.slice(5).trim());
                            } catch (_) {
                                continue;
                            }
                            if (matches(frame)) {
                                clearTimeout(timer);
                                return settle({ frame });
                            }
                        }
                    }
                }).catch((error) => {
                    if (!settled) settle({ error: String(error) });
                });
                "#,
                vec![
                    serde_json::json!(path),
                    query,
                    serde_json::json!(predicate),
                    serde_json::json!(timeout_ms),
                ],
            )
            .await?;
        let value = result.json().clone();
        if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
            return Err(anyhow!("{error} (subscribing to {path})"));
        }
        Ok(value["frame"].clone())
    }

    /// The `account/check-email` claim, in the shape the registration
    /// form dispatches it.
    fn check_email_claim_json(email: &str) -> serde_json::Value {
        serde_json::json!({
            "claims": [{
                "op": "assert",
                "application": {
                    "predicate": {
                        "kind": "transient",
                        "concept": {
                            "description": "Ask whether an address is registered.",
                            "with": {
                                "email": {
                                    "the": "dom.event.current-target.elements.email/value",
                                    "as": "Text"
                                }
                            }
                        }
                    },
                    "parameters": { "email": email }
                }
            }]
        })
    }

    /// Asking about an address answers on the overlay, and the answer
    /// says which of create / sign-in the form should offer.
    ///
    /// The unit tests cover the status mapping with no service in sight.
    /// This is the part they cannot see: the command decodes, the lookup
    /// reaches a real access service, and the answer lands somewhere the
    /// form can subscribe to.
    #[dialog_common::test]
    async fn it_answers_whether_an_address_is_registered(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        // A profile exists from first boot, so nothing has to be signed
        // in for the form to ask this.
        get_json(&driver, "/api/profile").await?;

        let unknown = "nobody-has-this@example.com";
        let dispatched = post_json(
            &driver,
            "/api/profile/branch/main/transact",
            check_email_claim_json(unknown),
        )
        .await?;
        successful_body("dispatch account/check-email", &dispatched);

        let answered = await_email_status(&driver, unknown).await?;
        assert_eq!(
            answered, "unregistered",
            "an address nobody registered is the create-an-account branch",
        );

        // Now one that IS registered: the same question, the other
        // answer, so the form offers sign-in instead of a ceremony that
        // would fail at the end.
        let taken = "taken@example.com";
        sign_up(&driver, &env, taken).await?;
        let dispatched = post_json(
            &driver,
            "/api/profile/branch/main/transact",
            check_email_claim_json(taken),
        )
        .await?;
        successful_body("dispatch account/check-email", &dispatched);

        let answered = await_email_status(&driver, taken).await?;
        assert_eq!(
            answered, "active",
            "a registered address is the sign-in branch, not a second signup",
        );

        driver.quit().await?;
        Ok(())
    }

    /// Read the overlay answer for `address`, waiting for the row that
    /// names it rather than whichever row happens to be there.
    async fn await_email_status(driver: &WebDriver, address: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let rows = post_json(
                driver,
                "/api/profile/branch/main/query",
                tonk_worker::helpers::email_status_wire_query(),
            )
            .await?;
            if let Some(found) = rows["body"].as_array().and_then(|rows| {
                rows.iter().find(|row| {
                    row["address"].as_str() == Some(address)
                        || row["fields"]["address"].as_str() == Some(address)
                })
            }) {
                let state = found["state"]
                    .as_str()
                    .or_else(|| found["fields"]["state"].as_str())
                    .unwrap_or_default();
                if !state.is_empty() {
                    return Ok(state.to_owned());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("no email-status answer for {address}: {rows}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// The Hub's own wizard creates a local-only spot before anyone
    /// registers.
    ///
    /// Every other test here builds the claim in Rust, which skips the
    /// form entirely — so a hidden input that prefills a remote is
    /// invisible to them. This one submits the real wizard, which is how
    /// `<tonk-default-remote auto>` went on wiring `origin + /ucan/`
    /// onto spots created with no account: the form supplied a remote,
    /// the worker honoured it as a deliberate choice, and the gate that
    /// keeps a spot local never got a say. The spot then synced to a
    /// service that refuses to serve it.
    #[dialog_common::test]
    async fn it_creates_a_local_only_spot_from_the_hub_wizard(env: TestEnvironment) -> Result<()> {
        // The authenticator id comes along so the ceremony can be
        // observed: a passkey either got minted or it did not.
        let (driver, authenticator) = driver_with_prf_authenticator(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        // Nothing is registered: a device has an account from first
        // boot, but no provider serves it until someone signs up.
        let customer = get_json(&driver, "/api/customer").await?;
        assert!(
            customer["body"]["provider"].as_str().is_none(),
            "this profile must not be served yet: {customer}",
        );

        let before = space_keys(&driver).await?;
        submit_hub_wizard(&driver).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let key = loop {
            let now = space_keys(&driver).await?;
            if let Some(key) = now.iter().find(|key| !before.contains(key)) {
                break key.clone();
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the wizard never created a spot; before={before:?} now={now:?}",
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };

        // Give the handler's post-navigation attach step room to run, so
        // "no remote" means it declined rather than that we looked early.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the spot configuration", &info);
        assert!(
            info["remote"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "a spot created before registering must wire no remote, got {}",
            info["remote"],
        );
        assert!(
            info["branch"]["main"]["upstream"].is_null(),
            "main must track nothing, got {}",
            info["branch"]["main"]["upstream"],
        );

        // The spot is local-only, which is what makes sharing it
        // refuse. Walk the rest of the flow from that refusal, asserting
        // at each step on WHAT THE USER SEES rather than on the fact
        // behind it.
        //
        // That distinction is the whole point of these steps. The
        // command is already covered by
        // `it_answers_whether_an_address_is_registered`, which polls the
        // worker's row directly — and therefore passes whether or not
        // anything ever renders the answer. The dialog shipped with a
        // write and no read, latching on "Checking…" forever, and that
        // test stayed green throughout.
        // The bar offers the account row, not the copy row: nothing is
        // registered. That is the visible half of the account
        // subscription — when its query failed, no frame ever arrived
        // and the bar sat on this row even after someone registered.
        await_share_row(&driver, "account").await?;
        open_register_dialog(&driver).await?;

        // Nothing is offered until the lookup answers. A ceremony
        // started before that runs creation against an address that
        // might already have an account, which fails at the end and
        // leaves an orphan passkey.
        let idle = register_action_label(&driver).await?;
        assert!(
            idle.is_empty(),
            "the action row must stay folded until the answer, got {idle:?}",
        );

        type_into_register_dialog(&driver, "nobody@example.com").await?;
        let label = await_register_action(&driver, "create a passkey").await?;
        assert_eq!(
            label, "create a passkey",
            "an address nobody registered is the create branch",
        );

        // The lookup itself must NOT have run a ceremony. `check-email`
        // and `account/register` are the same shape, and before the
        // marker every keystroke's lookup also decoded as a
        // registration — so a passkey prompt appeared while the user was
        // still typing.
        let typed = credential_count(&driver, &authenticator).await?;
        assert_eq!(
            typed, 0,
            "typing an address must not mint a passkey, got {typed}",
        );

        // Clicking it must actually RUN a ceremony. A successful
        // transact only means the command was accepted; the worker then
        // asks the page to run WebAuthn. When nothing on the page
        // listened for that request the dialog still reported success,
        // so the credential count is what tells the difference.
        click_register_action(&driver).await?;
        let after = await_credential_count(&driver, &authenticator, 1).await?;
        assert_eq!(after, 1, "the ceremony mints a passkey");

        // ...and the share it interrupted must finish, which is the
        // feature: the spot gains the remote it refused to share
        // without, and the invite link arrives.
        await_share_link(&driver).await?;

        driver.quit().await?;
        Ok(())
    }

    /// The account subscription's query actually returns rows.
    ///
    /// It shipped without binding `this`, which is a query ERROR rather
    /// than a wildcard: every attempt failed with `UnboundVariable` and
    /// no frame ever arrived. Nothing said so — the bar simply fell back
    /// to its defaults and reported stale sync, kept offering "log in to
    /// share" to an active account, and pushed toward creating a second
    /// one. Three symptoms, one missing term, no error anywhere.
    #[dialog_common::test]
    async fn it_reads_the_account_state_the_bar_subscribes_to(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "subscribed@example.com").await?;

        let rows = post_json(
            &driver,
            "/api/profile/branch/main/query",
            // The bar's query, inlined: `tonk-ui` does not depend on
            // `tonk-fab`. Pinned to it by
            // `logic::account_state_query::it_binds_its_subject`, which
            // asserts the same shape from the other side.
            serde_json::json!({
                "predicate": { "with": {
                    "status": {
                        "the": "xyz.tonk.account/customer-status",
                        "as": "Text", "cardinality": "one"
                    },
                    "provider": {
                        "the": "xyz.tonk.account/provider-address",
                        "as": "Text", "cardinality": "one"
                    }
                } },
                "terms": {
                    "this": { "?": { "name": "account" } },
                    "status": { "?": { "name": "status" } },
                    "provider": { "?": { "name": "provider" } }
                }
            }),
        )
        .await?;
        let rows = successful_body("read the account state", &rows);
        let rows = rows.as_array().context("the query answers with rows")?;
        assert!(
            !rows.is_empty(),
            "an activated account must resolve, got {rows:?}",
        );
        assert_eq!(
            rows[0]["fields"]["status"], "Active",
            "and say so: {rows:?}",
        );
        assert!(
            rows[0]["fields"]["provider"]
                .as_str()
                .is_some_and(|p| !p.is_empty()),
            "with the provider the service named: {rows:?}",
        );

        driver.quit().await?;
        Ok(())
    }

    /// The bar stops offering to log in once an account exists.
    ///
    /// The rendered half of the same subscription. Asserting on the row
    /// the user sees rather than on the fact behind it is what catches a
    /// query that silently answers nothing: the fact was right the whole
    /// time the bar was wrong.
    #[dialog_common::test]
    async fn it_offers_the_copy_row_once_an_account_exists(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        let key = create_space(&driver, "Shareable").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/did:key:{key}"))?.as_str())
            .await?;
        await_share_row(&driver, "account").await?;

        sign_up(&driver, &env, "bar-flips@example.com").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/did:key:{key}"))?.as_str())
            .await?;

        await_share_row(&driver, "link").await?;

        driver.quit().await?;
        Ok(())
    }

    /// Activating rewrites the answer about the address.
    ///
    /// `EmailStatus` was written only by the lookup handler, so an
    /// address checked BEFORE registering stayed `unregistered` in the
    /// overlay forever — and the form kept offering to create an account
    /// for one that had just finished activating. The order here is the
    /// point: check first, register second.
    #[dialog_common::test]
    async fn it_refreshes_the_address_answer_when_the_account_activates(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        // Ask about the address while nobody has it.
        let taken = "activates@example.com";
        let dispatched = post_json(
            &driver,
            "/api/profile/branch/main/transact",
            check_email_claim_json(taken),
        )
        .await?;
        successful_body("dispatch account/check-email", &dispatched);
        assert_eq!(
            await_email_status(&driver, taken).await?,
            "unregistered",
            "nobody has it yet",
        );

        // Now register and activate it.
        sign_up(&driver, &env, taken).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if await_email_status(&driver, taken).await? == "active" {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "activation never refreshed the answer; it still reads {:?}",
                await_email_status(&driver, taken).await?,
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        driver.quit().await?;
        Ok(())
    }

    /// An address that already has an account must offer to SIGN IN.
    ///
    /// Sending someone with an account through a creation ceremony
    /// leaves an orphan passkey in their authenticator and fails at the
    /// end, so the button has to route on the answer rather than assume.
    #[dialog_common::test]
    async fn it_offers_sign_in_for_an_address_that_already_has_an_account(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        let taken = "taken@example.com";
        sign_up(&driver, &env, taken).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        open_register_dialog(&driver).await?;
        type_into_register_dialog(&driver, taken).await?;
        let label = await_register_action(&driver, "log in with your passkey").await?;
        assert_eq!(
            label, "log in with your passkey",
            "a registered address must offer sign-in, not a second signup",
        );

        driver.quit().await?;
        Ok(())
    }

    /// A late answer must not render as an answer about what is typed
    /// now.
    ///
    /// The lookups are debounced and run concurrently, so an answer
    /// about a half-typed address can land after a later one. The row
    /// carries the address it is about precisely so the dialog can tell
    /// them apart.
    #[dialog_common::test]
    async fn it_ignores_an_answer_about_an_address_that_was_edited_away(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        let taken = "taken@example.com";
        sign_up(&driver, &env, taken).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        open_register_dialog(&driver).await?;

        // Ask about the registered address, then immediately edit to one
        // nobody has. The first answer ("Sign in") is in flight when the
        // second is asked for.
        type_into_register_dialog(&driver, taken).await?;
        type_into_register_dialog(&driver, "someone-else-entirely@example.com").await?;

        let label = await_register_action(&driver, "create a passkey").await?;
        assert_eq!(
            label, "create a passkey",
            "the dialog must answer about what is typed now, not what was typed before",
        );

        driver.quit().await?;
        Ok(())
    }

    /// The cluster's action row label, or empty while it is folded.
    async fn register_action_label(driver: &WebDriver) -> Result<String> {
        let label = driver
            .execute(
                r##"const a = document.querySelector("#tonk-register-action");
                   if (!a || a.hasAttribute("hidden")) return "";
                   return (a.textContent || "").trim();"##,
                Vec::new(),
            )
            .await?;
        Ok(label.json().as_str().unwrap_or_default().to_owned())
    }

    /// Click the cluster's action row.
    async fn click_register_action(driver: &WebDriver) -> Result<()> {
        let outcome = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                const a = document.querySelector("#tonk-register-action");
                if (!a) return done({ error: "no action row" });
                if (a.hasAttribute("hidden")) return done({ error: "the action row is folded" });
                a.click();
                done({ ok: true });
                "##,
                Vec::new(),
            )
            .await?;
        let value = outcome.json().clone();
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(anyhow!("could not run the step: {error}"));
        }
        Ok(())
    }

    /// Wait for the virtual authenticator to hold `expected` credentials.
    async fn await_credential_count(
        driver: &WebDriver,
        authenticator_id: &str,
        expected: usize,
    ) -> Result<usize> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut last = 0;
        loop {
            last = credential_count(driver, authenticator_id).await?;
            if last >= expected {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "no passkey ceremony ran: the authenticator still holds {last} \
                     credential(s), expected {expected}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait for the interrupted share to finish and hand over a link.
    async fn await_share_link(driver: &WebDriver) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let rows = post_json(
                driver,
                "/api/profile/branch/main/query",
                serde_json::json!({
                    "predicate": { "with": {
                        "link": {
                            "the": "xyz.tonk.credential/link",
                            "as": "Text", "cardinality": "one"
                        }
                    } },
                    "terms": { "link": { "?": { "name": "link" } } }
                }),
            )
            .await?;
            if let Some(link) = rows["body"].as_array().and_then(|rows| {
                rows.iter().find_map(|row| {
                    row["fields"]["link"]
                        .as_str()
                        .or_else(|| row["link"].as_str())
                        .filter(|link| !link.is_empty())
                })
            }) {
                return Ok(link.to_owned());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "registering never finished the share it interrupted: no invite link",
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Open the FAB's share menu and click a row in it.
    ///
    /// The bar lives in the profile frame and its rows are custom
    /// elements, so this reaches through the frame rather than clicking
    /// from the top document, which cannot see them.
    async fn click_share_row(driver: &WebDriver, marker: &str) -> Result<()> {
        let outcome = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                const marker = arguments[0];
                const roots = [document];
                for (const frame of document.querySelectorAll("iframe")) {
                    try { if (frame.contentDocument) roots.push(frame.contentDocument); } catch (e) {}
                }
                for (const root of roots) {
                    const bar = root.querySelector("tonk-fab");
                    if (!bar) continue;
                    // Open the share menu, then take the row it holds.
                    const share = bar.querySelector("[data-mi-share], [data-overflow-share]");
                    if (share) share.click();
                    const row = bar.querySelector(marker);
                    if (!row) continue;
                    row.click();
                    return done({ ok: true });
                }
                done({ error: "no bar row matching " + marker });
                "##,
                vec![serde_json::json!(marker)],
            )
            .await?;
        let value = outcome.json().clone();
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(anyhow!("could not click the share row: {error}"));
        }
        Ok(())
    }

    /// Which of the share menu's two rows the bar is offering.
    ///
    /// `log in to share` before an account exists, the copy row after —
    /// the visible half of the account subscription. Returns `None`
    /// while neither is showing.
    async fn share_row_offered(driver: &WebDriver) -> Result<Option<String>> {
        let outcome = driver
            .execute(
                r##"
                const roots = [document];
                for (const frame of document.querySelectorAll("iframe")) {
                    try { if (frame.contentDocument) roots.push(frame.contentDocument); } catch (e) {}
                }
                for (const root of roots) {
                    const bar = root.querySelector("tonk-fab");
                    if (!bar) continue;
                    const account = bar.querySelector("[data-share-account]");
                    const link = bar.querySelector("[data-share-link]");
                    if (account && !account.hasAttribute("hidden")) return "account";
                    if (link && !link.hasAttribute("hidden")) return "link";
                }
                return null;
                "##,
                Vec::new(),
            )
            .await?;
        Ok(outcome.json().as_str().map(str::to_owned))
    }

    /// Wait for the bar to offer `expected` (`account` or `link`).
    async fn await_share_row(driver: &WebDriver, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last = None;
        loop {
            last = share_row_offered(driver).await?;
            if last.as_deref() == Some(expected) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the bar never offered {expected:?}; it is showing {last:?}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Get to the registration cluster: open the share menu, take the
    /// "log in to share" row, wait for the cluster.
    async fn open_register_dialog(driver: &WebDriver) -> Result<()> {
        click_share_row(driver, "[data-share-account]").await?;
        await_register_dialog(driver).await
    }

    /// Wait for the registration cluster to be raised in the TOP page.
    ///
    /// It is raised there and nowhere else: WebAuthn needs a `window`
    /// and a user gesture, which neither the worker nor the profile
    /// frame has.
    async fn await_register_dialog(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let present = driver
                .execute(
                    r##"return !!document.querySelector("#tonk-register-email");"##,
                    Vec::new(),
                )
                .await?;
            if present.json().as_bool() == Some(true) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("the share refusal never raised the cluster"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Type an address into the cluster's own input, the way a user
    /// does.
    ///
    /// An `input` event is what the lookup debounces on, so setting
    /// `.value` alone would ask nothing.
    async fn type_into_register_dialog(driver: &WebDriver, address: &str) -> Result<()> {
        let outcome = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                const address = arguments[0];
                const field = document.querySelector("#tonk-register-email");
                if (!field) return done({ error: "no address field" });
                field.focus();
                field.value = address;
                field.dispatchEvent(new Event("input", { bubbles: true }));
                done({ ok: true });
                "##,
                vec![serde_json::json!(address)],
            )
            .await?;
        let value = outcome.json().clone();
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(anyhow!("could not type the address: {error}"));
        }
        Ok(())
    }

    /// Wait for the cluster's action row to offer `expected`.
    ///
    /// The row is hidden until the lookup answers, and its label IS the
    /// routing decision: "create a passkey" for an address nobody has,
    /// "log in with your passkey" for one that is taken. Waiting on it
    /// asserts the whole loop — command dispatched, answer written,
    /// subscription delivered, cluster rendered.
    async fn await_register_action(driver: &WebDriver, expected: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last = String::new();
        loop {
            let label = driver
                .execute(
                    r##"const a = document.querySelector("#tonk-register-action");
                       if (!a || a.hasAttribute("hidden")) return "";
                       return (a.textContent || "").trim();"##,
                    Vec::new(),
                )
                .await?;
            last = label.json().as_str().unwrap_or_default().to_owned();
            if last == expected {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the cluster never offered {expected:?}; it shows {last:?}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Drive the Hub's create wizard to a blank spot.
    ///
    /// The wizard pages with CSS-only radios (`#wiz-start` opens it,
    /// `#wiz-agent` is the blank path), and the Hub renders inside a
    /// sealed guest, so this reaches in through the frame rather than
    /// clicking from the top document — which cannot see those controls.
    async fn submit_hub_wizard(driver: &WebDriver) -> Result<()> {
        let outcome = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const frame = document.querySelector("iframe");
                const root = frame?.contentDocument;
                if (!root) return done({ error: "no guest frame" });
                const check = (id) => {
                    const radio = root.getElementById(id);
                    if (!radio) return false;
                    radio.checked = true;
                    radio.dispatchEvent(new Event("change", { bubbles: true }));
                    return true;
                };
                // Open the wizard, then take the blank path.
                if (!check("wiz-start")) return done({ error: "no #wiz-start" });
                if (!check("wiz-agent")) return done({ error: "no #wiz-agent" });
                // Submit the form itself: `wa-button[type=submit]` is a
                // custom element, so requestSubmit is what a click would
                // reach anyway and does not depend on it having upgraded.
                const form = root.querySelector("form.onb-overlay-body, form[onsubmit], form");
                if (!form) return done({ error: "no wizard form" });
                setTimeout(() => {
                    form.requestSubmit
                        ? form.requestSubmit()
                        : form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
                    done({ ok: true });
                }, 100);
                "#,
                Vec::new(),
            )
            .await?;
        let value = outcome.json().clone();
        if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
            return Err(anyhow!("could not drive the Hub wizard: {error}"));
        }
        Ok(())
    }

    /// Create a space the way the app does: dispatch the `space/create`
    /// transient and wait for the new key to appear in the profile.
    ///
    /// There is no creation endpoint to read a key from — a command's
    /// outcome lands as facts the page subscribes to, and the worker
    /// navigates the originating client itself — so a test discovers the
    /// key the way the Hub does, by watching the profile's space list.
    async fn create_space(driver: &WebDriver, name: &str) -> Result<String> {
        create_space_awaiting_remote(driver, name, false).await
    }

    /// [`create_space`], waiting for the remote to attach before
    /// returning.
    ///
    /// The handler creates the space, navigates the client, and attaches
    /// AFTER, so the navigation does not wait on the network — meaning
    /// the space appearing is not the attach having landed. A caller
    /// about to sync through that remote has to wait for it.
    async fn create_space_awaiting_remote(
        driver: &WebDriver,
        name: &str,
        expect_remote: bool,
    ) -> Result<String> {
        let before = space_keys(driver).await?;
        // `name` alone: where a space syncs is the worker's to resolve
        // from the account's registration, and template seeding went
        // with the template libraries.
        let claim = tonk_worker_api::create_space_claim_json(name);
        let dispatched = post_json(driver, "/api/profile/branch/main/transact", claim).await?;
        successful_body("dispatch space/create", &dispatched);

        // Subscribe for the replica rather than re-reading the profile
        // listing on a timer: the space lands as a `Replica` fact on
        // profile main, and the subscription delivers it on commit.
        let known = serde_json::to_string(&before).unwrap_or_else(|_| "[]".to_owned());
        await_subscription(
            driver,
            "/api/profile/branch/main/query",
            tonk_worker::helpers::replica_concept_wire_query(),
            &format!(
                r#"const before = new Set({known});
                   const rows = frame.conclusions || frame.asserted || [];
                   return rows.some((row) => {{
                       const text = JSON.stringify(row);
                       return [...text.matchAll(/did:key:[A-Za-z0-9]+/g)]
                           .some((m) => !before.has(m[0].slice("did:key:".length))
                                     && !before.has(m[0]));
                   }});"#
            ),
            30_000,
        )
        .await
        .context("the created space never appeared on profile main")?;

        // The subscription says a replica landed; the listing says which
        // key it is, in the shape callers use.
        let key = space_keys(driver)
            .await?
            .into_iter()
            .find(|key| !before.contains(key))
            .ok_or_else(|| anyhow!("a replica landed but no new key is listed"))?;

        // The handler navigates the client and attaches the remote
        // AFTER, so the navigation does not wait on the network. The
        // space appearing is therefore not the attach having landed —
        // and a caller that asked for a remote is about to sync through
        // it. Wait for it here so every caller does not have to.
        //
        // The endpoint this replaced attached synchronously before
        // answering, which is why no caller needed this before.
        if expect_remote {
            // Subscribe rather than poll: the attach lands as a `Remote`
            // fact on the profile branch, and the subscription delivers
            // it the moment it commits.
            await_subscription(
                driver,
                "/api/profile/branch/main/query",
                tonk_worker::helpers::remote_concept_wire_query(),
                &format!(
                    r#"const rows = frame.conclusions || frame.asserted || [];
                       return rows.some((row) =>
                           JSON.stringify(row).includes({key:?}));"#
                ),
                30_000,
            )
            .await
            .with_context(|| format!("the requested remote never attached to '{key}'"))?;
        }
        Ok(key)
    }

    /// Every space key this profile lists.
    async fn space_keys(driver: &WebDriver) -> Result<Vec<String>> {
        let listed = get_json(driver, "/api/profile").await?;
        Ok(listed["body"]["space"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry["key"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
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
                }).then(async response => {
                    // A body is not always JSON — an empty 200, or an
                    // error page from something upstream of the worker.
                    // Reporting the status with the raw text says what
                    // happened; `json()` alone throws and hides it.
                    const text = await response.text();
                    let body;
                    try { body = text ? JSON.parse(text) : null; }
                    catch (_) { body = { raw: text }; }
                    done({ status: response.status, body });
                }).catch(error => done({ error: String(error) }));
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

    fn active_profile_and_label(body: &serde_json::Value) -> Result<(String, String)> {
        let active = body["active"]
            .as_str()
            .context("profiles response omitted the active name")?;
        let entry = body["profiles"]
            .as_array()
            .and_then(|profiles| {
                profiles
                    .iter()
                    .find(|profile| profile["profileName"].as_str() == Some(active))
            })
            .context("profiles response omitted its active entry")?;
        let label = ["displayName", "email", "profileName"]
            .into_iter()
            .find_map(|field| {
                entry[field]
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
            })
            .context("active profile has no display label")?;
        Ok((active.to_string(), label.to_string()))
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
        let key = create_space(&driver, "Made While Waiting").await?;

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
        let key = create_space(&driver, "Made After Activation").await?;

        // The handler creates the space, navigates the client, and
        // attaches the remote AFTER — deliberately, so the navigation
        // does not wait on the network. The space existing is therefore
        // not the attach having landed, so poll rather than read once.
        await_subscription(
            &driver,
            "/api/profile/branch/main/query",
            tonk_worker::helpers::remote_concept_wire_query(),
            &format!(
                r#"const rows = frame.conclusions || frame.asserted || [];
                   return rows.some((row) => JSON.stringify(row).includes({key:?}));"#
            ),
            30_000,
        )
        .await
        .context("an activated account's space must wire the origin remote")?;
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the space configuration", &info);
        assert!(
            info["remote"]["origin"].is_object(),
            "the attached remote must be `origin`, got {}",
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

        let key = create_space(&driver, "Opted In").await?;

        // Creating a space navigates into it — the handler posts a
        // navigate effect to the originating client — so come back to
        // the account page before reading the panel. The endpoint this
        // replaced returned a key and navigated nothing.
        driver.goto(env.tonk_web.join("account")?.as_str()).await?;

        // Confirm the email, so a provider exists to attach to.
        activate(&driver, &env, email).await?;
        wait_for_text(&driver, "#account-registration-value", "Active").await?;

        // Still local: activation does not retroactively sync spaces
        // created before it. The user opts in per space.
        //
        // No settling wait is needed to make this meaningful. The
        // handler's attach step, had it run, would have run before
        // `create_space` returned — and the navigation, the whole
        // activation ceremony, and the panel wait have all happened
        // since. An absent remote here is a decision, not a race.
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

        let key = create_space_awaiting_remote(&creator, "Shared Garden", true).await?;
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
        claimer
            .goto(env.tonk_web.join("settings")?.as_str())
            .await?;
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

        let key = create_space_awaiting_remote(&driver, "Doomed Garden", true).await?;
        let pushed = post_json(
            &driver,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("push synced space", &pushed);

        click(&driver, "#account-delete-review").await?;
        element(&driver, "[role=alertdialog]").await?;
        wait_for_text(
            &driver,
            "#account-confirm-title",
            "delete account permanently",
        )
        .await?;
        click(&driver, "#account-confirm-cancel").await?;

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
        let key = create_space_awaiting_remote(&driver, "First Garden", true).await?;
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
        let profiles_before_add = successful_body("list profiles", &profiles);
        let profile_count_before_add = profiles_before_add["profiles"]
            .as_array()
            .context("profile roster is not an array")?
            .len();
        let (first_profile, first_label) = active_profile_and_label(profiles_before_add)?;

        // The real Hub frame renders the first account's space.
        driver.goto(env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, ".stack", "First Garden").await?;
        driver.enter_default_frame().await?;

        // Add account first opens a reversible Choice flow. It must not
        // rotate or grow the profile roster until a ceremony is submitted.
        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        element(&driver, "#account-add-profile")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let before_submit = successful_body("list profiles before add submit", &profiles);
        assert_eq!(
            before_submit["profiles"]
                .as_array()
                .context("profile roster is not an array")?
                .len(),
            profile_count_before_add,
            "opening Add account must not persist a profile"
        );
        assert_eq!(
            active_profile_and_label(before_submit)?.0,
            first_profile,
            "opening Add account must not switch profiles"
        );

        element(&driver, "#account-choose-create")
            .await?
            .click()
            .await?;
        element(&driver, "#account-email")
            .await?
            .send_keys("second@example.com")
            .await?;
        element(&driver, "#account-create-submit")
            .await?
            .click()
            .await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        activate(&driver, &env, "second@example.com").await?;

        // The second account sees none of the first account's spaces.
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("list second account's spaces", &listed)).is_empty(),
            "a fresh account must not see the other account's spaces"
        );
        let profiles = get_json(&driver, "/api/profiles").await?;
        let (_, second_label) =
            active_profile_and_label(successful_body("list second profile", &profiles))?;
        let summary = get_json(&driver, "/api/account/summary").await?;
        let passkey_created_on =
            successful_body("read second account summary", &summary)["passkey"]["createdOn"]
                .as_str()
                .context("second account summary omitted passkey creation device")?
                .to_string();

        // The second account's sealed Hub has its own empty roster.
        driver.goto(env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        if let Err(error) =
            wait_for_text_containing(&driver, "[data-account-trigger]", &second_label).await
        {
            let diagnostic = driver
                .execute(
                    r#"return {
                        trigger: document.querySelector('[data-account-trigger]')?.textContent,
                        error: document.querySelector('[data-account-error]')?.textContent,
                        errorHidden: document.querySelector('[data-account-error]')?.hidden,
                        activeProfile: document.querySelector('ui-hub-account')?.dataset.activeProfile,
                        activeProvider: document.querySelector('ui-hub-account')?.dataset.activeProvider,
                        hasTonkFetch: typeof window.tonk?.fetch === 'function'
                    }"#,
                    Vec::new(),
                )
                .await
                .map(|value| value.json().to_string())
                .unwrap_or_else(|diagnostic_error| {
                    format!("unable to inspect Hub state: {diagnostic_error}")
                });
            return Err(error).context(format!("Hub account diagnostic: {diagnostic}"));
        }
        wait_for_displayed(&driver, ".snew").await?;
        let create_action = element(&driver, ".snew").await?.text().await?;
        assert!(
            create_action.contains("create a new space"),
            "an empty Hub roster must show the creation action: {create_action:?}"
        );
        assert!(
            driver.find_all(By::Css(".srow-wrap")).await?.is_empty(),
            "an empty Hub roster must not render a space row"
        );
        let second_stack = element(&driver, ".stack").await?.text().await?;
        assert!(
            !second_stack.contains("First Garden"),
            "the second account's Hub must omit the first account's space"
        );

        // Settings reads real account and device facts through the sealed
        // guest, and keeps unsupported Usage/Syncing surfaces absent.
        click(&driver, "[data-account-trigger]").await?;
        click(&driver, "[data-open-settings]").await?;
        wait_for_text(&driver, "[data-account-email]", "second@example.com").await?;
        assert_eq!(
            element(&driver, "[data-passkey-created-on]")
                .await?
                .prop("textContent")
                .await?
                .as_deref(),
            Some(passkey_created_on.as_str()),
            "Hub settings must render the account summary's passkey creation device"
        );
        click(&driver, "[data-settings-tab=\"devices\"]").await?;
        wait_for_text_containing(&driver, "[data-device-list]", "current device").await?;
        let settings_text = element(&driver, "[data-settings-view]")
            .await?
            .text()
            .await?
            .to_ascii_lowercase();
        for forbidden in ["usage", "upgrade", "metering", "syncing"] {
            assert!(
                !settings_text.contains(forbidden),
                "Hub settings must not contain {forbidden}"
            );
        }
        let section_style = driver
            .execute(
                r#"const section = document.querySelector('.settings-section');
                const style = getComputedStyle(section);
                return { backgroundColor: style.backgroundColor, display: style.display };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            section_style.json(),
            &serde_json::json!({
                "backgroundColor": "rgba(0, 0, 0, 0)",
                "display": "block"
            }),
            "the attached settings section must not inherit the global badge treatment"
        );

        // The authoritative display-name write repaints the Hub trigger and
        // remains in the field after the dialog is reopened.
        click(&driver, "[data-settings-tab=\"account\"]").await?;
        let display_name = element(&driver, "[data-display-name]").await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Command + "a"
        } else {
            Key::Control + "a"
        };
        display_name.send_keys(select_all).await?;
        display_name.send_keys("Second Hub").await?;
        display_name.send_keys(Key::Enter).await?;
        if let Err(error) = wait_for_text(&driver, "[data-account-label]", "Second Hub").await {
            let diagnostic = driver
                .execute(
                    r#"const input = document.querySelector('[data-display-name]');
                    const error = document.querySelector('[data-display-name-error]');
                    const account = document.querySelector('ui-hub-account');
                    return {
                        trigger: document.querySelector('[data-account-trigger]')?.textContent,
                        inputValue: input?.value,
                        confirmedName: input?.dataset.confirmedName,
                        inputDisabled: input?.disabled,
                        inputBusy: input?.getAttribute('aria-busy'),
                        error: error?.textContent,
                        errorHidden: error?.hidden,
                        activeName: account?.dataset.activeName
                    }"#,
                    Vec::new(),
                )
                .await
                .map(|value| value.json().to_string())
                .unwrap_or_else(|diagnostic_error| {
                    format!("unable to inspect display-name state: {diagnostic_error}")
                });
            return Err(error).context(format!("Hub display-name diagnostic: {diagnostic}"));
        }
        click(&driver, "[data-return-spaces]").await?;
        let focus_restored = driver
            .execute(
                "return document.activeElement?.hasAttribute('data-return-spaces') === true",
                Vec::new(),
            )
            .await?;
        assert_eq!(focus_restored.json(), &serde_json::json!(true));
        click(&driver, "[data-open-settings]").await?;
        assert_eq!(
            element(&driver, "[data-display-name]")
                .await?
                .prop("value")
                .await?
                .as_deref(),
            Some("Second Hub")
        );
        click(&driver, "[data-return-spaces]").await?;

        // Switch back from the Hub's account roster. The component reloads
        // the whole top page, rebuilding subscriptions owned by the old
        // profile before mounting the first profile's Hub.
        driver.enter_default_frame().await?;
        let before_reload = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        wait_for_text_containing(&driver, "[data-account-menu]", &first_label).await?;
        let selector = format!("button[data-profile=\"{first_profile}\"]");
        click(&driver, &selector).await?;
        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if driver
                .execute("return performance.timeOrigin", Vec::new())
                .await
                .is_ok_and(|current| current.json() != &before_reload)
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for the profile switch reload"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, ".stack", "First Garden").await?;
        driver.enter_default_frame().await?;
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
        let mut url = env.tonk_web.join("settings/link")?;
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
        let mut url = env.tonk_web.join("settings/link")?;
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

        let key = create_space_awaiting_remote(&owner, "Revocable Garden", true).await?;
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

        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text_containing(&driver, "#account-device-list", "e2e terminal").await?;
        let selector = format!("#account-device-list button[data-revoke=\"{cli_did}\"]");
        click(&driver, &selector).await?;
        element(&driver, "[role=alertdialog]").await?;
        click(&driver, "#account-delete-submit").await?;
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
                                    "remote":   { "the": "dom.event.current-target.elements.remote/value", "as": "Text" }
                                }
                            }
                        },
                        "parameters": {
                            "name": "Custodied After Assertion",
                            "remote": env.tonk_web.join("ucan/")?
                        }
                    }
                }]
            }),
        )
        .await?;
        successful_body("create space command", &created);

        // Two correct outcomes race here. When the account pull has
        // already delivered the published `AccountEncryptionKey`, the
        // worker seals straight to it and no assertion is needed. When
        // it has not, the worker asks this page: a consent card
        // appears, and its button runs the assertion that records the
        // key with the root. Both paths end with the space custodied
        // under the account; only the root record differs.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut asserted = false;
        loop {
            if let Ok(button) = creator.find(By::Css("#tonk-custody-continue")).await {
                button.click().await?;
                asserted = true;
                break;
            }
            let profile = get_json(&creator, "/api/profile").await?;
            if profile["body"]["space"]
                .as_array()
                .is_some_and(|spaces| !spaces.is_empty())
            {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "neither the consent card nor the created space appeared"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        let asserted_recipient = if asserted {
            let root = poll_json(
                &creator,
                "/api/identity/root",
                "the assertion to record the key",
                |body| body.get("encryptionKey").is_some(),
            )
            .await?;
            let recipient = root["encryptionKey"]
                .as_str()
                .context("root status omitted the key")?
                .to_string();
            assert!(recipient.starts_with("did:key:z6LS"), "{recipient}");
            Some(recipient)
        } else {
            None
        };

        let profile = poll_json(
            &creator,
            "/api/profile",
            "the space to be created",
            |body| {
                body["space"]
                    .as_array()
                    .is_some_and(|spaces| !spaces.is_empty())
            },
        )
        .await?;
        let key = profile["space"][0]["key"]
            .as_str()
            .context("profile space entry omitted its key")?
            .to_string();

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
        let rows = successful_body("custodied seeds", &rows)
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            rows.iter().any(|row| {
                let subject = row["fields"]["subject"].as_str().unwrap_or_default();
                let sealed_to = row["fields"]["recipient"].as_str().unwrap_or_default();
                subject.ends_with(&key)
                    && match &asserted_recipient {
                        // The page's assertion derived the recipient;
                        // the seed must be sealed to exactly that key.
                        Some(recipient) => sealed_to == recipient,
                        // The pulled fact supplied it; any X25519
                        // recipient is the account's published key.
                        None => sealed_to.starts_with("did:key:z6LS"),
                    }
            }),
            "the new space's seed is sealed to the account's key: {rows:?}"
        );

        creator.quit().await?;
        Ok(())
    }
}
