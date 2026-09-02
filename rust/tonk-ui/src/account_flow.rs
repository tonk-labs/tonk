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

    use crate::helpers::{TestEnvironment, driver_with_prf, driver_with_prf_authenticator, goto};

    const EMAIL: &str = "person@example.com";

    async fn install_account_capture_fixture(driver: &WebDriver) -> Result<()> {
        ChromeDevTools::new(driver.handle.clone())
            .execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                serde_json::json!({
                    "source": r#"
                        (() => {
                            const read = () => {
                                try { return JSON.parse(sessionStorage.getItem("tonk:test:account-events") || "[]"); }
                                catch { return []; }
                            };
                            let config = null;
                            let superProperties = {};
                            const fixture = {
                                init(_key, next) { config = next; },
                                register(next) { superProperties = { ...superProperties, ...next }; },
                                identify() {},
                                capture(event, properties = {}) {
                                    let payload = {
                                        event,
                                        properties: {
                                            ...superProperties,
                                            ...properties,
                                            $current_url: location.href,
                                            $pathname: location.pathname,
                                            $referrer: document.referrer
                                        }
                                    };
                                    if (config && config.before_send) payload = config.before_send(payload);
                                    if (!payload) return;
                                    const events = read();
                                    events.push({ ...payload, captured_at: Date.now() });
                                    sessionStorage.setItem("tonk:test:account-events", JSON.stringify(events));
                                }
                            };
                            Object.defineProperty(window, "posthog", {
                                configurable: false,
                                get: () => fixture,
                                set: () => {}
                            });
                        })();
                    "#
                }),
            )
            .await?;
        Ok(())
    }

    async fn captured_account_events(driver: &WebDriver) -> Result<Vec<serde_json::Value>> {
        let value = driver
            .execute(
                r#"return JSON.parse(sessionStorage.getItem("tonk:test:account-events") || "[]")
                    .filter(event => event.event === "account_event");"#,
                Vec::new(),
            )
            .await?;
        Ok(serde_json::from_value(value.json().clone())?)
    }

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
        enter_guest(driver).await?;
        element(driver, ".hub-page").await?;
        Ok(())
    }

    /// Enter the sealed guest frame, whatever page it is showing.
    ///
    /// The guest renders at an opaque origin, so `contentDocument` is
    /// unreachable from the top document: reaching its DOM at all means
    /// switching the driver's browsing context to it. Every helper that
    /// touches the bar or a space page goes through here.
    async fn enter_guest(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        let frame = element(driver, "tonk-site > iframe").await?;
        frame.enter_frame().await?;
        Ok(())
    }

    /// Enter the seeded view inside the space shell's own sealed frame.
    async fn enter_space_view(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let frame = element(driver, "tonk-site > iframe").await?;
        frame.enter_frame().await?;
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

    async fn wait_for_absent(driver: &WebDriver, selector: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match driver.find_all(By::Css(selector.to_string())).await {
                Ok(found) if found.is_empty() => return Ok(()),
                Ok(_) => {}
                Err(error) if tokio::time::Instant::now() >= deadline => {
                    return Err(error).with_context(|| {
                        format!("timed out waiting for `{selector}` to disappear")
                    });
                }
                Err(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for `{selector}` to disappear"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn registration_motion_styles(driver: &WebDriver) -> Result<serde_json::Value> {
        driver
            .execute(
                r#"const action = document.querySelector('.obtn');
                   action.classList.add('wait', 'flash');
                   const read = selector => {
                     const style = getComputedStyle(document.querySelector(selector));
                     return {
                       animation: style.animationName,
                       transition: style.transitionDuration
                     };
                   };
                   return {
                     cluster: read('.tonk-cluster'),
                     row: read('.orow'),
                     action: read('.obtn'),
                     cursor: read('.cur')
                   };"#,
                Vec::new(),
            )
            .await
            .map(|value| value.json().clone())
            .map_err(Into::into)
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
    /// no long-running script to time out. The page's boot path nudges
    /// an already-active worker to claim it, and a boot that wedges is
    /// recovered by the page's own watchdog (index.html) — a reload,
    /// then a reload with caches and workers cleared — so this wait
    /// only has to outlast that ladder.
    async fn wait_for_service_worker(driver: &WebDriver) -> Result<()> {
        // From the TOP page. Inside the sealed guest
        // `navigator.serviceWorker.controller` is null — the frame is at
        // an opaque origin and is not the registration's client — so a
        // caller that had just reached into the bar would wait out the
        // whole deadline on a worker that has been in control the entire
        // time.
        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(150);
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

    /// A second browser holding the same passkey: a different device, the
    /// same person.
    ///
    /// The virtual authenticator is per-driver — it is created over CDP on
    /// one browser — so a second driver cannot be handed the first's. What
    /// CDP does allow is reading the credentials out of one authenticator
    /// and adding them to another, which is what a passkey synced through
    /// a platform keychain looks like from the page's side.
    async fn second_device_with_same_passkey(
        env: &TestEnvironment,
        first: &WebDriver,
        first_authenticator: &str,
    ) -> Result<(WebDriver, String)> {
        let source = ChromeDevTools::new(first.handle.clone());
        let credentials = source
            .execute_cdp_with_params(
                "WebAuthn.getCredentials",
                serde_json::json!({ "authenticatorId": first_authenticator }),
            )
            .await?;
        let credentials = credentials["credentials"]
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow!("Chrome omitted the virtual authenticator credentials"))?;
        if credentials.is_empty() {
            return Err(anyhow!(
                "the first device registered no passkey, so there is none to carry over"
            ));
        }

        let (second, authenticator) = driver_with_prf_authenticator(env).await?;
        let devtools = ChromeDevTools::new(second.handle.clone());
        for credential in credentials {
            devtools
                .execute_cdp_with_params(
                    "WebAuthn.addCredential",
                    serde_json::json!({
                        "authenticatorId": authenticator,
                        "credential": credential,
                    }),
                )
                .await?;
        }

        // What the copy above loses: the credential's PRF secret.
        // `WebAuthn.getCredentials` exports the signing key but not the
        // hmac-secret, so the copied passkey signs fine and yields no
        // PRF outputs — and custody derives its keys from those, so a
        // login on the second device dies at "this platform cannot
        // unlock custody". A real synced passkey carries the secret
        // with it. Model that: evaluate the custody salts once on the
        // device that holds the secret, and graft the outputs into the
        // second device's assertions.
        let (key_output, kek_output) = custody_prf_outputs(first).await?;
        graft_prf_outputs(&second, &key_output, &kek_output).await?;
        Ok((second, authenticator))
    }

    /// The PRF outputs this driver's authenticator derives for the two
    /// custody salts — the values a platform keychain syncs with the
    /// passkey and CDP cannot export. One silent assertion; the page
    /// must be on the passkey's relying-party origin.
    async fn custody_prf_outputs(driver: &WebDriver) -> Result<(String, String)> {
        let outcome = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const [keyContext, kekContext] = [arguments[0], arguments[1]];
                navigator.credentials.get({ publicKey: {
                    challenge: crypto.getRandomValues(new Uint8Array(32)),
                    userVerification: "required",
                    extensions: { prf: { eval: {
                        first: new TextEncoder().encode(keyContext),
                        second: new TextEncoder().encode(kekContext),
                    }}},
                }}).then(credential => {
                    const prf = (credential.getClientExtensionResults() || {}).prf;
                    if (!prf || !prf.results || !prf.results.first || !prf.results.second) {
                        return done({ error: "the source authenticator returned no PRF outputs" });
                    }
                    const b64 = buffer => btoa(String.fromCharCode(...new Uint8Array(buffer)));
                    done({ first: b64(prf.results.first), second: b64(prf.results.second) });
                }).catch(error => done({ error: String(error) }));
                "#,
                vec![
                    serde_json::json!(std::str::from_utf8(
                        tonk_identity::envelope::CUSTODY_KEY_CONTEXT
                    )?),
                    serde_json::json!(std::str::from_utf8(
                        tonk_identity::envelope::CUSTODY_KEK_CONTEXT
                    )?),
                ],
            )
            .await?;
        let outcome = outcome.json().clone();
        if let Some(error) = outcome.get("error").and_then(|error| error.as_str()) {
            return Err(anyhow!("could not read the custody PRF outputs: {error}"));
        }
        let field = |name: &str| -> Result<String> {
            outcome[name]
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("PRF read returned no {name} output"))
        };
        Ok((field("first")?, field("second")?))
    }

    /// Make every future document on `driver` answer custody assertions
    /// with `key_output`/`kek_output` (base64) as its PRF results.
    ///
    /// The assertion itself still runs against the local authenticator —
    /// the signature is real — only the extension outputs are replaced,
    /// which is the one thing the credential copy cannot carry.
    async fn graft_prf_outputs(
        driver: &WebDriver,
        key_output: &str,
        kek_output: &str,
    ) -> Result<()> {
        let script = format!(
            r#"
            (() => {{
                const outputs = {{ first: "{key_output}", second: "{kek_output}" }};
                const unb64 = text =>
                    Uint8Array.from(atob(text), letter => letter.charCodeAt(0)).buffer;
                const real = navigator.credentials.get.bind(navigator.credentials);
                navigator.credentials.get = async options => {{
                    const credential = await real(options);
                    const asked = options && options.publicKey
                        && options.publicKey.extensions && options.publicKey.extensions.prf;
                    if (asked) {{
                        const results = credential.getClientExtensionResults.bind(credential);
                        Object.defineProperty(credential, "getClientExtensionResults", {{
                            value: () => {{
                                const r = results();
                                r.prf = Object.assign({{}}, r.prf, {{ results: {{
                                    first: unb64(outputs.first),
                                    second: unb64(outputs.second),
                                }}}});
                                return r;
                            }},
                        }});
                    }}
                    return credential;
                }};
            }})();
            "#
        );
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools
            .execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                serde_json::json!({ "source": script }),
            )
            .await?;
        Ok(())
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

    async fn emulate_phone(driver: &WebDriver, width: u32, height: u32) -> Result<()> {
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools
            .execute_cdp_with_params(
                "Emulation.setDeviceMetricsOverride",
                serde_json::json!({
                    "width": width,
                    "height": height,
                    "deviceScaleFactor": 2,
                    "mobile": true,
                    "screenWidth": width,
                    "screenHeight": height
                }),
            )
            .await?;
        devtools
            .execute_cdp_with_params(
                "Emulation.setTouchEmulationEnabled",
                serde_json::json!({ "enabled": true, "maxTouchPoints": 5 }),
            )
            .await?;
        Ok(())
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
        goto(driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(driver, "tonk-account[data-mode=\"choice\"]").await?;
        run_cluster_ceremony(driver, email).await?;
        // And that is where it stops. Creating an account leaves the
        // ceremony standing on "awaiting confirmation" — the emailed
        // link is the next step, and the cluster says so — so there is
        // no dashboard to land on yet. `activate` is what finishes it.
        Ok(())
    }

    /// Print the browser console (page and service worker alike) via
    /// chromedriver's classic log endpoint. Diagnostic only; requires
    /// the driver to have been created with TONK_E2E_CHROME_LOG set.
    async fn dump_browser_log(driver: &WebDriver, env: &TestEnvironment) {
        let path = format!("session/{}/se/log", driver.session_id());
        let Ok(url) = env.chromedriver.join(&path) else {
            return;
        };
        match reqwest::Client::new()
            .post(url)
            .json(&serde_json::json!({ "type": "browser" }))
            .send()
            .await
        {
            Ok(response) => eprintln!(
                "BROWSER LOG DUMP: {}",
                response.text().await.unwrap_or_default()
            ),
            Err(error) => eprintln!("BROWSER LOG DUMP failed: {error}"),
        }
    }

    /// Take the cluster down the way its own control does.
    async fn dismiss_register_dialog(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        driver
            .execute(
                r##"
                const back = document.querySelector("#tonk-register-dismiss");
                if (back) back.click();
                "##,
                Vec::new(),
            )
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let gone = driver
                .execute(
                    r##"return !document.querySelector("#tonk-register");"##,
                    Vec::new(),
                )
                .await?;
            if gone.json().as_bool() == Some(true) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("the cluster stayed up after dismiss"));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Run the account ceremony for `email` from the raised cluster.
    ///
    /// The panel that asked "create account or log in" before knowing
    /// the address is gone: one entry raises this, and the address
    /// lookup picks which ceremony runs. Every caller that used to click
    /// through those panels goes through here.
    pub(crate) async fn run_cluster_ceremony(driver: &WebDriver, email: &str) -> Result<()> {
        element(driver, "#account-choose-link")
            .await?
            .click()
            .await?;
        await_register_dialog(driver).await?;
        type_into_register_dialog(driver, email).await?;
        await_register_action(driver, "create a passkey").await?;
        click_register_action(driver).await?;
        let passkey = await_settled_row(driver, "passkey").await?;
        anyhow::ensure!(
            passkey.contains(" on "),
            "the passkey row names the device, got {passkey:?}",
        );
        // The ceremony hands enrollment off rather than awaiting a
        // receipt — it is a command now — so the row asking for the
        // emailed link is what says it landed. A caller that goes
        // straight to the inbox would otherwise read it before the
        // service had been asked to send anything.
        await_narrator_containing(driver, "confirmation link").await?;
        Ok(())
    }

    /// Sign in to an existing account for `email`, from the cluster.
    ///
    /// The counterpart to [`run_cluster_ceremony`]: the address decides
    /// which of the two runs, so signing in is the same control and the
    /// same field, answered differently. The old login panel it
    /// replaced is no longer reachable — one entry raises this instead.
    pub(crate) async fn run_cluster_login(driver: &WebDriver, email: &str) -> Result<()> {
        element(driver, "#account-choose-link")
            .await?
            .click()
            .await?;
        await_register_dialog(driver).await?;
        type_into_register_dialog(driver, email).await?;
        await_register_action(driver, "log in with your passkey").await?;
        click_register_action(driver).await?;
        // Wait for the ceremony's own receipt before taking the cluster
        // down, the way signing up does. Dismissing on the click alone
        // races the assertion the platform is still holding, so a caller
        // that goes straight on to read the panel is reading it before
        // there is anything to read.
        await_settled_row(driver, "passkey").await?;
        dismiss_register_dialog(driver).await?;
        Ok(())
    }

    /// Confirm the emailed address from a SECOND TAB, leaving the
    /// tab that raised the ceremony exactly where it is.
    ///
    /// Which is what the emailed link does, and what the cluster
    /// requires: it is a DOM element with no persistence, so navigating
    /// the ceremony's own tab to the link and back destroys it, and the
    /// confirmation comes home to nothing. Activation reaches the
    /// waiting tab as a fact on profile main, which is why it can cross
    /// tabs at all.
    pub(crate) async fn activate_in_another_tab(
        driver: &WebDriver,
        env: &TestEnvironment,
        email: &str,
    ) -> Result<()> {
        let link = activation_link(env, email).await?;
        let ceremony = driver.window().await?;
        let confirm = driver.new_tab().await?;
        driver.switch_to_window(confirm).await?;
        goto(driver, &link).await?;
        element(driver, "#activate-accept").await?.click().await?;
        element(driver, "#activate-done").await?;
        driver.close_window().await?;
        driver.switch_to_window(ceremony).await?;
        Ok(())
    }

    /// Present `email`'s activation invocation to the access service
    /// over plain HTTP — what another device's activation page does, as
    /// far as this browser can tell: nothing in it handles the link.
    async fn activate_over_http(env: &TestEnvironment, email: &str) -> Result<()> {
        use base64::Engine as _;
        let link = activation_link(env, email).await?;
        let encoded = link
            .split("ucan=")
            .nth(1)
            .ok_or_else(|| anyhow!("the activation link names no invocation"))?;
        let invocation = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)?;
        let response = reqwest::Client::new()
            .post(env.access_service.join("ucan/")?)
            .header("content-type", "application/cbor")
            .body(invocation)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success(),
            "activation was refused: {}",
            response.status()
        );
        Ok(())
    }

    /// Copy every credential from one virtual authenticator to another:
    /// what a passkey manager's sync does, over CDP.
    async fn copy_credentials(
        from: &WebDriver,
        from_id: &str,
        to: &WebDriver,
        to_id: &str,
    ) -> Result<()> {
        let from_tools = ChromeDevTools::new(from.handle.clone());
        let to_tools = ChromeDevTools::new(to.handle.clone());
        let held = from_tools
            .execute_cdp_with_params(
                "WebAuthn.getCredentials",
                serde_json::json!({ "authenticatorId": from_id }),
            )
            .await?;
        let credentials = held["credentials"].as_array().cloned().unwrap_or_default();
        anyhow::ensure!(
            !credentials.is_empty(),
            "the first device holds a credential to copy"
        );
        for credential in credentials {
            to_tools
                .execute_cdp_with_params(
                    "WebAuthn.addCredential",
                    serde_json::json!({ "authenticatorId": to_id, "credential": credential }),
                )
                .await?;
        }
        Ok(())
    }

    /// Activation opened somewhere this browser cannot see still reaches
    /// the waiting ceremony, and QUICKLY: the gate stops refusing the
    /// account sweep, the sweep the ceremony itself is driving gets
    /// served, and THIS browser records the fact its subscription flips
    /// on. The cross-tab variant cannot pin this — an activating tab
    /// shares the worker and does the recording itself — and this exact
    /// seam is where the live flow broke: the reactor's cached branch
    /// session predated the upstream wiring, so every post-activation
    /// sweep failed `Branch main has no upstream`, forever.
    #[dialog_common::test]
    async fn it_notices_activation_performed_on_another_device(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        enroll_only(&driver, &env, "confirmed-elsewhere@example.com").await?;

        activate_over_http(&env, "confirmed-elsewhere@example.com").await?;

        // The waiting row resolves from the sweep alone — inside the
        // helper's one-minute patience, where the ceremony's own nudge
        // cadence is seconds.
        await_row_value(&driver, "email", "verified").await?;
        driver.quit().await?;
        Ok(())
    }

    /// The whole three-device story. Device A starts registration and
    /// waits. Device B signs in with the same passkey while the email
    /// is unopened — parked on the same awaiting row, not an error.
    /// Device C (here: plain HTTP) opens the link. Both A and B then
    /// finish ON THEIR OWN: A's sweep is served and records the fact,
    /// and B's worker kept the assertion's derivation handles and
    /// completes the parked login — no second passkey tap.
    #[dialog_common::test]
    async fn it_finishes_both_waiting_devices_when_a_third_confirms(
        env: TestEnvironment,
    ) -> Result<()> {
        let email = "three-devices@example.com";
        let (device_a, authenticator_a) = driver_with_prf_authenticator(&env).await?;
        enroll_only(&device_a, &env, email).await?;

        // Device B: a separate browser holding the same passkey. The
        // credential copy carries the signing key but not the PRF
        // secret custody derives its keys from, so the second half of
        // what a platform keychain syncs is grafted alongside it — see
        // `second_device_with_same_passkey`.
        let (device_b, authenticator_b) = driver_with_prf_authenticator(&env).await?;
        copy_credentials(&device_a, &authenticator_a, &device_b, &authenticator_b).await?;
        let (key_output, kek_output) = custody_prf_outputs(&device_a).await?;
        graft_prf_outputs(&device_b, &key_output, &kek_output).await?;
        goto(&device_b, env.tonk_web.join("settings")?.as_str()).await?;
        element(&device_b, "tonk-account[data-mode=\"choice\"]").await?;
        element(&device_b, "#account-choose-link")
            .await?
            .click()
            .await?;
        await_register_dialog(&device_b).await?;
        type_into_register_dialog(&device_b, email).await?;
        await_register_action(&device_b, "log in with your passkey").await?;
        click_register_action(&device_b).await?;
        // Refused by the gate, and parked rather than failed.
        await_row_value(&device_b, "email", "awaiting confirmation").await?;

        // Device C.
        activate_over_http(&env, email).await?;

        // Device A's ceremony resolves from its own sweep.
        await_row_value(&device_a, "email", "verified").await?;
        // Device B's parked login finishes silently: verified, with the
        // passkey row the completed sign-in shows — and nothing asked
        // for a second assertion.
        await_row_value(&device_b, "email", "verified").await?;
        await_settled_row(&device_b, "passkey").await?;

        device_a.quit().await?;
        device_b.quit().await?;
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
        goto(driver, &link).await?;
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
        goto(driver, env.tonk_web.join("settings")?.as_str()).await?;
        wait_for_backup_done(driver).await?;
        // Back to where the caller was: activation is a detour, not a
        // navigation the caller asked for.
        goto(driver, account.as_str()).await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_redirects_legacy_account_routes_without_losing_the_query(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let mut legacy = env.tonk_web.join("account")?;
        legacy.set_query(Some("next=%2Fspace%2Fdid%3Akey%3AzOne&add=1"));
        goto(&driver, legacy.as_str()).await?;
        element(&driver, "tonk-account").await?;
        let current = driver.current_url().await?;
        assert_eq!(current.path(), "/settings");
        assert_eq!(current.query(), legacy.query());

        let mut legacy_link = env.tonk_web.join("account/link")?;
        legacy_link.set_query(Some(
            "audience=did%3Akey%3AzCli&callback=http%3A%2F%2F127.0.0.1%3A9999&name=terminal",
        ));
        goto(&driver, legacy_link.as_str()).await?;
        element(&driver, "tonk-account").await?;
        let current = driver.current_url().await?;
        assert_eq!(current.path(), "/settings/link");
        assert_eq!(current.query(), legacy_link.query());

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_captures_the_ordered_signup_account_journey(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        install_account_capture_fixture(&driver).await?;
        sign_up(&driver, &env, "observability-signup@example.com").await?;

        let events = captured_account_events(&driver).await?;
        let wire = serde_json::to_string(&events)?;
        for sentinel in [
            "observability-signup@example.com",
            "did:key:",
            "credentialId",
            "activation?ucan=",
            "/api/account",
            "127.0.0.1",
        ] {
            anyhow::ensure!(
                !wire.contains(sentinel),
                "captured account payload exposed privacy sentinel {sentinel:?}: {wire}"
            );
        }

        let expected = [
            (
                "open_registration",
                "finished",
                "input",
                Some("success"),
                None,
            ),
            ("create_account", "started", "input", None, None),
            ("create_account", "checkpoint", "email_lookup", None, None),
            ("create_account", "checkpoint", "passkey_create", None, None),
            (
                "create_account",
                "finished",
                "activation_wait",
                Some("blocked"),
                Some("awaiting_activation"),
            ),
            ("activate_account", "started", "input", None, None),
            (
                "activate_account",
                "finished",
                "complete",
                Some("success"),
                None,
            ),
            ("settle_account", "started", "account_sync", None, None),
            (
                "settle_account",
                "finished",
                "complete",
                Some("success"),
                None,
            ),
        ];
        let mut cursor = 0;
        for (action, phase, stage, result, failure) in expected {
            let Some(offset) = events[cursor..].iter().position(|event| {
                let properties = &event["properties"];
                properties["action"] == action
                    && properties["phase"] == phase
                    && properties["stage"] == stage
                    && result.is_none_or(|value| properties["result"] == value)
                    && failure.is_none_or(|value| properties["failure_kind"] == value)
            }) else {
                anyhow::bail!(
                    "signup account_event sequence missed {action}/{phase}/{stage}: {events:?}"
                );
            };
            cursor += offset + 1;
        }

        let mut terminals = std::collections::HashMap::<String, usize>::new();
        for event in &events {
            if event["properties"]["phase"] == "finished"
                && let Some(attempt_id) = event["properties"]["attempt_id"].as_str()
            {
                *terminals.entry(attempt_id.to_owned()).or_default() += 1;
            }
        }
        anyhow::ensure!(
            terminals.values().all(|count| *count == 1),
            "an account attempt emitted more than one terminal event: {events:?}"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_up_through_the_account_panels(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        install_account_capture_fixture(&driver).await?;
        sign_up(&driver, &env, EMAIL).await?;

        wait_for_text_containing(&driver, "#account-email-value", EMAIL).await?;
        // Creation mints the first custody passkey in the same ceremony
        // that generates and seals the secret, so the dashboard
        // describes it immediately.
        if let Err(error) =
            wait_for_text_containing(&driver, "#account-passkey-device-value", "Chrome on ").await
        {
            let summary = get_json(&driver, "/api/account/summary").await;
            eprintln!("PROBE /api/account/summary: {summary:?}");
            dump_browser_log(&driver, &env).await;
            return Err(error);
        }
        // The device list lives on the Devices tab, whose pane is
        // hidden until selected — and hidden text reads as empty.
        click(&driver, "#account-tab-devices").await?;
        wait_for_text_containing(&driver, "#account-device-list", "Chrome on ").await?;
        // Back to the Account tab: everything below reads from it.
        click(&driver, "#account-tab-account").await?;
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

        let select_all = || {
            if cfg!(target_os = "macos") {
                Key::Meta + "a"
            } else {
                Key::Control + "a"
            }
        };
        // The name write lands in the account state, whose first sync
        // races this test right after activation — until the pull lands
        // the worker refuses it as account_state_unavailable, and the
        // input springs back to its previous value. Each retry is the
        // same user gesture; the deadline is on the outcome.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let display_name = element(&driver, "#account-display-name").await?;
            // A save in flight disables the input, and typing into a
            // disabled input errors — that too reads as "not yet".
            let typed = async {
                display_name.send_keys(select_all()).await?;
                display_name.send_keys("Settings Name").await?;
                display_name.send_keys(Key::Enter).await?;
                Ok::<(), thirtyfour::error::WebDriverError>(())
            }
            .await;
            if typed.is_err() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let settled = tokio::time::Instant::now() + Duration::from_secs(5);
            let saved = loop {
                if let Ok(found) = driver.find(By::Css("#account-display-name")).await
                    && found.prop("value").await?.as_deref() == Some("Settings Name")
                    && found.attr("aria-busy").await?.is_none()
                {
                    break true;
                }
                if tokio::time::Instant::now() >= settled {
                    break false;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            };
            if saved {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the display name never saved; the account state likely never hydrated"
            );
        }
        let settings = driver.current_url().await?;
        goto(&driver, settings.as_str()).await?;
        wait_for_value(&driver, "#account-display-name", "Settings Name").await?;

        let events = captured_account_events(&driver).await?;
        anyhow::ensure!(
            !events.is_empty(),
            "signup emitted no account_event payloads"
        );
        let wire = serde_json::to_string(&events)?;
        for sentinel in [
            EMAIL,
            "did:key:",
            "credentialId",
            "activation?ucan=",
            "/api/account",
            "127.0.0.1",
        ] {
            anyhow::ensure!(
                !wire.contains(sentinel),
                "captured account payload exposed privacy sentinel {sentinel:?}: {wire}"
            );
        }
        let mut terminals = std::collections::HashMap::<String, usize>::new();
        for event in &events {
            let properties = &event["properties"];
            if properties["phase"] == "finished"
                && let Some(attempt_id) = properties["attempt_id"].as_str()
            {
                *terminals.entry(attempt_id.to_owned()).or_default() += 1;
            }
        }
        anyhow::ensure!(
            terminals.values().all(|count| *count == 1),
            "an account attempt emitted more than one terminal event: {events:?}"
        );

        let event_position = |action: &str,
                              phase: &str,
                              stage: Option<&str>,
                              result: Option<&str>,
                              failure: Option<&str>| {
            events.iter().position(|event| {
                let properties = &event["properties"];
                properties["action"] == action
                    && properties["phase"] == phase
                    && stage.is_none_or(|value| properties["stage"] == value)
                    && result.is_none_or(|value| properties["result"] == value)
                    && failure.is_none_or(|value| properties["failure_kind"] == value)
            })
        };
        let registration = event_position(
            "open_registration",
            "finished",
            Some("input"),
            Some("success"),
            None,
        );
        let create_start = event_position("create_account", "started", None, None, None);
        let email_lookup = event_position(
            "create_account",
            "checkpoint",
            Some("email_lookup"),
            None,
            None,
        );
        let passkey_create = event_position(
            "create_account",
            "checkpoint",
            Some("passkey_create"),
            None,
            None,
        );
        let create_wait = event_position(
            "create_account",
            "finished",
            Some("activation_wait"),
            Some("blocked"),
            Some("awaiting_activation"),
        );
        let activation_start = event_position("activate_account", "started", None, None, None);
        let activation_success = event_position(
            "activate_account",
            "finished",
            Some("complete"),
            Some("success"),
            None,
        );
        let settle_start = event_position("settle_account", "started", None, None, None);
        let settle_success = event_position(
            "settle_account",
            "finished",
            Some("complete"),
            Some("success"),
            None,
        );
        let sequence = [
            registration,
            create_start,
            email_lookup,
            passkey_create,
            create_wait,
            activation_start,
            activation_success,
            settle_start,
            settle_success,
        ];
        anyhow::ensure!(
            sequence.iter().all(Option::is_some),
            "signup account_event sequence was incomplete: {events:?}"
        );
        let positions = sequence.into_iter().flatten().collect::<Vec<_>>();
        anyhow::ensure!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "signup account_event sequence was out of order: {events:?}"
        );
        anyhow::ensure!(
            events.windows(2).all(|pair| {
                pair[0]["captured_at"].as_u64().unwrap_or_default()
                    <= pair[1]["captured_at"].as_u64().unwrap_or_default()
            }),
            "signup account_event timestamps were out of order: {events:?}"
        );

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
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
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

        goto(&driver, env.tonk_web.as_str()).await?;
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
        driver.enter_default_frame().await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        for (window_width, expected_total, expected_rail, expected_body) in [(1200, 720, 144, 576)]
        {
            driver.set_window_rect(0, 0, window_width, 900).await?;
            let geometry = driver
                .execute(
                    r#"const settings = document.querySelector('.account__settings').getBoundingClientRect();
                        const rail = document.querySelector('.account__rail').getBoundingClientRect();
                        const body = document.querySelector('.account__settings-body').getBoundingClientRect();
                        document.querySelector('#account-tab-account').click();
                        const selectedTabElement = document.querySelector('#account-tab-account');
                        const selectedTab = selectedTabElement.getBoundingClientRect();
                        // Computed-style declarations are live; snapshot the
                        // selected state before switching to Devices below.
                        const selectedTabBorderRight = getComputedStyle(selectedTabElement).borderRightWidth;
                        const selectedTabBridgeWidth = getComputedStyle(selectedTabElement, '::after').width;
                        const accountHeight = Math.round(document.querySelector('.account__settings-body').getBoundingClientRect().height);
                        document.querySelector('#account-tab-devices').click();
                        const devicesHeight = Math.round(document.querySelector('.account__settings-body').getBoundingClientRect().height);
                        const error = document.querySelector('#account-error');
                        error.hidden = false;
                        error.focus();
                        const errorRight = Math.round(error.getBoundingClientRect().right);
                        const errorWidth = Math.round(error.getBoundingClientRect().width);
                        const errorFocusShadow = getComputedStyle(error).boxShadow;
                        // Read the body's edge in the SAME layout state:
                        // revealing the notice can grow the page past the
                        // viewport, and a classic scrollbar appearing then
                        // shifts the centered column half a gutter — a
                        // comparison across that boundary measures the
                        // scrollbar, not the alignment.
                        const errorBodyRight = Math.round(document.querySelector('.account__settings-body').getBoundingClientRect().right);
                        error.hidden = true;
                        const logo = document.querySelector('.account__logo').getBoundingClientRect();
                        return {
                          settings: Math.round(settings.width),
                          rail: Math.round(rail.width),
                          body: Math.round(body.width),
                          railTop: Math.round(rail.top),
                          bodyTop: Math.round(body.top),
                          selectedTabRight: Math.round(selectedTab.right),
                          bodyLeft: Math.round(body.left),
                          selectedTabBorderRight,
                          selectedTabBridgeWidth,
                          accountHeight,
                          devicesHeight,
                          bodyRight: errorBodyRight,
                          errorRight,
                          errorWidth,
                          errorFocusShadow,
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
            assert_eq!(geometry["railTop"], geometry["bodyTop"]);
            assert_eq!(geometry["selectedTabRight"], geometry["bodyLeft"]);
            assert_eq!(geometry["selectedTabBorderRight"], "0px");
            assert_eq!(geometry["selectedTabBridgeWidth"], "2px");
            assert_eq!(
                geometry["accountHeight"], geometry["devicesHeight"],
                "Account and Devices tabs must keep one panel height at {window_width}px"
            );
            assert_eq!(
                geometry["errorRight"], geometry["bodyRight"],
                "settings notices must align with the panel body at {window_width}px"
            );
            assert_eq!(
                geometry["errorWidth"], geometry["body"],
                "settings notices must span the panel body at {window_width}px"
            );
            assert!(
                !geometry["errorFocusShadow"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("inset"),
                "focused settings notices must keep their ordinary frame"
            );
            assert_eq!(geometry["logoVisible"], true);
        }

        emulate_phone(&driver, 390, 844).await?;
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
                      undersized: visible.flatMap(el => {
                        const rect = el.getBoundingClientRect();
                        if (rect.width >= 44 && rect.height >= 44) return [];
                        return [{
                          selector: el.id ? `#${el.id}` : el.tagName.toLowerCase(),
                          width: rect.width,
                          height: rect.height
                        }];
                      })
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

    /// Signing in on a second device before the emailed link is opened
    /// waits, rather than failing.
    ///
    /// The regression this pins: `existing` meant "an account exists for
    /// this address", and the ceremony read it as "the account is
    /// activated" — so a second device closed the ceremony, could not
    /// hydrate the account branch, and showed "We couldn't finish logging
    /// you in" with nothing to act on. What it is actually waiting for is
    /// an email someone has not opened yet, on a device that may not be
    /// this one.
    ///
    /// Two things had to be true for the wait to work at all, and both are
    /// exercised here:
    ///
    /// - the passkey's custody space must be PROVISIONED even though the
    ///   customer is unconfirmed, or the gate refuses with "not
    ///   provisioned" and `Recourse::None` — a dead end
    /// - the gate's refusal must be readable as "waiting on the email",
    ///   which is what turns it into a row instead of an error
    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn it_waits_for_the_email_when_a_second_device_signs_in(
        env: TestEnvironment,
    ) -> Result<()> {
        const EMAIL: &str = "second-device@example.com";

        // First device: enrol, and stop. The link is never opened, so the
        // customer stays unconfirmed for the whole test.
        let (first, authenticator) = driver_with_prf_authenticator(&env).await?;
        enroll_only(&first, &env, EMAIL).await?;

        // A second device holding the same passkey. A fresh profile is
        // what makes it a different device; the shared virtual
        // authenticator is what makes it the same person.
        let (second, _second_authenticator) =
            second_device_with_same_passkey(&env, &first, &authenticator).await?;
        wait_for_service_worker(&second).await?;
        goto(&second, env.tonk_web.join("settings")?.as_str()).await?;
        element(&second, "tonk-account[data-mode=\"choice\"]").await?;
        click(&second, "#account-choose-link").await?;
        await_register_dialog(&second).await?;
        type_into_register_dialog(&second, EMAIL).await?;
        // The address is taken, so the offer is to sign in rather than
        // create — that much already worked.
        await_register_action(&second, "log in with your passkey").await?;
        click_register_action(&second).await?;

        // What this test exists for: a row naming the outstanding step,
        // not a failure. The ceremony stays up, because the thing it
        // waits on has not happened yet.
        let row = match element(&second, "#tonk-register-confirm-row").await {
            Ok(row) => row,
            Err(error) => {
                if let Ok(status) = second.find(By::Css("#tonk-register-status")).await {
                    let text = status.text().await.unwrap_or_default();
                    eprintln!("PROBE register status: {text:?}");
                }
                dump_browser_log(&second, &env).await;
                return Err(error);
            }
        };
        let text = row.text().await?;
        assert!(
            text.contains("awaiting confirmation"),
            "a second device should wait on the email, got {text:?}"
        );

        let status = element(&second, "#tonk-register-status")
            .await?
            .text()
            .await?;
        assert!(
            !status.contains("couldn't finish"),
            "and must not report a failure for a wait: {status:?}"
        );
        assert!(
            status.contains("confirmation link"),
            "it should name the step that finishes this: {status:?}"
        );

        first.quit().await?;
        second.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_explains_email_verification_before_account_sync(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        enroll_only(&driver, &env, "verify-first@example.com").await?;

        // The ceremony is still standing, and it is what the person is
        // looking at — so it is where the next step has to be named. The
        // panel behind it used to carry this notice, back when creation
        // happened in the panel itself.
        let notice = element(&driver, "#tonk-register-status")
            .await?
            .text()
            .await?;
        assert!(
            notice.contains("confirmation link"),
            "pending setup should direct the person to the emailed link: {notice:?}"
        );
        assert!(
            !notice.contains("hydration") && !notice.contains("could not be synchronized"),
            "pending setup should not expose account-state implementation terms: {notice:?}"
        );
        assert!(
            !notice.contains("reload /settings"),
            "opening the emailed link should be the only requested next step: {notice:?}"
        );

        // Navigate the way a person following a settings link would while
        // the emailed confirmation is still unopened. The customer command
        // can still be settling here; the dashboard must keep probing until
        // it can replace its temporary unhydrated fallback with the actual
        // prerequisite.
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;

        wait_for_text_containing(&driver, "#account-error", "verification link").await?;
        let dashboard_error = element(&driver, "#account-error").await?.text().await?;
        assert!(
            dashboard_error.contains("verify your email"),
            "settings should name the pending email step: {dashboard_error:?}"
        );
        for technical in [
            "not synchronized",
            "hydration",
            "reload /settings",
            "account state",
            "HTTP",
        ] {
            assert!(
                !dashboard_error
                    .to_lowercase()
                    .contains(&technical.to_lowercase()),
                "settings should not expose {technical:?}: {dashboard_error:?}"
            );
        }

        let display_name = element(&driver, "#account-display-name").await?;
        assert!(
            !display_name.is_enabled().await?,
            "authoritative account fields must stay disabled until verification lets shared account state load"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_scopes_registration_focus_and_restores_the_opener(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;

        let opener = element(&driver, "#account-choose-link").await?;
        opener.click().await?;
        await_register_dialog(&driver).await?;

        let focus_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let state = loop {
            let state = driver
                .execute(
                    r#"const dialog = document.querySelector('#tonk-register');
                       return {
                         tag: dialog?.tagName,
                         open: dialog?.open,
                         status: (document.querySelector('#tonk-register-status')?.textContent || '').trim(),
                         focusedInside: dialog?.contains(document.activeElement)
                       };"#,
                    Vec::new(),
                )
                .await?;
            if state.json()["focusedInside"] == true {
                break state;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < focus_deadline,
                "registration did not move initial focus inside the dialog: {}",
                state.json()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let state = state.json();
        assert_eq!(state["tag"], "DIALOG");
        assert_eq!(state["open"], true);
        assert_eq!(state["focusedInside"], true);
        assert!(
            state["status"]
                .as_str()
                .is_some_and(|status| !status.is_empty()),
            "registration must narrate its initial step: {state}"
        );

        assert!(
            opener.click().await.is_err(),
            "the modal top layer must make background actions non-interactable"
        );
        for _ in 0..8 {
            driver.action_chain().send_keys(Key::Tab).perform().await?;
            let focus = driver
                .execute(
                    r#"const dialog = document.querySelector('#tonk-register');
                       return {
                         inside: dialog.contains(document.activeElement),
                         active: document.activeElement?.id || document.activeElement?.tagName
                       };"#,
                    Vec::new(),
                )
                .await?;
            assert_eq!(
                focus.json()["inside"],
                true,
                "Tab escaped the registration dialog: {}",
                focus.json()
            );
        }

        driver
            .action_chain()
            .send_keys(Key::Escape)
            .perform()
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let closed = driver
                .execute(
                    r#"return {
                         gone: !document.querySelector('#tonk-register'),
                         active: document.activeElement?.id || ''
                       };"#,
                    Vec::new(),
                )
                .await?;
            if closed.json()["gone"] == true {
                let focus_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                loop {
                    let active = driver
                        .execute(r#"return document.activeElement?.id || '';"#, Vec::new())
                        .await?;
                    if active.json() == "account-choose-link" {
                        break;
                    }
                    anyhow::ensure!(
                        tokio::time::Instant::now() < focus_deadline,
                        "closing registration did not restore the Settings opener: {}",
                        active.json()
                    );
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "registration remained after Escape"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_restores_registration_focus_to_the_guest_opener(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;

        enter_hub(&driver).await?;
        let opener = wait_for_displayed(&driver, "[data-account-trigger]").await?;
        opener.click().await?;
        await_register_dialog(&driver).await?;

        driver.enter_default_frame().await?;
        driver
            .action_chain()
            .send_keys(Key::Escape)
            .perform()
            .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if driver.find(By::Css("#tonk-register")).await.is_err() {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "registration remained after Escape"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let focus_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let outer = driver
                .execute(
                    r#"return document.activeElement?.matches('tonk-site > iframe') || false;"#,
                    Vec::new(),
                )
                .await?;
            if outer.json() == true {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < focus_deadline,
                "focus did not return through the sealed Hub frame"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        enter_hub(&driver).await?;
        loop {
            let guest = driver
                .execute(
                    r#"return document.activeElement?.matches('[data-account-trigger]') || false;"#,
                    Vec::new(),
                )
                .await?;
            if guest.json() == true {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < focus_deadline,
                "focus did not return to the exact Hub account trigger"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_removes_registration_motion_when_reduced_motion_is_requested(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        click(&driver, "#account-choose-link").await?;
        await_register_dialog(&driver).await?;

        let normal = registration_motion_styles(&driver).await?;
        assert!(
            normal["row"]["transition"]
                .as_str()
                .is_some_and(|duration| duration != "0s"),
            "normal mode retains the authored row transition: {normal}"
        );

        ChromeDevTools::new(driver.handle.clone())
            .execute_cdp_with_params(
                "Emulation.setEmulatedMedia",
                serde_json::json!({
                    "features": [{ "name": "prefers-reduced-motion", "value": "reduce" }]
                }),
            )
            .await?;
        let reduced = registration_motion_styles(&driver).await?;
        for selector in ["cluster", "row", "action"] {
            assert_eq!(
                reduced[selector]["transition"], "0s",
                "{selector} transition must stop in reduced motion: {reduced}"
            );
        }
        for selector in ["action", "cursor"] {
            assert_eq!(
                reduced[selector]["animation"], "none",
                "{selector} animation must stop in reduced motion: {reduced}"
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
        goto(&driver, activation.as_str()).await?;
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
        assert_eq!(desktop["page"], "rgb(232, 230, 228)");
        assert_eq!(desktop["mainWidth"], 576);
        assert_eq!(desktop["mainCenter"], desktop["viewportCenter"]);
        assert_eq!(desktop["ceremonyWidth"], 432);
        assert_eq!(desktop["logoWidth"], 132);
        assert_eq!(desktop["actionHeight"], 36);
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
        assert_eq!(done.json()["actionHeight"], 36);

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
                      undersized: visible.flatMap(el => {
                        const rect = el.getBoundingClientRect();
                        if (rect.width >= 44 && rect.height >= 44) return [];
                        return [{
                          selector: el.id ? `#${el.id}` : el.tagName.toLowerCase(),
                          width: rect.width,
                          height: rect.height
                        }];
                      })
                    };"#,
                Vec::new(),
            )
            .await?;
        let compact = compact.json();
        let viewport = compact["viewport"].as_i64().unwrap_or_default();
        // Some browsers refuse to shrink a window below a floor of their
        // own (Chrome 152 headless clamps to 500px), and the compact
        // layout is only what it claims to be at the width we asked for.
        // Above that floor `.account__ceremony`'s own 432px cap is what
        // limits it, not the viewport, so the assertion below would be
        // measuring the wrong rule rather than a broken layout.
        if viewport > 390 {
            driver.quit().await?;
            return Ok(());
        }
        let available = viewport - 32;
        assert_eq!(compact["mainWidth"], available);
        assert_eq!(compact["ceremonyWidth"], available);
        assert_eq!(compact["logoWidth"], 98);
        assert_eq!(compact["overflow"], false);
        assert_eq!(compact["undersized"], serde_json::json!([]));

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_join_targets_accessible_at_phone_sizes(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;

        for (width, height) in [(320_u32, 568_u32), (390, 844)] {
            emulate_phone(&driver, width, height).await?;
            goto(&driver, env.tonk_web.join("join")?.as_str()).await?;
            enter_guest(&driver).await?;
            element(&driver, ".join-view").await?;

            for dark in [false, true] {
                let geometry = driver
                    .execute(
                        r#"const dark = arguments[0];
                           document.documentElement.classList.toggle('wa-dark', dark);
                           document.documentElement.classList.toggle('wa-light', !dark);
                           const visible = [...document.querySelectorAll('a,button,input:not([type=hidden])')]
                             .filter(el => el.getClientRects().length > 0);
                           const mast = document.querySelector('.edge-mast').getBoundingClientRect();
                           const wordmark = document.querySelector('.edge-mast img').getBoundingClientRect();
                           const input = document.querySelector('.edge-input');
                           return {
                             width: innerWidth,
                             height: innerHeight,
                             overflow: document.documentElement.scrollWidth > innerWidth,
                             mast: { width: mast.width, height: mast.height },
                             wordmark: { width: wordmark.width, height: wordmark.height },
                             inputFont: getComputedStyle(input).fontSize,
                             undersized: visible.flatMap(el => {
                               const rect = el.getBoundingClientRect();
                               if (rect.width >= 44 && rect.height >= 44) return [];
                               return [{
                                 selector: el.className || el.id || el.tagName.toLowerCase(),
                                 width: rect.width,
                                 height: rect.height
                               }];
                             })
                           };"#,
                        vec![serde_json::json!(dark)],
                    )
                    .await?;
                let geometry = geometry.json();
                assert_eq!(geometry["width"], width);
                assert_eq!(geometry["height"], height);
                assert_eq!(geometry["overflow"], false);
                assert_eq!(geometry["undersized"], serde_json::json!([]));
                assert_eq!(geometry["inputFont"], "16px");
                assert_eq!(geometry["mast"]["width"], 98.0);
                assert!(
                    geometry["mast"]["height"].as_f64().unwrap_or_default() >= 44.0,
                    "the wordmark link needs a 44px hit area: {geometry}"
                );
                assert_eq!(geometry["wordmark"]["width"], 98.0);
                assert!(
                    geometry["wordmark"]["height"]
                        .as_f64()
                        .is_some_and(|height| height < 44.0),
                    "the visual wordmark must keep its existing scale: {geometry}"
                );
            }
            driver.enter_default_frame().await?;
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

        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        click(&driver, "#account-unlink").await?;
        element(&driver, "[role=alertdialog]").await?;
        click(&driver, "#account-delete-submit").await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;

        run_cluster_login(&driver, EMAIL).await?;
        if let Err(wait_error) = element(&driver, "tonk-account[data-mode=\"success\"]").await {
            let host = element(&driver, "tonk-account").await?;
            let mode = host.attr("data-mode").await?.unwrap_or_default();
            let error = element(&driver, "#account-error").await?.text().await?;
            // Whether the worker still answers at all separates a state
            // bug from a wedged worker.
            let health = driver
                .execute_async(
                    r#"const done = arguments[arguments.length - 1];
                       const timer = setTimeout(() => done({ timedOut: true }), 3000);
                       fetch("/api/health")
                           .then(async r => { clearTimeout(timer); done({ status: r.status, body: await r.text() }); })
                           .catch(e => { clearTimeout(timer); done({ error: String(e) }); });"#,
                    vec![],
                )
                .await
                .map(|value| value.json().clone());
            eprintln!("PROBE /api/health: {health:?}");
            for path in ["/api/account", "/api/account/summary"] {
                let answer = get_json(&driver, path).await;
                eprintln!("PROBE {path}: {answer:?}");
            }
            dump_browser_log(&driver, &env).await;
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

    /// A taken address is routed, not refused.
    ///
    /// Creation used to be chosen before the address was known, so
    /// typing one that already had an account ran creation against it
    /// and failed at the end — after the custody passkey existed. The
    /// cost was an orphaned credential in the authenticator per attempt,
    /// and the recovery was to retype and try again.
    ///
    /// The lookup answers first now, and the answer picks the ceremony:
    /// an address someone holds offers sign-in. Nothing is minted for
    /// the wrong one.
    #[dialog_common::test]
    async fn it_offers_sign_in_for_a_taken_address_without_minting(
        env: TestEnvironment,
    ) -> Result<()> {
        let existing_email = "existing@example.com";
        let available_email = "available@example.com";

        let creator = driver_with_prf(&env).await?;
        sign_up(&creator, &env, existing_email).await?;
        creator.quit().await?;

        let (driver, authenticator_id) = driver_with_prf_authenticator(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        element(&driver, "#account-choose-link")
            .await?
            .click()
            .await?;
        await_register_dialog(&driver).await?;

        // The taken address offers sign-in...
        type_into_register_dialog(&driver, existing_email).await?;
        await_register_action(&driver, "log in with your passkey").await?;
        assert_eq!(
            credential_count(&driver, &authenticator_id).await?,
            0,
            "an address that already has an account must mint nothing",
        );

        // ...and editing to a free one offers creation, in the same
        // cluster, with nothing to undo in between.
        type_into_register_dialog(&driver, available_email).await?;
        await_register_action(&driver, "create a passkey").await?;
        assert_eq!(
            credential_count(&driver, &authenticator_id).await?,
            0,
            "changing the address must not have minted anything either",
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_begins_only_one_registration_action_per_offered_step(
        env: TestEnvironment,
    ) -> Result<()> {
        let (driver, authenticator) = driver_with_prf_authenticator(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        click(&driver, "#account-choose-link").await?;
        await_register_dialog(&driver).await?;
        type_into_register_dialog(&driver, "one-action@example.com").await?;
        await_register_action(&driver, "create a passkey").await?;

        driver
            .execute(
                r#"const action = document.querySelector('#tonk-register-action');
                   const email = document.querySelector('#tonk-register-email');
                   action.click();
                   email.dispatchEvent(new KeyboardEvent('keydown', {
                     key: 'Enter', bubbles: true, cancelable: true
                   }));"#,
                Vec::new(),
            )
            .await?;
        await_credential_count(&driver, &authenticator, 1).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            credential_count(&driver, &authenticator).await?,
            1,
            "click and Enter in one turn must begin one passkey ceremony"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_retries_the_committed_address_after_a_failed_passkey_ceremony(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"choice\"]").await?;
        click(&driver, "#account-choose-link").await?;
        await_register_dialog(&driver).await?;

        driver
            .execute(
                r#"window.__registerCreateCalls = 0;
                   Object.defineProperty(navigator.credentials, 'create', {
                     configurable: true,
                     value: () => {
                       window.__registerCreateCalls += 1;
                       return Promise.reject(new DOMException(
                         'controlled passkey rejection', 'NotAllowedError'
                       ));
                     }
                   });"#,
                Vec::new(),
            )
            .await?;

        let email = "retry-committed@example.com";
        type_into_register_dialog(&driver, email).await?;
        await_register_action(&driver, "create a passkey").await?;
        click(&driver, "#tonk-register-action").await?;
        await_register_action(&driver, "create a passkey").await?;

        let first = driver
            .execute(
                r#"return {
                     calls: window.__registerCreateCalls,
                     row: (document.querySelector('#tonk-register-email-row')?.textContent || '').trim(),
                     status: (document.querySelector('#tonk-register-status')?.textContent || '').trim()
                   };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(first.json()["calls"], 1);
        assert!(
            first.json()["row"]
                .as_str()
                .is_some_and(|row| row.contains(email)),
            "the committed email receipt must remain visible: {}",
            first.json()
        );

        driver
            .execute(
                r#"const action = document.querySelector('#tonk-register-action');
                   action.click();
                   action.click();"#,
                Vec::new(),
            )
            .await?;
        await_register_action(&driver, "create a passkey").await?;
        let retried = driver
            .execute(
                r#"return {
                     calls: window.__registerCreateCalls,
                     row: (document.querySelector('#tonk-register-email-row')?.textContent || '').trim(),
                     status: (document.querySelector('#tonk-register-status')?.textContent || '').trim()
                   };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            retried.json()["calls"],
            2,
            "a sequential retry must run once, while its duplicate pending click is ignored"
        );
        assert!(
            retried.json()["row"]
                .as_str()
                .is_some_and(|row| row.contains(email)),
            "the retry must retain the original address receipt: {}",
            retried.json()
        );
        assert_ne!(
            retried.json()["status"],
            "Enter the address you want to use.",
            "retry must read the committed address after the live input is gone"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_submits_activation_once_while_the_request_is_pending(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let mut activation = env.tonk_web.join("activate")?;
        activation.set_query(Some("ucan=AA"));
        goto(&driver, activation.as_str()).await?;
        element(&driver, "#activate-accept").await?;

        let count = driver
            .execute_async(
                r#"const done = arguments[arguments.length - 1];
                   const original = window.fetch;
                   let requests = 0;
                   window.fetch = (...args) => {
                     const url = String(args[0]?.url || args[0]);
                     if (url.includes('/ucan/')) {
                       requests += 1;
                       return new Promise(() => {});
                     }
                     return original(...args);
                   };
                   const accept = document.querySelector('#activate-accept');
                   accept.click();
                   accept.click();
                   queueMicrotask(() => done({
                     requests,
                     disabled: accept.disabled,
                     busy: document.querySelector('#activate-confirm')?.getAttribute('aria-busy')
                   }));"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(count.json()["requests"], 1, "activation must post once");
        assert_eq!(count.json()["disabled"], true);
        assert_eq!(count.json()["busy"], "true");

        driver.quit().await?;
        Ok(())
    }

    fn tonk_bin() -> PathBuf {
        let path = std::env::var_os("TONK_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // Runtime variable first: under the `tests-e2e` archive
                // the compile-time path names the Nix build sandbox.
                let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
                    .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_string());
                PathBuf::from(manifest_dir)
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

    const LINK_ERROR_DIAGNOSTIC: &str = "tonk:test:link-error";

    /// Preserve the account panel's exact LinkCli diagnostic across the
    /// activation detour. The visible copy is deliberately safe/generic; this
    /// test-only probe makes a failed real-browser run identify which async
    /// boundary failed without changing production UI or logging broadly.
    async fn install_link_error_probe(driver: &WebDriver) -> Result<()> {
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools
            .execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                serde_json::json!({
                    "source": r#"
                        (() => {
                            if (globalThis.__tonkLinkErrorProbe) return;
                            globalThis.__tonkLinkErrorProbe = true;
                            const original = console.error.bind(console);
                            console.error = (...args) => {
                                try {
                                    const message = args[0];
                                    if (
                                        typeof message === "string" &&
                                        message.startsWith("account LinkCli failed:")
                                    ) {
                                        const scrubbed = message
                                            .replace(/\b[A-Za-z0-9+/_=-]{48,}\b/g, "[redacted]")
                                            .slice(0, 1000);
                                        sessionStorage.setItem("tonk:test:link-error", scrubbed);
                                    }
                                } catch {}
                                original(...args);
                            };
                        })();
                    "#,
                }),
            )
            .await?;
        Ok(())
    }

    async fn link_error_diagnostic(driver: &WebDriver) -> Option<String> {
        driver
            .execute(
                "return sessionStorage.getItem(arguments[0]);",
                vec![serde_json::json!(LINK_ERROR_DIAGNOSTIC)],
            )
            .await
            .ok()?
            .json()
            .as_str()
            .map(str::to_owned)
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
            command.args(["--service-url", env.access_service.as_str()]);
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

        if register_first {
            install_link_error_probe(driver).await?;
        }

        goto(driver, approval_url.as_str()).await?;
        if register_first {
            // A browser with no account yet registers before approving:
            // the link page opens on the signup panels, and the ceremony
            // that creates and enrolls the account flows straight into
            // the approval it was interrupted by.
            element(driver, "tonk-account[data-mode=\"choice\"]").await?;
            run_cluster_ceremony(driver, EMAIL).await?;
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
            goto(driver, approval_url.as_str()).await?;
        }
        element(driver, "tonk-account[data-mode=\"handoff\"]").await?;
        wait_for_text(driver, "#account-handoff-name", "e2e terminal").await?;
        // The DID is tucked behind the "technical details" disclosure,
        // and collapsed text reads as empty — open it the way a person
        // checking the fingerprint would.
        click(driver, "#account-handoff details summary").await?;
        let handoff_did = element(driver, "#account-handoff-did")
            .await?
            .text()
            .await?;
        assert_eq!(handoff_did, audience);
        element(driver, "#account-handoff-submit")
            .await?
            .click()
            .await?;
        // The callback's bridge page re-posts the fragment on loopback and
        // redirects back here, where the outcome uses the account styling.
        if let Err(wait_error) = element(driver, "tonk-account[data-mode=\"success\"]").await {
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
            let diagnostic = link_error_diagnostic(driver).await.unwrap_or_default();
            return Err(wait_error).context(format!(
                "approval stopped in mode {mode:?} at {url}; error={error:?}; \
                 status={working:?}; diagnostic={diagnostic:?}"
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
                // A mismatch here is usually the CLI exiting instead of
                // finishing (EOF reads as an empty line); its stderr says
                // why, so bring it into the failure instead of asserting
                // on the bare line.
                if outcome_line.trim_end() != "signed in" {
                    child.kill().await?;
                    let mut stderr_text = String::new();
                    use tokio::io::AsyncReadExt as _;
                    let _ = tokio::time::timeout(
                        Duration::from_secs(5),
                        stderr.read_to_string(&mut stderr_text),
                    )
                    .await;
                    return Err(anyhow!(
                        "the CLI never reported \"signed in\" (got {outcome_line:?}); \
                         its stderr: {stderr_text}"
                    ));
                }
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
                    // The flag is a consistency guard against the account's
                    // RECORDED provider, which the approving page named:
                    // the page-origin endpoint, not the direct address the
                    // harness spawned the service on.
                    "--service-url",
                    env.tonk_web.join("ucan")?.as_str(),
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
        // These calls are `fetch` FROM THE PAGE, answered by the service
        // worker — so a page has to be loaded and the worker has to be
        // controlling it. Without that the request leaves for the static
        // server, which has no `/api/*` and answers 405.
        driver.goto(env.tonk_web.as_str()).await?;
        wait_for_service_worker(&driver).await?;
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
                // `checking` is the question being asked, not an answer
                // to it: the handler writes it before the lookup runs so
                // the form can say it is working. Returning it would
                // report whatever was read first rather than what the
                // address turned out to be.
                if !state.is_empty() && state != tonk_schema::email_state::CHECKING {
                    return Ok(state.to_owned());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("no email-status answer for {address}: {rows}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// The Hub's own wizard creates a local-only space before anyone
    /// registers.
    ///
    /// Every other test here builds the claim in Rust, which skips the
    /// form entirely — so a hidden input that prefills a remote is
    /// invisible to them. This one submits the real wizard, which is how
    /// `<tonk-default-remote auto>` went on wiring `origin + /ucan/`
    /// onto spaces created with no account: the form supplied a remote,
    /// the worker honoured it as a deliberate choice, and the gate that
    /// keeps a space local never got a say. The space then synced to a
    /// service that refuses to serve it.
    #[dialog_common::test]
    async fn it_creates_a_local_only_space_from_the_hub_wizard(env: TestEnvironment) -> Result<()> {
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
                "the wizard never created a space; before={before:?} now={now:?}",
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };

        // Give the handler's post-navigation attach step room to run, so
        // "no remote" means it declined rather than that we looked early.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the space configuration", &info);
        assert!(
            info["remote"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "a space created before registering must wire no remote, got {}",
            info["remote"],
        );
        assert!(
            info["branch"]["main"]["upstream"].is_null(),
            "main must track nothing, got {}",
            info["branch"]["main"]["upstream"],
        );

        // The space is local-only, which is what makes sharing it
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

        // The share cannot finish until the address is confirmed: the
        // access service refuses to provision a customer that still
        // awaits activation ("the subject's own registration awaits
        // email activation"), so minting before this is asking for a
        // refusal, not for a link. In a second tab, because the cluster
        // is a DOM element with no persistence and this tab is holding
        // the ceremony that the share is waiting on.
        activate_in_another_tab(&driver, &env, "nobody@example.com").await?;

        // Confirmation comes home to the waiting cluster as a fact, and
        // the ceremony walks the rest of its steps: the address settles
        // as verified, the name commits, and the closing action is the
        // thing the share was for.
        await_row_value(&driver, "email", "verified").await?;
        type_into_settled_row(&driver, "display name", "Nobody").await?;
        await_register_action(&driver, "copy share link").await?;
        click_register_action(&driver).await?;

        // ...and THEN the share it interrupted finishes, which is the
        // feature: the space gains the remote it refused to share
        // without, and the invite link arrives.
        await_share_link(&driver, &key).await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_replaces_agent_link_progress_with_the_share_refusal(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        submit_hub_wizard(&driver).await?;
        let key = await_new_space(&driver, &before).await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;
        enter_space_view(&driver).await?;

        wait_for_displayed(&driver, ".local-invite-notice").await?;
        let canvas = element(&driver, ".blank-canvas__deeplink")
            .await?
            .text()
            .await?;
        assert!(
            canvas.contains("sharing unavailable"),
            "the settled refusal needs a neutral label: {canvas:?}"
        );
        assert!(
            !canvas.contains("Generating link"),
            "pending progress must disappear when refusal settles: {canvas:?}"
        );
        assert!(
            !canvas.contains("condition banner"),
            "the refusal must not point to absent UI: {canvas:?}"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_removes_a_space_without_letting_focus_escape_the_sealed_guest(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        submit_hub_wizard(&driver).await?;
        let key = await_new_space(&driver, &before).await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        let row = wait_for_displayed(&driver, ".srow-wrap").await?;
        driver
            .action_chain()
            .move_to_element_center(&row)
            .perform()
            .await?;
        let opener = wait_for_displayed(&driver, "[data-space-remove-open]").await?;
        opener.click().await?;
        wait_for_displayed(&driver, "tonk-dialog[data-space-remove-dialog]").await?;

        for _ in 0..8 {
            driver.action_chain().send_keys(Key::Tab).perform().await?;
            driver.enter_default_frame().await?;
            let outer = driver
                .execute(
                    r#"return document.activeElement?.matches('tonk-site > iframe') || false;"#,
                    Vec::new(),
                )
                .await?;
            assert_eq!(
                outer.json(),
                true,
                "Tab must not escape the sealed Hub while removal stays open"
            );

            enter_hub(&driver).await?;
            let guest = driver
                .execute(
                    r#"const dialog = document.querySelector('tonk-dialog[data-space-remove-dialog]');
                       const active = document.activeElement;
                       return {
                         open: dialog?.open || false,
                         inside: !!dialog && (active === dialog || dialog.contains(active))
                       };"#,
                    Vec::new(),
                )
                .await?;
            assert_eq!(guest.json()["open"], true);
            assert_eq!(
                guest.json()["inside"],
                true,
                "Tab focus left the open removal dialog: {}",
                guest.json()
            );
        }

        driver
            .action_chain()
            .send_keys(Key::Escape)
            .perform()
            .await?;
        let restored = driver
            .execute(
                r#"return {
                     open: document.querySelector('tonk-dialog[data-space-remove-dialog]')?.open || false,
                     opener: document.activeElement?.matches('[data-space-remove-open]') || false
                   };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(restored.json()["open"], false);
        assert_eq!(
            restored.json()["opener"],
            true,
            "Escape must restore the remove opener"
        );

        click(&driver, "[data-space-remove-open]").await?;
        wait_for_displayed(&driver, "tonk-dialog[data-space-remove-dialog]").await?;
        let association = driver
            .execute(
                r#"const button = document.querySelector('.m-go');
                   const form = document.querySelector('form[data-remove]');
                   return {
                     attribute: button?.getAttribute('form') || null,
                     associated: button?.form?.id || null,
                     expected: form?.id || null
                   };"#,
                Vec::new(),
            )
            .await?;
        let expected_form = association.json()["expected"]
            .as_str()
            .ok_or_else(|| anyhow!("the rendered remove form has no id: {}", association.json()))?;
        assert_eq!(
            association.json()["associated"].as_str(),
            Some(expected_form),
            "the rendered remove button must submit its row's form: {}",
            association.json()
        );
        click(&driver, ".m-go").await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let spaces = space_keys(&driver).await?;
            if !spaces.contains(&key) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "removed space {key:?} remained in the profile listing: {spaces:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        wait_for_absent(&driver, ".srow-wrap").await?;

        driver.quit().await?;
        Ok(())
    }

    /// Sign up in order to share, end to end, as a person does it.
    ///
    /// Every other test here reaches into the worker: it builds a claim
    /// in Rust, or polls a row, or asserts on `/api/repository`. Each of
    /// those passes while the thing the user touches is broken — which
    /// is how the registration dialog shipped latched on "Checking…"
    /// forever with a green suite.
    ///
    /// This one only clicks and reads. It is deliberately the longest
    /// test in the file, because the value is in the SEQUENCE: steps
    /// that pass alone still fail in order.
    ///
    /// The ceremony's later rows (passkey, verification, display name,
    /// and the closing copy-link) are not built yet, so this fails part
    /// way through by design — it is the specification of the flow, and
    /// what it reports is how far the flow actually gets.
    #[dialog_common::test]
    async fn it_signs_up_to_share_and_hands_over_the_link(env: TestEnvironment) -> Result<()> {
        let (driver, authenticator) = driver_with_prf_authenticator(&env).await?;

        // 1–2. The Hub, with nothing in it.
        driver.goto(env.tonk_web.as_str()).await?;
        let spaces = space_keys(&driver).await?;
        assert!(
            spaces.is_empty(),
            "a fresh profile has no spaces, got {spaces:?}"
        );

        // 3–4. Create one, and land in it.
        submit_hub_wizard(&driver).await?;
        let key = await_new_space(&driver, &spaces).await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;

        // 5–6. Share offers to log in: nothing is registered.
        open_share_stack(&driver).await?;
        await_share_row(&driver, "account").await?;

        // 7–8. The cluster comes up with the address field focused, so
        // typing works without aiming at anything.
        click_share_row(&driver, "[data-share-account]").await?;
        await_register_dialog(&driver).await?;
        assert_eq!(
            focused_element_id(&driver).await?,
            "tonk-register-email",
            "the address field must take focus when the cluster opens",
        );

        // 9–10. An address nobody has reveals the create step. The label
        // IS the routing decision, so asserting it covers the whole loop:
        // command dispatched, answer written, subscription delivered.
        let email = "alice@web.mail";
        type_into_register_dialog(&driver, email).await?;
        await_register_action(&driver, "create a passkey").await?;

        // 11–12. Running it waits on the platform, and says so.
        let before = credential_count(&driver, &authenticator).await?;
        click_register_action(&driver).await?;
        await_register_action(&driver, "waiting for your device").await?;

        // 12–13. The ceremony settles into a record naming the device.
        await_credential_count(&driver, &authenticator, before + 1).await?;
        let passkey = await_settled_row(&driver, "passkey").await?;
        assert!(
            passkey.contains(" on "),
            "the passkey row names the device, got {passkey:?}",
        );

        // 14. And the narrator asks for the emailed link.
        await_narrator_containing(&driver, "confirmation link").await?;

        // 15–17. Open it, accept, and come back — in the tab the
        // emailed link opens, which is also the only place the cluster
        // survives it.
        activate_in_another_tab(&driver, &env, email).await?;

        // 18. The email row settles: the address is confirmed.
        //
        // Waiting for the VALUE, not merely for a settled row: the row
        // is already settled at `awaiting confirmation` while the link
        // is out, so asking only "has it settled" answers yes before
        // activation has reached this tab at all.
        let staged: Result<()> = async {
            await_row_value(&driver, "email", "verified").await?;

            // 19. Then the name, typed and committed.
            type_into_settled_row(&driver, "display name", "Alice").await?;
            assert_eq!(await_settled_row(&driver, "display name").await?, "Alice");

            // 20–22. The closing action is the thing the share was for.
            await_register_action(&driver, "copy share link").await?;
            watch_clipboard(&driver).await?;
            click_register_action(&driver).await?;
            await_register_action(&driver, "copying link…").await?;
            await_narrator_containing(&driver, "invite someone into a space").await?;
            Ok(())
        }
        .await;
        if let Err(error) = staged {
            dump_browser_log(&driver, &env).await;
            return Err(error);
        }

        // 23. And it really is an invite.
        //
        // The narrator IS the observation. The link itself is
        // overlay-only by design — it carries the membership seed in its
        // fragment, so `Credential` and `InviteState` are asserted into
        // the session overlay and never written to a branch — which
        // means no query from here can read it, and the clipboard needs
        // a permission the harness's Chrome does not grant. What the
        // page says once it holds a link is the reachable proof that it
        // does, and the dialog only says it in that one branch.
        await_narrator_containing(&driver, "invite someone into a space").await?;

        // 24–25. A fresh profile opening it lands in the same space.
        //
        // The link comes from the clipboard, read in the page that just
        // wrote it: the copy is a user gesture, which is what grants the
        // permission, and the row behind it is overlay-only so no query
        // from out here can reach it.
        let invite = copied_text(&driver).await?;
        assert!(
            invite.contains("/join") || invite.contains("/@/"),
            "the copied link must be an invite, got {invite:?}",
        );
        let guest = driver_with_prf(&env).await?;
        guest.goto(&invite).await?;
        await_url_containing(&guest, &key).await?;

        guest.quit().await?;
        driver.quit().await?;
        Ok(())
    }

    /// The Hub offers to link an account, and does it in one step.
    ///
    /// It used to read "log in" and navigate to `/settings`, which put
    /// two surfaces between the label and the ceremony — press it, land
    /// on a panel, press "link an account" there, meet the cluster only
    /// then. It also named the wrong act: the address decides whether it
    /// creates a passkey or signs you in, so half of "log in"'s readers
    /// were told something untrue before they had typed anything.
    ///
    /// Both halves are asserted from the page, because both are what a
    /// person sees: the word on the control, and what one press does.
    #[dialog_common::test]
    async fn it_links_an_account_from_the_hub_in_one_step(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        // The word on the control, before anything is linked.
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, "[data-account-trigger]", "link an account").await?;

        // One press. The Hub is a sealed guest, so the cluster it asks
        // for is raised by the TOP page — which is also why pressing it
        // must not navigate the Hub anywhere.
        let before = driver.current_url().await?;
        click(&driver, "[data-account-trigger]").await?;
        await_register_dialog(&driver).await?;

        driver.enter_default_frame().await?;
        assert_eq!(
            driver.current_url().await?,
            before,
            "linking an account happens in place, with no page in between",
        );

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
                    "activated_at": {
                        "the": "xyz.tonk.account/activated-at",
                        "as": "UnsignedInteger", "cardinality": "one"
                    }
                } },
                "terms": {
                    "this": { "?": { "name": "account" } },
                    "activated_at": { "?": { "name": "activated_at" } },
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
        // Presence, not a status string: the row resolves only when the
        // account has an activation fact, so a row arriving at all is the
        // answer. The bar reads it the same way.
        assert!(
            rows[0]["fields"]["activated_at"].as_u64().is_some(),
            "and carry when it activated: {rows:?}",
        );
        // Where the account syncs is on the REGISTRATION, not here: it is
        // known at enrollment and unchanged by activation, which is what
        // lets a client attach its remote before the emailed link is
        // opened and learn it was activated from the gate answering 200.

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

        // `create_space` answers with the full DID; prefixing `did:key:`
        // again names nothing, and the bar then has no space to answer
        // about.
        let key = create_space(&driver, "Shareable").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/{key}"))?.as_str())
            .await?;
        await_share_row(&driver, "account").await?;

        sign_up(&driver, &env, "bar-flips@example.com").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/{key}"))?.as_str())
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
        wait_for_service_worker(&driver).await?;

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
            // Ask again, the way the form does every time the address is
            // typed. The answer is written when the question is asked;
            // nothing re-publishes it behind the scenes, so an answer
            // from before the account existed stays exactly as true as
            // it was when it was given.
            let asked = post_json(
                &driver,
                "/api/profile/branch/main/transact",
                check_email_claim_json(taken),
            )
            .await?;
            successful_body("re-ask account/check-email", &asked);
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
        // Register the address in a profile of its own, then ask about
        // it from a fresh one. The share row that raises the cluster is
        // only offered while THIS browser has no account, so a profile
        // that just signed up cannot reach the cluster to ask anything —
        // and the question here is what the lookup says about an address
        // someone else already holds.
        let owner = driver_with_prf(&env).await?;
        let taken = "taken@example.com";
        sign_up(&owner, &env, taken).await?;
        owner.quit().await?;

        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        open_register_dialog_from_a_space(&driver, &env, "Signed In").await?;
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
        // Registered elsewhere, for the same reason as
        // `it_offers_sign_in_for_an_address_that_already_has_an_account`:
        // a profile with its own account is never offered the row that
        // raises the cluster.
        let owner = driver_with_prf(&env).await?;
        let taken = "taken@example.com";
        sign_up(&owner, &env, taken).await?;
        owner.quit().await?;

        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        open_register_dialog_from_a_space(&driver, &env, "Edited Away").await?;

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

    /// The id of whatever currently has focus.
    async fn focused_element_id(driver: &WebDriver) -> Result<String> {
        let id = driver
            .execute(r##"return document.activeElement?.id || "";"##, Vec::new())
            .await?;
        Ok(id.json().as_str().unwrap_or_default().to_owned())
    }

    /// Wait until the address bar contains `fragment`.
    async fn await_url_containing(driver: &WebDriver, fragment: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let url = driver.current_url().await?;
            if url.as_str().contains(fragment) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("never navigated to {fragment}; still at {url}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait for a space key that was not there before, and return it.
    async fn await_new_space(driver: &WebDriver, before: &[String]) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let now = space_keys(driver).await?;
            if let Some(key) = now.iter().find(|key| !before.contains(key)) {
                return Ok(key.clone());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "no space was created; before={before:?} now={now:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait for a row to read `expected`.
    ///
    /// A row's value changes as its step advances (`awaiting
    /// confirmation` → `verified`), so the value is the observation and
    /// settledness alone is not.
    async fn await_row_value(driver: &WebDriver, noun: &str, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut last = String::new();
        loop {
            last = await_settled_row(driver, noun).await.unwrap_or(last);
            if last == expected {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the {noun:?} row never reached {expected:?}; it reads {last:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Read a settled row's value by its noun.
    ///
    /// A row settles when its step completes: the noun stays and the
    /// value becomes a record (`passkey  Chrome on macOS`). Waiting on
    /// the value is how a step's completion is observed.
    async fn await_settled_row(driver: &WebDriver, noun: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut last;
        loop {
            let value = driver
                .execute(
                    r##"
                    const noun = arguments[0];
                    // LAST match, not first: the ceremony stacks rows as
                    // it advances, and more than one can carry the same
                    // noun — the address row says which address, and the
                    // row below it says where its confirmation got to.
                    // The newest is the step being reported on.
                    let seen = "";
                    for (const row of document.querySelectorAll("#tonk-register .orow")) {
                        const k = row.querySelector(".k");
                        if (!k || k.textContent.trim() !== noun) continue;
                        const v = row.querySelector(".v");
                        // A row still being edited holds an input; a
                        // settled one holds text.
                        if (!v || v.querySelector("input")) continue;
                        seen = v.textContent.trim();
                    }
                    return seen;
                    "##,
                    vec![serde_json::json!(noun)],
                )
                .await?;
            last = value.json().as_str().unwrap_or_default().to_owned();
            if !last.is_empty() {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("the {noun:?} row never settled"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Type into the row named `noun` and commit it with Enter.
    async fn type_into_settled_row(driver: &WebDriver, noun: &str, value: &str) -> Result<()> {
        // The row unfolds a beat after the step before it settles — the
        // ceremony reads the account summary between "email · verified"
        // and asking for a name — so an absent row is waited out rather
        // than failed on the first look.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute_async(
                    r##"
                    const done = arguments[arguments.length - 1];
                    const [noun, value] = [arguments[0], arguments[1]];
                    for (const row of document.querySelectorAll("#tonk-register .orow")) {
                        const k = row.querySelector(".k");
                        if (!k || k.textContent.trim() !== noun) continue;
                        const input = row.querySelector("input");
                        if (!input) return done({ error: noun + " row takes no input" });
                        input.focus();
                        input.value = value;
                        input.dispatchEvent(new Event("input", { bubbles: true }));
                        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
                        return done({ ok: true });
                    }
                    done({ error: "no row named " + noun });
                    "##,
                    vec![serde_json::json!(noun), serde_json::json!(value)],
                )
                .await?;
            let outcome = outcome.json().clone();
            match outcome.get("error").and_then(|error| error.as_str()) {
                None => return Ok(()),
                Some(error) if error.starts_with("no row named") => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(anyhow!("could not fill the {noun:?} row: {error}"));
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Some(error) => return Err(anyhow!("could not fill the {noun:?} row: {error}")),
            }
        }
    }

    /// Wait for the narrator to say something containing `fragment`.
    async fn await_narrator_containing(driver: &WebDriver, fragment: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut last;
        loop {
            let text = driver
                .execute(
                    r##"const p = document.querySelector("#tonk-register-status");
                       return p ? (p.textContent || "").trim() : "";"##,
                    Vec::new(),
                )
                .await?;
            last = text.json().as_str().unwrap_or_default().to_owned();
            if last.to_lowercase().contains(&fragment.to_lowercase()) {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the narrator never said {fragment:?}; it reads {last:?}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Watch what the page copies.
    ///
    /// Installed BEFORE the copy runs, because reading the clipboard
    /// back is not available here: the permission is not granted to the
    /// harness, and granting it over CDP did not change the answer. What
    /// the page passes to `writeText` is the same string the person ends
    /// up with, and it is observable.
    async fn watch_clipboard(driver: &WebDriver) -> Result<()> {
        driver
            .execute(
                r##"
                window.__tonkCopied = "";
                const clipboard = navigator.clipboard;
                const write = clipboard.writeText.bind(clipboard);
                clipboard.writeText = (text) => {
                    window.__tonkCopied = text;
                    return write(text).catch(() => {});
                };
                "##,
                Vec::new(),
            )
            .await?;
        Ok(())
    }

    /// What the page passed to `writeText`, once it has.
    async fn copied_text(driver: &WebDriver) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let text = driver
                .execute(r##"return window.__tonkCopied || "";"##, Vec::new())
                .await?;
            let text = text.json().as_str().unwrap_or_default().to_owned();
            if !text.is_empty() {
                return Ok(text);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("the page never copied anything"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
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
        let mut last;
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
    ///
    /// Read from the SPACE's branch, keyed by the space, because that is
    /// where the mint writes: `enable-sync` attaches the remote and
    /// asserts `xyz.tonk.invite/url` on the space it shared. Profile main
    /// never carries it, so asking there answers `[]` for a share that
    /// worked — which is exactly the report this used to give. The
    /// dialog reads the same row to fill the clipboard, so this is the
    /// row the person ends up with, not a proxy for it.
    async fn await_share_link(driver: &WebDriver, space: &str) -> Result<String> {
        let ask = serde_json::json!({
            "predicate": { "with": {
                "status": {
                    "the": "xyz.tonk.invite/status", "as": "Entity", "cardinality": "one"
                },
                "url": {
                    "the": "xyz.tonk.invite/url", "as": "Text",
                    "cardinality": "one", "optional": true
                }
            } },
            "terms": {
                "this": space,
                "status": { "?": { "name": "status" } },
                "url": { "?": { "name": "url" } }
            }
        });
        let endpoint = format!("/api/repository/{space}/branch/main/query");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let rows = post_json(driver, &endpoint, ask.clone()).await?;
            if let Some(link) = rows["body"].as_array().and_then(|rows| {
                rows.iter().find_map(|row| {
                    row["fields"]["url"]
                        .as_str()
                        .or_else(|| row["url"].as_str())
                        .filter(|url| !url.is_empty())
                })
            }) {
                return Ok(link.to_owned());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "registering never finished the share it interrupted: \
                     no invite link. {space} answered: {rows}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Click the bar's `share` cell, which opens the share stack.
    ///
    /// The cell is in the bar's shadow root; the stack it reveals is
    /// slotted light content.
    async fn open_share_stack(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const bar = document.querySelector("tonk-fab");
                    if (!bar || !bar.shadowRoot) return false;
                    const cell = bar.shadowRoot.querySelector('[data-cell="share"]');
                    if (!cell) return false;
                    cell.click();
                    return true;
                    "##,
                    Vec::new(),
                )
                .await?;
            if outcome.json().as_bool() == Some(true) {
                driver.enter_default_frame().await?;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                driver.enter_default_frame().await?;
                return Err(anyhow!("the bar never showed a share cell to click"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Open the FAB's share stack and click one of its rows.
    ///
    /// Two DOM boundaries sit between the driver and the row. The bar
    /// lives in the sealed guest, at an opaque origin, so the browsing
    /// context has to be switched into it; and the cell that OPENS the
    /// stack lives in the bar's shadow root, while the row the stack
    /// holds is a slotted light child. Querying the light tree alone
    /// finds the row but never opens the stack it is hidden inside.
    async fn click_share_row(driver: &WebDriver, marker: &str) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const marker = arguments[0];
                    const bar = document.querySelector("tonk-fab");
                    if (!bar) return { error: "no bar" };
                    // The stack opens from a shadow cell.
                    const cell = bar.shadowRoot
                        && bar.shadowRoot.querySelector('[data-cell="share"]');
                    if (!cell) return { error: "no share cell" };
                    if (cell.getAttribute("aria-expanded") !== "true") cell.click();
                    // The rows are slotted light children, and each is a
                    // `<tonk-mi>` whose click listener sits on `.row`
                    // INSIDE its own shadow root — picking a row is the
                    // stack's only verb, and that is where it is heard.
                    // Clicking the host element reaches no listener, so
                    // the stack rendered, hovered, and did nothing.
                    const row = bar.querySelector(marker);
                    if (!row) return { error: "no row matching " + marker };
                    if (row.hasAttribute("hidden")) return { error: "row is hidden: " + marker };
                    const inner = row.shadowRoot && row.shadowRoot.querySelector(".row");
                    if (!inner) return { error: "row has no shadow .row: " + marker };
                    inner.click();
                    return { ok: true };
                    "##,
                    vec![serde_json::json!(marker)],
                )
                .await?;
            let value = outcome.json().clone();
            if value.get("ok").is_some() {
                // Back to the top page. A helper that leaves the driver
                // inside the guest hands the next one a context where
                // `navigator.serviceWorker.controller` is null and the
                // account UI does not exist — which reads as the worker
                // never taking control, from a page it never claimed.
                driver.enter_default_frame().await?;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let reason = value
                    .get("error")
                    .and_then(|error| error.as_str())
                    .unwrap_or("unknown");
                driver.enter_default_frame().await?;
                return Err(anyhow!("could not click the share row: {reason}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Which of the share stack's two rows the bar is offering.
    ///
    /// `log in to share` before an account exists, the copy row after —
    /// the visible half of the account subscription. Returns `None`
    /// while neither is showing.
    async fn share_row_offered(driver: &WebDriver) -> Result<Option<String>> {
        enter_guest(driver).await?;
        let outcome = driver
            .execute(
                r##"
                const bar = document.querySelector("tonk-fab");
                if (!bar) return null;
                const account = bar.querySelector("[data-share-account]");
                const link = bar.querySelector("[data-share-link]");
                if (account && !account.hasAttribute("hidden")) return "account";
                if (link && !link.hasAttribute("hidden")) return "link";
                return null;
                "##,
                Vec::new(),
            )
            .await?;
        driver.enter_default_frame().await?;
        Ok(outcome.json().as_str().map(str::to_owned))
    }

    /// Wait for the bar to offer `expected` (`account` or `link`).
    async fn await_share_row(driver: &WebDriver, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last;
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

    /// Raise the cluster from a space of its own.
    ///
    /// The bar is a space's control, so the Hub has no `tonk-fab` and no
    /// share row to take — reaching for one there fails with "no bar".
    /// A test that only wants the cluster still has to come at it the
    /// way a person does: from inside a space, through share.
    async fn open_register_dialog_from_a_space(
        driver: &WebDriver,
        env: &TestEnvironment,
        name: &str,
    ) -> Result<()> {
        // `create_space` answers with the full DID, so the path takes it
        // whole: prefixing `did:key:` again names a space that does not
        // exist, and the page then has nothing to raise a share from.
        let key = create_space(driver, name).await?;
        driver
            .goto(env.tonk_web.join(&format!("space/{key}"))?.as_str())
            .await?;
        // Open the stack, THEN take its row — the same two steps the
        // signup flow makes. Clicking the cell and the row in one pass
        // reaches for a row the stack has not rendered yet.
        open_share_stack(driver).await?;
        await_share_row(driver, "account").await?;
        open_register_dialog(driver).await
    }

    /// Wait for the registration cluster to be raised in the TOP page.
    ///
    /// It is raised there and nowhere else: WebAuthn needs a `window`
    /// and a user gesture, which neither the worker nor the profile
    /// frame has.
    async fn await_register_dialog(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
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
                // The raise crosses two realms — a guest row click, a
                // portal message, a top-page handler — and a plain
                // timeout says only that it did not arrive. Report both
                // ends so the next reader knows WHICH hop dropped it
                // rather than starting the bisect over.
                let diag = driver
                    .execute(
                        r##"
                        return {
                            url: location.href,
                            hasTonk: typeof window.tonk,
                            register: window.tonk && typeof window.tonk.register,
                            ids: Array.from(document.querySelectorAll("[id]"))
                                .map(n => n.id).filter(i => i.includes("register")),
                            dialogs: Array.from(document.querySelectorAll("dialog, .tonk-ceremony, .tonk-cluster"))
                                .map(n => n.tagName + "." + n.className),
                        };
                        "##,
                        Vec::new(),
                    )
                    .await
                    .map(|value| value.json().to_string())
                    .unwrap_or_else(|error| format!("diagnostic failed: {error}"));
                // And the guest side: did the row exist, was it hidden,
                // and does the guest have the bridge method the click
                // forwards through?
                enter_guest(driver).await?;
                let guest = driver
                    .execute(
                        r##"
                        const bar = document.querySelector("tonk-fab");
                        const row = bar && bar.querySelector("[data-share-account]");
                        return {
                            bar: !!bar,
                            row: !!row,
                            rowHidden: row ? row.hasAttribute("hidden") : null,
                            tonk: typeof window.tonk,
                            register: (window.tonk && typeof window.tonk.register) || null,
                            space: bar ? bar.getAttribute("space") : null,
                        };
                        "##,
                        Vec::new(),
                    )
                    .await
                    .map(|value| value.json().to_string())
                    .unwrap_or_else(|error| format!("guest diagnostic failed: {error}"));
                driver.enter_default_frame().await?;
                return Err(anyhow!(
                    "the share refusal never raised the cluster.\n  \
                     top page: {diag}\n  guest: {guest}"
                ));
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
        let mut last;
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

    /// Create a space from the Hub, the way a person does.
    ///
    /// Through WebDriver's frame switching, not
    /// `iframe.contentDocument`: the Hub renders in a SEALED guest at an
    /// opaque origin, so script in the outer page cannot reach its
    /// document at all — a reach-in returns `no guest frame` and says
    /// nothing about the app.
    async fn submit_hub_wizard(driver: &WebDriver) -> Result<()> {
        enter_hub(driver).await?;
        wait_for_displayed(driver, ".snew").await?.click().await?;
        // Back to the top document: everything after this — the space
        // page, the bar, the cluster — lives there.
        driver.enter_default_frame().await?;
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
        wait_for_service_worker(driver).await?;
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

    /// The callback roundtrip is settled only when the linked CLI sees both
    /// sides of the handoff. The CLI self-describes its own row locally, so a
    /// terminal-only list is an intermediate state, not proof that the
    /// signing browser's account facts have converged.
    fn callback_device_rows_ready(rows: &[CliDeviceRow]) -> bool {
        let browser = rows
            .iter()
            .any(|row| row.status == "active" && row.name.starts_with("Chrome on "));
        let terminal = rows
            .iter()
            .any(|row| row.status == "active" && row.name == "e2e terminal" && row.this_device);
        browser && terminal
    }

    async fn wait_for_callback_device_rows(
        profile: &TempDir,
        env: &TestEnvironment,
    ) -> Result<(CliOutput, Vec<CliDeviceRow>)> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let output = devices(profile, env).await?;
            if !output.status.success() {
                return Err(anyhow!("devices failed: {}", output.stderr));
            }
            let rows = device_rows(&output.stdout)?;
            if callback_device_rows_ready(&rows) {
                return Ok((output, rows));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the callback device list never converged to the signing browser and linked terminal: {}\n--- devices stderr ---\n{}",
                    output.stdout,
                    output.stderr
                ));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    #[test]
    fn callback_device_readiness_rejects_the_terminal_only_intermediate_state() {
        let terminal = CliDeviceRow {
            status: "active".into(),
            name: "e2e terminal".into(),
            did: "did:key:zTerminal".into(),
            this_device: true,
        };
        assert!(!callback_device_rows_ready(std::slice::from_ref(&terminal)));

        let browser = CliDeviceRow {
            status: "active".into(),
            name: "Chrome on Linux".into(),
            did: "did:key:zBrowser".into(),
            this_device: false,
        };
        assert!(callback_device_rows_ready(&[terminal, browser]));
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
        // The ceremony is still standing, and it is what the person is
        // looking at, so it is where the unfinished step is named. The
        // panel's own pending banner is behind it, and only renders once
        // the cluster comes down — which this test never does.
        await_narrator_containing(&driver, "confirmation link").await?;

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

    /// An enrolled account stays an account everywhere while its email is
    /// still unconfirmed.
    ///
    /// The account customer row has no provider until activation. The FABB
    /// used to require that optional field in its query, so this exact state
    /// resolved as no row: the space offered "log in to share" and raised the
    /// signup ceremony even though the account already existed.
    #[dialog_common::test]
    async fn it_names_pending_activation_consistently_in_a_space(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "pending-space@example.com";
        enroll_only(&driver, &env, email).await?;
        dismiss_register_dialog(&driver).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text(
            &driver,
            "#account-registration-value",
            "Waiting for email confirmation",
        )
        .await?;

        let key = create_space(&driver, "Waiting for Email").await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;

        enter_guest(&driver).await?;
        let banner = wait_for_displayed(&driver, "#fabb-activation-banner").await?;
        let banner_text = banner.text().await?;
        assert!(
            banner_text.contains(email) && banner_text.contains("waiting for email confirmation"),
            "the space must name the existing account's pending step: {banner_text:?}",
        );
        driver.enter_default_frame().await?;

        open_share_stack(&driver).await?;
        await_share_row(&driver, "link").await?;
        enter_guest(&driver).await?;
        let share_copy = driver
            .execute(
                r#"const bar = document.querySelector('tonk-fab');
                   return {
                     accountHidden: bar?.querySelector('[data-share-account]')?.hasAttribute('hidden'),
                     link: (bar?.querySelector('[data-share-link]')?.textContent || '').trim()
                   };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            share_copy.json()["accountHidden"],
            true,
            "a pending account must not offer login or signup: {}",
            share_copy.json(),
        );
        assert!(
            share_copy.json()["link"]
                .as_str()
                .is_some_and(|text| text.contains("confirm your email to share")),
            "the share row must name activation as the missing step: {}",
            share_copy.json(),
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
        // Take the ceremony down by its own back button, which is what
        // puts the panel behind it back in view. The pending banner is
        // the panel's, and it is the same row that reads "Active" once
        // the emailed link is opened below — before and after on one
        // surface.
        dismiss_register_dialog(&driver).await?;
        if let Err(error) =
            wait_for_text_containing(&driver, "#account-activation-notice", "activation pending")
                .await
        {
            for path in ["/api/account", "/api/customer", "/api/identity/root"] {
                let answer = get_json(&driver, path).await;
                eprintln!("PROBE {path}: {answer:?}");
            }
            let mode = element(&driver, "tonk-account")
                .await?
                .attr("data-mode")
                .await?;
            eprintln!("PROBE panel mode: {mode:?}");
            dump_browser_log(&driver, &env).await;
            return Err(error);
        }

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
    async fn it_backs_up_a_claimed_space_for_another_account_device(
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
        goto(&claimer, env.tonk_web.as_str()).await?;
        wait_for_service_worker(&claimer).await?;
        goto(&claimer, env.tonk_web.join("settings")?.as_str()).await?;
        element(&claimer, "tonk-account[data-mode=\"choice\"]").await?;
        run_cluster_login(&claimer, "claimer@example.com").await?;
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
    async fn it_keeps_destructive_dialog_controls_inside_a_short_mobile_viewport(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "short-mobile@example.com";
        sign_up(&driver, &env, email).await?;

        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        driver.set_window_rect(0, 0, 320, 568).await?;
        click(&driver, "#account-delete-review").await?;
        wait_for_displayed(&driver, "#account-delete-arming").await?;

        // The number of owned spaces varies by account. Fill the real plan list
        // to its maximum-height state so this remains a geometry regression,
        // rather than depending on a costly remote-space fixture.
        driver
            .execute(
                r#"const list = document.querySelector('#account-delete-spaces');
                   for (let index = list.children.length; index < 8; index += 1) {
                     const item = document.createElement('li');
                     item.textContent = `owned space ${index + 1} with a deliberately long name`;
                     list.append(item);
                   }"#,
                Vec::new(),
            )
            .await?;
        element(&driver, "#account-delete-email")
            .await?
            .send_keys(email)
            .await?;
        element(&driver, "#account-delete-understood")
            .await?
            .click()
            .await?;

        for (width, height) in [(320, 568), (390, 844)] {
            driver.set_window_rect(0, 0, width, height).await?;
            let geometry = driver
                .execute(
                    r#"const dialog = document.querySelector('.account__dialog');
                   dialog.scrollTop = dialog.scrollHeight;
                   const bounds = dialog.getBoundingClientRect();
                   const inspect = selector => {
                     const element = document.querySelector(selector);
                     const rect = element.getBoundingClientRect();
                     element.focus();
                     return {
                       top: rect.top,
                       bottom: rect.bottom,
                       visible: rect.top >= 0 && rect.bottom <= innerHeight,
                       focused: document.activeElement === element,
                       disabled: element.disabled
                     };
                   };
                   return {
                     top: bounds.top,
                     bottom: bounds.bottom,
                     viewport: innerHeight,
                     cancel: inspect('#account-confirm-cancel'),
                     submit: inspect('#account-delete-submit')
                   };"#,
                    Vec::new(),
                )
                .await?;
            let geometry = geometry.json();
            let top = geometry["top"].as_f64().unwrap_or(f64::NEG_INFINITY);
            let bottom = geometry["bottom"].as_f64().unwrap_or(f64::INFINITY);
            let viewport = geometry["viewport"].as_f64().unwrap_or_default();
            assert!(
                top >= 0.0,
                "dialog starts above the {width}x{height} viewport: {geometry}"
            );
            assert!(
                bottom <= viewport,
                "dialog ends below the {width}x{height} viewport: {geometry}"
            );
            for action in ["cancel", "submit"] {
                assert_eq!(
                    geometry[action]["visible"], true,
                    "{action} must remain visible at {width}x{height}: {geometry}"
                );
                assert_eq!(
                    geometry[action]["focused"], true,
                    "{action} must remain focusable at {width}x{height}: {geometry}"
                );
                assert_eq!(
                    geometry[action]["disabled"], false,
                    "{action} must be enabled in the armed state: {geometry}"
                );
            }
        }

        driver.quit().await?;
        Ok(())
    }

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

        // Creating a space navigates the page into it — the handler
        // sends the client there once the replica lands — so the
        // deletion controls are no longer on screen. Go back to where
        // they live.
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account").await?;

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
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, ".stack", "First Garden").await?;
        driver.enter_default_frame().await?;

        // Add account first opens a reversible Choice flow. It must not
        // rotate or grow the profile roster until a ceremony is submitted.
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
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

        run_cluster_ceremony(&driver, "second@example.com").await?;
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
        goto(&driver, env.tonk_web.as_str()).await?;
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

        // The sealed Hub routes settings into the top-level account page,
        // which reads real account and device facts and keeps unsupported
        // Usage/Syncing surfaces absent.
        click(&driver, "[data-account-trigger]").await?;
        click(&driver, "[data-open-settings]").await?;
        driver.enter_default_frame().await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        wait_for_text(&driver, "#account-email-value", "second@example.com").await?;
        assert_eq!(
            element(&driver, "#account-passkey-device-value")
                .await?
                .prop("textContent")
                .await?
                .as_deref(),
            Some(passkey_created_on.as_str()),
            "settings must render the account summary's passkey creation device"
        );
        click(&driver, "#account-tab-devices").await?;
        wait_for_text_containing(&driver, "#account-device-list", "this device").await?;
        let settings_text = element(&driver, "tonk-account")
            .await?
            .text()
            .await?
            .to_ascii_lowercase();
        for forbidden in ["usage", "upgrade", "metering", "syncing"] {
            assert!(
                !settings_text.contains(forbidden),
                "settings must not contain {forbidden}"
            );
        }

        // The authoritative display-name write repaints the Hub trigger and
        // remains in the field after the settings page is reloaded.
        click(&driver, "#account-tab-account").await?;
        let display_name = element(&driver, "#account-display-name").await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Command + "a"
        } else {
            Key::Control + "a"
        };
        display_name.send_keys(select_all).await?;
        display_name.send_keys("Second Hub").await?;
        display_name.send_keys(Key::Enter).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let input = element(&driver, "#account-display-name").await?;
            if input.prop("value").await?.as_deref() == Some("Second Hub")
                && input.attr("data-confirmed-name").await?.as_deref() == Some("Second Hub")
                && input.attr("aria-busy").await?.is_none()
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let error = element(&driver, "#account-display-name-error")
                    .await?
                    .prop("textContent")
                    .await?
                    .unwrap_or_default();
                return Err(anyhow!(
                    "timed out waiting for the second account display name to save: {error}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let settings = driver.current_url().await?;
        goto(&driver, settings.as_str()).await?;
        wait_for_value(&driver, "#account-display-name", "Second Hub").await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text(&driver, "[data-account-label]", "Second Hub").await?;

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
        // The approving page names the service its deployment uses, and
        // the CLI records that answer over its own `--service-url` guess
        // — so the record is the page-origin endpoint Caddy proxies, not
        // the direct address the harness spawned the service on.
        assert_eq!(url::Url::parse(provider)?, env.tonk_web.join("ucan")?);
        assert!(linked.link.stdout.contains("signed in"));

        // The approving page describes the terminal's row and pushes the
        // account branch best-effort before it delivers the grant; a push
        // that loses the race to the CLI's own bounded pull leaves one side
        // for the next sync sweep. The CLI also self-describes locally, so
        // its terminal row can appear before the signing browser's remote
        // row. Each `devices` call pulls again: wait for BOTH sides rather
        // than exiting on that guaranteed local row.
        let (devices, device_rows) = wait_for_callback_device_rows(&linked.profile, &env).await?;
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
        // Same page-named record as the flagged variant above: the
        // deployment's own endpoint, not the harness's direct address.
        assert_eq!(url::Url::parse(provider)?, env.tonk_web.join("ucan")?);

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
    /// The CLI's half is a loopback server that accepts a bodyless GET, serves
    /// a fragment bridge, then accepts one same-origin form POST. A test needs
    /// no CLI process to play that part, only the same contract. It hands back
    /// whatever the page delivered.
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

        async fn bridge() -> axum::response::Html<&'static str> {
            axum::response::Html(
                r##"<!doctype html>
<meta charset="utf-8">
<p>Returning authorization to Tonk…</p>
<script>
  const fields = new URLSearchParams(window.location.hash.slice(1));
  history.replaceState(null, "", window.location.pathname + window.location.search);
  const form = document.createElement("form");
  form.method = "post";
  form.action = window.location.pathname + window.location.search;
  for (const [name, value] of fields) {
    const input = document.createElement("input");
    input.type = "hidden";
    input.name = name;
    input.value = value;
    form.appendChild(input);
  }
  document.body.appendChild(form);
  form.submit();
</script>
"##,
            )
        }

        let app = axum::Router::new()
            .route("/", axum::routing::get(bridge).post(deliver))
            .with_state(slot);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((url, receiver))
    }

    /// The browser half of `tonk account login --via`: the page reads the
    /// waiting profile's DID and callback out of the URL, runs a real passkey
    /// ceremony, and returns the grant through the loopback bridge.
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
        goto(&driver, url.as_str()).await?;

        // The panel names the profile that is waiting, so the user knows what
        // they are approving. The DID sits behind the "technical
        // details" disclosure; collapsed text reads as empty.
        element(&driver, "tonk-account[data-mode=\"handoff\"]").await?;
        click(&driver, "#account-handoff details summary").await?;
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
        // the device registration before the callback navigation, and a loaded
        // CI runner stretches each of them.
        let (field, value) = tokio::time::timeout(Duration::from_secs(60), delivered)
            .await
            .context("the page never delivered an authorization")??;
        assert_eq!(field, "authorize", "approving must deliver a grant");

        // What arrived is what the CLI decodes: base64 over a payload
        // carrying the delegation, descriptor, exact service attachment, and
        // provider needed for a crash-safe CLI activation.
        let decoded = base64::engine::general_purpose::STANDARD.decode(&value)?;
        let payload: serde_json::Value = serde_json::from_slice(&decoded)?;
        for field in [
            "delegationHex",
            "descriptorHex",
            "attachmentId",
            "serviceUrl",
        ] {
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
        goto(&driver, url.as_str()).await?;

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
        // Cache the complete callback view before revocation. The CLI creates
        // its own row locally, so a terminal-only list does not prove it has
        // pulled the browser row that must remain visible in its stale cache.
        let (_listed, listed_rows) = wait_for_callback_device_rows(&linked.profile, &env).await?;
        let cli_did = did_for_device(&listed_rows, "e2e terminal")
            .context("CLI device was absent from the account device list")?
            .to_string();

        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        element(&driver, "tonk-account[data-mode=\"success\"]").await?;
        // The device list lives on the Devices tab, whose pane is
        // hidden until selected — and hidden text reads as empty.
        click(&driver, "#account-tab-devices").await?;
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
    /// before the encryption key existed did: the account passkey signs
    /// this device in, and the root is stored WITHOUT the key. The
    /// account's virtual authenticator must already hold the passkey.
    ///
    /// Built on today's ceremony surface: `authorizeDevice` unlocks the
    /// account and mints the `account → device` grant (the ceremony
    /// `unlockWithPasskey` became), and the two worker saves replay what
    /// a legacy page persisted — the root save deliberately omitting the
    /// `encryptionKey` the modern path would carry. The credential id is
    /// read from the virtual authenticator over CDP: it is the value a
    /// legacy page had stored, and the custody relay later asserts
    /// against exactly that credential.
    async fn legacy_link(
        driver: &WebDriver,
        env: &TestEnvironment,
        authenticator_id: &str,
    ) -> Result<()> {
        let identify = get_json(driver, "/api/identify").await?;
        let device_did = successful_body("identify", &identify)["did"]
            .as_str()
            .context("identify omitted the device DID")?
            .to_string();

        use base64::Engine as _;
        let devtools = ChromeDevTools::new(driver.handle.clone());
        let held = devtools
            .execute_cdp_with_params(
                "WebAuthn.getCredentials",
                serde_json::json!({ "authenticatorId": authenticator_id }),
            )
            .await?;
        let credential_id = held["credentials"]
            .get(0)
            .and_then(|credential| credential["credentialId"].as_str())
            .context("the virtual authenticator holds no credential to link with")?;
        let credential_id = hex::encode(
            base64::engine::general_purpose::STANDARD
                .decode(credential_id)
                .context("CDP credential id is not base64")?,
        );

        let ceremony = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const [deviceDid] = arguments;
                window.tonkIdentity.authorizeDevice({
                    deviceDid,
                    remote: `${window.location.origin}/ucan/`,
                    endpoint: `${window.location.origin}/ucan/`,
                }).then(authorized => done({ authorized }))
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![serde_json::json!(device_did)],
            )
            .await?
            .json()
            .clone();
        anyhow::ensure!(
            ceremony.get("error").is_none(),
            "legacy link ceremony failed: {ceremony}"
        );
        let authorized = &ceremony["authorized"];

        let saved = post_json(
            driver,
            "/api/identity/root",
            serde_json::json!({
                "credentialId": credential_id,
                "delegationHex": authorized["delegationHex"],
            }),
        )
        .await?;
        successful_body("legacy root save", &saved);
        let attached = post_json(
            driver,
            "/api/account/attach",
            serde_json::json!({
                "provider": env.tonk_web.join("ucan/")?,
                "rootDid": authorized["rootDid"],
                "credentialId": credential_id,
                "delegationHex": authorized["delegationHex"],
                "remote": env.tonk_web.join("ucan/")?,
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
        let (creator, authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&creator, &env, EMAIL).await?;
        // A second device on the same account, in the same session so the
        // virtual authenticator still holds the passkey: "Add account"
        // rotates the worker onto a fresh profile with no root of its own,
        // which is exactly what a new browser is.
        let added = post_json(&creator, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add profile", &added);
        goto(&creator, env.tonk_web.as_str()).await?;
        legacy_link(&creator, &env, &authenticator).await?;

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
        // account's X25519 recipient — a `SecretPrincipal` row naming the
        // space, whose `seed` points at the `SecretMessage` carrying the
        // sealed bytes, whose `to` is the recipient. The facts follow the
        // seal, so poll for them rather than assert on the first read.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let recipient = loop {
            let principals = post_json(
                &creator,
                "/api/profile/branch/main/query",
                serde_json::json!({
                    "terms": {
                        "this": { "?": { "name": "this" } },
                        "seed": { "?": { "name": "seed" } }
                    },
                    "predicate": {
                        "with": {
                            "seed": { "the": "xyz.tonk.secret/seed", "cardinality": "one", "as": "Entity" }
                        }
                    }
                }),
            )
            .await?;
            let principals = principals["body"].as_array().cloned().unwrap_or_default();
            let seed = principals.iter().find_map(|row| {
                let subject = row["fields"]["this"].as_str().unwrap_or_default();
                let seed = row["fields"]["seed"].as_str().unwrap_or_default();
                (subject.ends_with(&key) && !seed.is_empty()).then(|| seed.to_string())
            });
            if let Some(seed) = seed {
                let messages = post_json(
                    &creator,
                    "/api/profile/branch/main/query",
                    serde_json::json!({
                        "terms": {
                            "this": { "?": { "name": "this" } },
                            "to": { "?": { "name": "to" } }
                        },
                        "predicate": {
                            "with": {
                                "to": { "the": "xyz.tonk.secret/to", "cardinality": "one", "as": "Entity" }
                            }
                        }
                    }),
                )
                .await?;
                let messages = messages["body"].as_array().cloned().unwrap_or_default();
                if let Some(sealed_to) = messages.iter().find_map(|row| {
                    let envelope = row["fields"]["this"].as_str().unwrap_or_default();
                    let sealed_to = row["fields"]["to"].as_str().unwrap_or_default();
                    (envelope == seed && !sealed_to.is_empty()).then(|| sealed_to.to_string())
                }) {
                    break sealed_to;
                }
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the new space's seed was never custodied: {principals:?}"
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
