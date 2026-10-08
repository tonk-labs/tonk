//! Real-browser account-panel and UI↔CLI roundtrip tests.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::process::{ExitStatus, Stdio};
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow};
    use tempfile::TempDir;
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::prelude::*;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::{Child, Command};

    use crate::helpers::{TestEnvironment, driver_with_prf, driver_with_prf_authenticator, goto};

    fn assert_prompt_command(prompt: &str, origin: &url::Url, invite: &str) -> Result<()> {
        let loopback = matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let origin = origin.origin().ascii_serialization();
        let command = if origin != "https://tonk.network" {
            let executable = if loopback {
                "tonk"
            } else {
                "npx --yes @tonk/cli"
            };
            format!("{executable} join --via \"{origin}\" '{invite}'")
        } else {
            format!("npx --yes @tonk/cli join '{invite}'")
        };
        anyhow::ensure!(prompt.contains(&command), "rendered agent prompt: {prompt}");
        if loopback {
            anyhow::ensure!(!prompt.contains("npx --yes @tonk/cli"));
        }
        anyhow::ensure!(!prompt.contains("TONK_CONNECTION_ORIGIN="));
        Ok(())
    }

    /// Going home from a space swaps the space chrome for the Hub inside the
    /// same guest frame. The site display must see the route change before
    /// the space chrome nested in it does: otherwise the chrome loses its
    /// row first and re-renders its nested displays against a blank
    /// context, which flashed "Model not found" on the tab-title display.
    /// The blank re-render is checked for directly, since how long the
    /// flash stays up depends on how slow the branch is to poll.
    #[dialog_common::test]
    async fn it_switches_between_a_space_and_the_hub_without_an_absence_flash(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        create_space(&driver, "Navigation test").await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_displayed(&driver, ".space-card > a.srow").await?;

        let origin = driver
            .execute(
                r#"window.__absences = [];
                   const note = (element) => {
                     const state = element.getAttribute && element.getAttribute('data-state');
                     if (state === 'no-model') {
                       window.__absences.push(`no-model ${element.getAttribute('model') || '?'}`);
                     }
                   };
                   new MutationObserver((records) => {
                     for (const record of records) {
                       // A display re-rendered against a blank context: its
                       // routing attributes go from a value to nothing. That
                       // is the render the flash comes from, however briefly
                       // it shows.
                       if (record.type === 'attributes'
                           && record.attributeName !== 'data-state'
                           && record.target.localName === 'tonk-display'
                           && record.oldValue
                           && !record.target.getAttribute(record.attributeName)) {
                         window.__absences.push(
                           `blank ${record.attributeName} on ${record.target.getAttribute('model') || '?'}`);
                       }
                       note(record.target);
                       for (const node of record.addedNodes || []) {
                         if (node.nodeType === 1) {
                           note(node);
                           node.querySelectorAll('[data-state]').forEach(note);
                         }
                       }
                     }
                   }).observe(document, {
                     subtree: true, childList: true,
                     attributes: true, attributeOldValue: true,
                     attributeFilter: ['data-state', 'with', 'entity'],
                   });
                   return performance.timeOrigin;"#,
                Vec::new(),
            )
            .await?
            .json()
            .clone();

        for _ in 0..8 {
            enter_guest(&driver).await?;
            driver
                .find(By::Css(".space-card > a.srow"))
                .await?
                .click()
                .await?;
            driver.enter_default_frame().await?;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while !driver.current_url().await?.path().starts_with("/space/") {
                anyhow::ensure!(
                    tokio::time::Instant::now() < deadline,
                    "the space row did not open its space"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            enter_space_view(&driver).await?;
            wait_for_displayed(&driver, ".blank-canvas").await?;
            driver.enter_default_frame().await?;
            driver.execute("history.back()", Vec::new()).await?;
            enter_hub(&driver).await?;
            wait_for_displayed(&driver, ".space-card > a.srow").await?;
        }

        enter_guest(&driver).await?;
        let observed = driver
            .execute(
                "return { origin: performance.timeOrigin, absences: window.__absences || null };",
                Vec::new(),
            )
            .await?
            .json()
            .clone();
        anyhow::ensure!(
            observed["origin"] == origin,
            "the guest frame was replaced, so the observer missed the switches: {observed}"
        );
        anyhow::ensure!(
            observed["absences"] == serde_json::json!([]),
            "a display rendered against a blank context while switching: {observed}"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_opens_the_hub_without_creating_a_welcome_space(env: TestEnvironment) -> Result<()> {
        let driver = env.blank_driver().await?;
        for _ in 0..2 {
            goto(&driver, env.tonk_web.as_str()).await?;
            enter_hub(&driver).await?;
            driver.enter_default_frame().await?;
            assert_eq!(driver.current_url().await?.path(), "/");
            let reply = get_json(&driver, "/api/profile").await?;
            let profile = successful_body("list spaces after a root visit", &reply);
            let profile: tonk_worker::ProfileInfo = serde_json::from_value(profile.clone())?;
            assert!(
                profile.space.is_empty(),
                "root visits must not create spaces"
            );
        }
        driver.quit().await?;
        Ok(())
    }

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

    fn retryable_find_error(error: &thirtyfour::error::WebDriverErrorInner) -> bool {
        matches!(
            error,
            thirtyfour::error::WebDriverErrorInner::NoSuchElement(_)
                | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
        )
    }

    fn retryable_element_read_error(error: &thirtyfour::error::WebDriverErrorInner) -> bool {
        // Not interactable is a race too: a menu row before its menu has
        // opened, a control before the display it waits on has resolved.
        matches!(
            error,
            thirtyfour::error::WebDriverErrorInner::NoSuchElement(_)
                | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
                | thirtyfour::error::WebDriverErrorInner::ElementNotInteractable(_)
        )
    }

    #[test]
    fn it_only_retries_expected_dom_races() {
        use thirtyfour::error::{WebDriverErrorInfo, WebDriverErrorInner, WebDriverErrorValue};

        let info = |error: &str| WebDriverErrorInfo {
            status: 400,
            error: error.to_string(),
            value: WebDriverErrorValue::new(error.to_string()),
        };

        assert!(retryable_find_error(&WebDriverErrorInner::NoSuchElement(
            info("no such element")
        )));
        assert!(retryable_find_error(
            &WebDriverErrorInner::StaleElementReference(info("stale element reference"))
        ));
        assert!(!retryable_find_error(
            &WebDriverErrorInner::InvalidSelector(info("invalid selector"))
        ));
        assert!(retryable_element_read_error(
            &WebDriverErrorInner::StaleElementReference(info("stale element reference"))
        ));
        assert!(retryable_element_read_error(
            &WebDriverErrorInner::ElementNotInteractable(info("element not interactable"))
        ));
        assert!(retryable_element_read_error(
            &WebDriverErrorInner::NoSuchElement(info("no such element"))
        ));
        assert!(!retryable_element_read_error(
            &WebDriverErrorInner::ElementClickIntercepted(info("element click intercepted"))
        ));
    }

    async fn page_diagnostic_state(driver: &WebDriver) -> serde_json::Value {
        driver
            .execute(
                r##"
                const account = document.querySelector("tonk-account");
                const error = document.querySelector("#account-error");
                const firstSegment = location.pathname.split("/").filter(Boolean)[0] || "";
                const path = ["", "account", "activate", "settings"].includes(firstSegment)
                    ? `/${firstSegment}`
                    : "/<redacted>";
                let controlled = false;
                try { controlled = !!navigator.serviceWorker?.controller; } catch (_) {}
                return {
                    path,
                    addingAccount: new URLSearchParams(location.search).has("add"),
                    ready: document.readyState,
                    controlled,
                    terminalStatus: document.querySelector('[data-terminal-status]')?.textContent || null,
                    terminalPaneHidden: document.querySelector('[data-pane="terminal-link"]')?.hidden,
                    settingsHidden: document.querySelector('[data-settings-view]')?.hidden,
                    accountMode: account?.getAttribute("data-mode") || null,
                    accountBusy: account?.getAttribute("aria-busy") || null,
                    accountError: (error?.textContent || "").slice(0, 500) || null,
                };
                "##,
                vec![],
            )
            .await
            .map(|value| value.json().clone())
            .unwrap_or_else(|_| serde_json::json!({ "webdriverError": "diagnostic query failed" }))
    }

    async fn element(driver: &WebDriver, selector: &str) -> Result<WebElement> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(element) => return Ok(element),
                Err(error)
                    if retryable_find_error(error.as_inner())
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => {
                    if retryable_find_error(error.as_inner()) {
                        let state = page_diagnostic_state(driver).await;
                        return Err(error).with_context(|| {
                            format!("timed out waiting for `{selector}`; page={state}")
                        });
                    }
                    return Err(error).with_context(|| format!("failed to find `{selector}`"));
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
            // A row inside a display is clickable before the display has
            // wired its `on:` bindings; the display marks itself
            // `data-bound` once they are live, and the library's own
            // elements wait on that marker before acting. So does this,
            // for a click that has a binding to reach: the display
            // resolves a click through the nearest `on:` ancestor, and
            // one with none to wire never marks itself.
            let bound = driver
                .execute(
                    "const target = arguments[0];
                     // A library element's handlers arrive with its upgrade; the
                     // registry watches each such tag, so one it watches must be
                     // upgraded before a click inside it means anything.
                     for (let node = target; node; node = node.parentElement) {
                       const tag = node.tagName.toLowerCase();
                       if (!tag.includes('-') || !document.querySelector(`tonk-element-watch[data-tag=\"${tag}\"]`)) continue;
                       const definition = customElements.get(tag);
                       if (!definition || !(node instanceof definition)) return false;
                     }
                     let bound = target;
                     while (bound && !bound.getAttributeNames().some((name) => name.startsWith('on:'))) {
                       bound = bound.parentElement;
                     }
                     const display = bound && bound.closest('tonk-display');
                     return !display || display.hasAttribute('data-bound');",
                    vec![found.to_json()?],
                )
                .await?;
            if bound.json().as_bool() != Some(true) {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow!("timed out waiting for `{selector}` to be bound"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            match found.click().await {
                Ok(()) => return Ok(()),
                Err(error)
                    if retryable_element_read_error(error.as_inner())
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => {
                    let action = if retryable_element_read_error(error.as_inner()) {
                        "timed out clicking"
                    } else {
                        "failed to click"
                    };
                    return Err(error).with_context(|| format!("{action} `{selector}`"));
                }
            }
        }
    }

    async fn wait_for_text(driver: &WebDriver, selector: &str, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last: String;
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(found) => match found.text().await {
                    Ok(text) if text == expected => return Ok(()),
                    Ok(text) => last = format!("text was {text:?}"),
                    Err(error) if retryable_element_read_error(error.as_inner()) => {
                        last = error.to_string();
                    }
                    Err(error) => return Err(error).context("failed to read element text"),
                },
                Err(error) if retryable_find_error(error.as_inner()) => {
                    last = error.to_string();
                }
                Err(error) => return Err(error).context("failed to find element for text wait"),
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to equal {expected:?}; last={last}; page={state}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn wait_for_value(driver: &WebDriver, selector: &str, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last: String;
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(found) => match found.prop("value").await {
                    Ok(value) if value.as_deref() == Some(expected) => return Ok(()),
                    Ok(value) => last = format!("value was {value:?}"),
                    Err(error) if retryable_element_read_error(error.as_inner()) => {
                        last = error.to_string();
                    }
                    Err(error) => return Err(error).context("failed to read element value"),
                },
                Err(error) if retryable_find_error(error.as_inner()) => {
                    last = error.to_string();
                }
                Err(error) => return Err(error).context("failed to find element for value wait"),
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` value to equal {expected:?}; last={last}; page={state}"
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
        let mut last: String;
        loop {
            // The text read is fallible for the same reason the find is:
            // a list that re-renders between the two invalidates the
            // handle ("stale element reference"). Treat that as "not yet"
            // and go round again — propagating it aborts the wait on a
            // race the wait exists to absorb.
            match driver.find(By::Css(selector.to_string())).await {
                Ok(found) => match found.text().await {
                    Ok(text) if text.contains(expected) => return Ok(()),
                    Ok(text) => last = format!("text was {text:?}"),
                    Err(error) if retryable_element_read_error(error.as_inner()) => {
                        last = error.to_string();
                    }
                    Err(error) => return Err(error).context("failed to read element text"),
                },
                Err(error) if retryable_find_error(error.as_inner()) => {
                    last = error.to_string();
                }
                Err(error) => return Err(error).context("failed to find element for text wait"),
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to contain {expected:?}; last={last}; page={state}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait until `selector`'s text no longer contains `gone` — the shape
    /// a retraction takes in the DOM.
    async fn wait_for_text_without(driver: &WebDriver, selector: &str, gone: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last: String;
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(found) => match found.text().await {
                    Ok(text) if !text.contains(gone) => return Ok(()),
                    Ok(text) => last = format!("text was {text:?}"),
                    Err(error) if retryable_element_read_error(error.as_inner()) => {
                        last = error.to_string();
                    }
                    Err(error) => return Err(error).context("failed to read element text"),
                },
                Err(error) if retryable_find_error(error.as_inner()) => {
                    last = error.to_string();
                }
                Err(error) => return Err(error).context("failed to find element for text wait"),
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to stop containing {gone:?}; last={last}; page={state}"
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
        // Dressed, not merely present: the chrome's stylesheet is minted
        // after the views mount, and a row hovered or measured before it
        // lands moves when it does. Every carrier, since the page and the
        // stack each declare their own.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let dressed = driver
                .execute(
                    r#"const links = [...document.querySelectorAll('link[data-tonk-embed]')];
                       return links.length > 0 && links.every((link) => link.sheet && link.sheet.cssRules.length);"#,
                    Vec::new(),
                )
                .await?;
            if dressed.json().as_bool() == Some(true) {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the hub's stylesheet never loaded"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait for the view's `with:src` embed to finish resolving.
    ///
    /// `enter_hub` only waits for `.hub-page` to EXIST. Resolving an
    /// embed is two query round-trips that run after the view mounts,
    /// so a read taken the moment the element appears samples an
    /// arbitrary point in that work — usually resolved on a warm
    /// machine, usually not on a cold one. Waiting for the stylesheet
    /// to be loaded rather than for the attribute to be set: the
    /// attribute is set before the browser has fetched the blob, and
    /// what the assertions below read is computed style.
    async fn hub_style_applied(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let ready = driver
                .execute(
                    r#"const link = document.querySelector('link[data-tonk-embed]');
                       return !!(link && link.sheet && link.sheet.cssRules.length);"#,
                    Vec::new(),
                )
                .await
                .ok()
                .and_then(|ret| ret.json().as_bool());
            if ready == Some(true) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let state = driver
                    .execute(
                        r#"const link = document.querySelector('link[data-tonk-embed]');
                           return {
                             carriers: document.querySelectorAll('link[data-tonk-embed]').length,
                             minted: document.querySelectorAll('link[data-tonk-embed-src]').length,
                             href: link ? link.getAttribute('href') : null,
                             sheet: !!(link && link.sheet),
                           };"#,
                        Vec::new(),
                    )
                    .await
                    .map(|ret| ret.json().clone())
                    .unwrap_or(serde_json::Value::Null);
                return Err(anyhow!(
                    "the view's embedded stylesheet never applied; state={state}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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

    /// Enter the profile's frame once its worker answers there.
    ///
    /// The worker's API is the profile's origin's, so a test asks it from
    /// the profile's frame. The frame loads more than once while it starts
    /// its worker, and a script run in a document that is replaced is lost:
    /// this enters again until the frame is one its worker controls, with
    /// its bridge to the page up.
    async fn enter_profile(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let entered: Result<bool> = async {
                enter_guest(driver).await?;
                let ready = driver
                    .execute_async(
                        r#"
                        const done = arguments[arguments.length - 1];
                        (async () => {
                            if (!navigator.serviceWorker.controller || !window.tonk) return done(false);
                            await window.tonk.ready;
                            done(true);
                        })().catch(() => done(false));
                        "#,
                        Vec::new(),
                    )
                    .await?;
                Ok(ready.json().as_bool() == Some(true))
            }
            .await;
            if matches!(entered, Ok(true)) {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the profile's frame never came up: {entered:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
        let mut last: String;
        loop {
            match driver.find(By::Css(selector.to_string())).await {
                Ok(found) => match found.is_displayed().await {
                    Ok(true) => return Ok(found),
                    Ok(false) => last = "element was hidden".to_string(),
                    Err(error) if retryable_element_read_error(error.as_inner()) => {
                        last = error.to_string();
                    }
                    Err(error) => return Err(error).context("failed to read element visibility"),
                },
                Err(error) if retryable_find_error(error.as_inner()) => {
                    last = error.to_string();
                }
                Err(error) => {
                    return Err(error).context("failed to find element for visibility wait");
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to be displayed; last={last}; page={state}"
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
                Err(error) if retryable_find_error(error.as_inner()) => {}
                Err(error) => {
                    return Err(error).context("failed to find elements for absence wait");
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let state = page_diagnostic_state(driver).await;
                return Err(anyhow!(
                    "timed out waiting for `{selector}` to disappear; page={state}"
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

    /// Wait until the deferred account work is done: the custody cell
    /// published and nothing left in the queue, with the profile
    /// reading as registered.
    ///
    /// Call with the dashboard as the current page.
    async fn wait_for_backup_done(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            // The custody-cell publish sits in the deferred queue until
            // activation lets it through; the account is backed up once
            // that entry is gone and the profile reads as registered.
            // The probe is also what notices activation on this device
            // and replays the work it deferred, as the settings page's
            // registration read once did on every load.
            let _ = get_json(driver, "/api/customer").await?;
            let pending = get_json(driver, "/api/customer/pending").await?;
            let publishing = pending["body"]
                .as_array()
                .is_some_and(|queue| queue.iter().any(|work| work["kind"] == "publishCustody"));
            let account = get_json(driver, "/api/account").await?;
            let registered = account["body"]["status"] == "registered";
            if !publishing && registered {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the account backup never settled (pending={pending} account={account})"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Raise the panel that adds an account from the Hub's account trigger.
    ///
    /// The panel is the profile's own and renders in the Hub's frame, so
    /// this returns with the driver in that frame. Only an unlinked
    /// profile's trigger raises it: a linked one opens the account menu
    /// instead.
    async fn raise_cluster_from_hub(driver: &WebDriver, env: &TestEnvironment) -> Result<()> {
        goto(driver, env.tonk_web.as_str()).await?;
        enter_hub(driver).await?;
        wait_for_text_containing(driver, "[data-account-trigger]", "add an account").await?;
        click(driver, "[data-account-trigger]").await?;
        await_register_dialog(driver).await?;
        Ok(())
    }

    /// Open the Hub's settings page and enter the guest that renders it,
    /// waiting for the account facts to land.
    async fn open_hub_settings(driver: &WebDriver, env: &TestEnvironment) -> Result<()> {
        goto(driver, env.tonk_web.join("settings")?.as_str()).await?;
        enter_hub(driver).await?;
        element(driver, "account-settings").await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let email = element(driver, "[data-settings-email]")
                .await?
                .text()
                .await?;
            if !email.is_empty() && !email.contains("loading") {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the settings page never filled its account facts (email reads {email:?})"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait until a profile-changing action has rebuilt the top document.
    async fn wait_for_top_reload(
        driver: &WebDriver,
        before: &serde_json::Value,
        action: &str,
    ) -> Result<()> {
        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if driver
                .execute("return performance.timeOrigin", Vec::new())
                .await
                .is_ok_and(|current| current.json() != before)
            {
                wait_for_service_worker(driver).await?;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for {action} to reload the page"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Content-safe state for locating a stalled command ceremony. This says
    /// whether WebAuthn was still pending, failed visibly, or the card had
    /// already gone; it carries no credential, account, or request values.
    async fn custody_consent_diagnostic(driver: &WebDriver) -> serde_json::Value {
        if driver.enter_default_frame().await.is_err() {
            return serde_json::Value::String("could not enter the top document".to_owned());
        }
        driver
            .execute(
                r##"const card = document.querySelector("#tonk-custody-consent");
                   const actions = card?.querySelector("#tonk-custody-actions");
                   const button = card?.querySelector("#tonk-custody-continue");
                   const rect = button ? button.getBoundingClientRect() : null;
                   return {
                     present: !!card,
                     cards: document.querySelectorAll("#tonk-custody-consent").length,
                     anchored: !!card?.hasAttribute("data-anchored"),
                     visibility: card?.firstElementChild ? getComputedStyle(card.firstElementChild).visibility : null,
                     continueRect: rect ? [rect.left, rect.top, rect.width, rect.height].map(Math.round) : null,
                     message: card?.querySelector("#tonk-custody-text")?.textContent?.trim() || null,
                     awaitingChoice: !!actions,
                   };"##,
                vec![],
            )
            .await
            .map(|result| result.json().clone())
            .unwrap_or_else(|error| {
                serde_json::Value::String(format!("could not read consent state: {error}"))
            })
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
        // WebAuthn is the top page's: the guest's frames are not allowed it.
        driver.enter_default_frame().await?;
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

    async fn credential_ids(driver: &WebDriver, authenticator_id: &str) -> Result<Vec<String>> {
        let result = ChromeDevTools::new(driver.handle.clone())
            .execute_cdp_with_params(
                "WebAuthn.getCredentials",
                serde_json::json!({ "authenticatorId": authenticator_id }),
            )
            .await?;
        result["credentials"]
            .as_array()
            .map(|credentials| {
                credentials
                    .iter()
                    .filter_map(|credential| credential["credentialId"].as_str().map(str::to_owned))
                    .collect()
            })
            .ok_or_else(|| anyhow!("Chrome omitted the virtual authenticator credentials"))
    }

    async fn remove_credential(
        driver: &WebDriver,
        authenticator_id: &str,
        credential_id: &str,
    ) -> Result<()> {
        ChromeDevTools::new(driver.handle.clone())
            .execute_cdp_with_params(
                "WebAuthn.removeCredential",
                serde_json::json!({
                    "authenticatorId": authenticator_id,
                    "credentialId": credential_id,
                }),
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
        raise_cluster_from_hub(driver, env).await?;
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

    /// Put the panel away the way its own control does.
    async fn dismiss_register_dialog(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        driver
            .execute(
                r##"document.querySelector("#tonk-register button.ghost")?.click();"##,
                Vec::new(),
            )
            .await?;
        wait_for_absent(driver, "#tonk-register").await
    }

    /// Create an account for `email` from the raised panel.
    ///
    /// The address decides which ceremony runs: a free one asks for a
    /// display name and then a new passkey. Every caller that signs up
    /// goes through here.
    pub(crate) async fn run_cluster_ceremony(driver: &WebDriver, email: &str) -> Result<()> {
        await_register_dialog(driver).await?;
        type_into_register_dialog(driver, email).await?;
        await_register_action(driver, "create a passkey").await?;
        let before = top_time_origin(driver).await?;
        type_into_settled_row(driver, "display name", "Tab Owner").await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            // Creating an account under an already-account-bound local
            // profile promotes a fresh profile, and the page reloads into
            // it.
            if top_time_origin(driver).await.is_ok_and(|now| now != before) {
                wait_for_service_worker(driver).await?;
                return Ok(());
            }
            // When the tap no longer counts by the time the worker asks, the
            // page asks for one more on its own card, as it would a person.
            driver
                .execute(
                    r##"const go = document.querySelector("#tonk-custody-continue");
                       if (go && go.checkVisibility()) go.click();"##,
                    Vec::new(),
                )
                .await?;
            // The enrollment is a command the worker hands off, so the
            // stage asking for the emailed link is what says it landed. A
            // caller that goes straight to the inbox would otherwise read
            // it before the service had been asked to send anything.
            if enter_guest(driver).await.is_ok()
                && registration_stage(driver)
                    .await
                    .is_ok_and(|stage| stage == "confirming")
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "account creation neither asked for the emailed link nor reloaded; stage {:?}",
                    registration_stage(driver).await.unwrap_or_default()
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The top document's `performance.timeOrigin`, which changes when a
    /// profile switch rebuilds it. Leaves the driver in the top document.
    async fn top_time_origin(driver: &WebDriver) -> Result<serde_json::Value> {
        driver.enter_default_frame().await?;
        Ok(driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone())
    }

    /// Which stage the panel shows, or empty when it is put away. Reads the
    /// frame the driver is in.
    async fn registration_stage(driver: &WebDriver) -> Result<String> {
        Ok(driver
            .execute(
                r##"return document.querySelector("#tonk-register")?.dataset.registration ?? "";"##,
                Vec::new(),
            )
            .await?
            .json()
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// Wait for the panel to show `expected` (empty for put away), in the
    /// guest frame. The guest is entered on every look, since a sign-in can
    /// rebuild the page under it.
    async fn await_registration_stage(driver: &WebDriver, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            let stage = match enter_guest(driver).await {
                Ok(()) => registration_stage(driver).await.ok(),
                Err(_) => None,
            };
            if stage.as_deref() == Some(expected) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the panel never reached {expected:?}; it shows {stage:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Log in to the existing account for `email` from the raised panel.
    ///
    /// The counterpart to [`run_cluster_ceremony`]: the same field, and the
    /// address taken, so going on runs the log-in ceremony.
    pub(crate) async fn run_cluster_login(driver: &WebDriver, email: &str) -> Result<()> {
        await_register_dialog(driver).await?;
        let before = top_time_origin(driver).await?;
        type_into_register_dialog(driver, email).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            if top_time_origin(driver).await.is_ok_and(|now| now != before) {
                wait_for_service_worker(driver).await?;
                return Ok(());
            }
            // A log-in to an account the service serves puts the panel
            // away; one whose address is not yet confirmed waits for the
            // emailed link. Do not accept disappearance alone: the panel
            // also goes when it is cancelled, so the account is the
            // receipt.
            let stage = match enter_guest(driver).await {
                Ok(()) => registration_stage(driver).await.unwrap_or_default(),
                Err(_) => String::from("?"),
            };
            if stage == "failed" {
                let said = await_narrator_containing(driver, "")
                    .await
                    .unwrap_or_default();
                return Err(anyhow!("the log-in failed: {said}"));
            }
            if stage.is_empty()
                && let Ok(account) =
                    tokio::time::timeout(Duration::from_secs(2), get_json(driver, "/api/account"))
                        .await
                && let Ok(account) = account
                && account["status"] == 200
                && account["body"]["accountState"] == "ready"
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "login neither settled nor reloaded after profile routing; stage {stage:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
        enter_guest(driver).await?;
        element(driver, "#activate-accept").await?.click().await?;
        // Displayed, not merely present: the done panel is in the DOM
        // from page load, only hidden, so a presence wait returns while
        // the activation POST is still in flight — and closing the tab
        // then abandons the request and drops the flow's terminal
        // telemetry event. Visible means the response landed.
        wait_for_displayed(driver, "#activate-done").await?;
        driver.close_window().await?;
        driver.switch_to_window(ceremony).await?;
        Ok(())
    }

    async fn await_signup_hub(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            driver.enter_default_frame().await?;
            let url = driver.current_url().await?;
            let stage = match enter_guest(driver).await {
                Ok(()) => registration_stage(driver).await.ok(),
                Err(_) => None,
            };
            if url.path() == "/" && stage.as_deref() == Some("") {
                driver.enter_default_frame().await?;
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "original tab did not finish signup: it is at {url}, its panel shows {stage:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[dialog_common::test]
    async fn it_finishes_signup_in_the_original_tab(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "activation-tab@example.com";
        wait_for_service_worker(&driver).await?;
        raise_cluster_from_hub(&driver, &env).await?;
        type_into_register_dialog(&driver, email).await?;
        await_register_action(&driver, "create a passkey").await?;
        // An empty name cannot create the account or send its email.
        click_register_action(&driver).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(registration_stage(&driver).await?, "naming");
        let inbox: Vec<(String, String)> = reqwest::get(env.access_service.join("_test/emails")?)
            .await?
            .json()
            .await?;
        assert!(
            !inbox.iter().any(|(to, _)| to == email),
            "an empty name must not send verification email"
        );
        type_into_settled_row(&driver, "display name", "Tab Owner").await?;
        await_registration_stage(&driver, "confirming").await?;
        let summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("pending account summary", &summary)["displayName"],
            "Tab Owner"
        );
        // Returning before verification must keep the ceremony waiting.
        driver
            .execute("window.dispatchEvent(new Event('focus'))", Vec::new())
            .await?;
        await_row_value(&driver, "email", "awaiting confirmation").await?;
        let original = driver.window().await?;
        let activation = driver.new_tab().await?;
        driver.switch_to_window(activation).await?;
        goto(&driver, &activation_link(&env, email).await?).await?;
        enter_guest(&driver).await?;
        element(&driver, "#activate-accept").await?.click().await?;
        wait_for_displayed(&driver, "#activate-done").await?;
        assert!(driver.find_all(By::Css("#tonk-register")).await?.is_empty());
        assert_eq!(
            element(&driver, "#activate-done-title")
                .await?
                .text()
                .await?,
            "account verified"
        );
        assert!(
            element(&driver, "#activate-done")
                .await?
                .text()
                .await?
                .contains("You may close this tab")
        );
        assert!(
            driver
                .find_all(By::Css(
                    "#activate-done a, #activate-done button, #activate-done input"
                ))
                .await?
                .is_empty()
        );
        driver.close_window().await?;
        driver.switch_to_window(original).await?;
        await_signup_hub(&driver).await?;
        let summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("account summary", &summary)["displayName"],
            "Tab Owner"
        );
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_does_not_resume_signup_for_another_accounts_activation(
        env: TestEnvironment,
    ) -> Result<()> {
        let owner = driver_with_prf(&env).await?;
        enroll_only(&owner, &env, "link-owner@example.com").await?;
        let other = driver_with_prf(&env).await?;
        enroll_only(&other, &env, "other-account@example.com").await?;
        goto(
            &other,
            &activation_link(&env, "link-owner@example.com").await?,
        )
        .await?;
        enter_guest(&other).await?;
        element(&other, "#activate-accept").await?.click().await?;
        wait_for_displayed(&other, "#activate-done").await?;
        assert!(other.find_all(By::Css("#tonk-register")).await?.is_empty());
        let summary = account_summary(&other).await?;
        let summary = successful_body("other account summary", &summary);
        assert_eq!(summary["email"], "other-account@example.com");
        assert_eq!(summary["displayName"], "Tab Owner");
        owner.quit().await?;
        other.quit().await?;
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
        await_signup_hub(&driver).await?;
        driver.quit().await?;
        Ok(())
    }

    /// A stalled live subscription must not strand an already-verified signup.
    /// Verification happens elsewhere while the original tab stays visible,
    /// with no focus event or page reload to drive completion.
    #[dialog_common::test]
    async fn it_finishes_signup_when_activation_subscription_stalls(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "activation-stream-stalled@example.com";
        enroll_only(&driver, &env, email).await?;
        driver
            .execute(
                r#"const host = document.querySelector('#tonk-register');
               host.reset = host.update = () => {};"#,
                Vec::new(),
            )
            .await?;
        activate_over_http(&env, email).await?;
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("verified customer", &customer)["status"],
            "Active"
        );
        await_signup_hub(&driver).await?;
        let summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("account summary", &summary)["displayName"],
            "Tab Owner"
        );
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
        raise_cluster_from_hub(&device_b, &env).await?;
        type_into_register_dialog(&device_b, email).await?;
        // Refused by the gate, and parked rather than failed.
        await_row_value(&device_b, "email", "awaiting confirmation").await?;

        // Device C.
        activate_over_http(&env, email).await?;

        // Device A's ceremony resolves from its own sweep.
        await_signup_hub(&device_a).await?;
        // Device B's parked login finishes silently: verified, with the
        // passkey row the completed sign-in shows — and nothing asked
        // for a second assertion.
        await_signup_hub(&device_b).await?;

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
        enter_guest(driver).await?;
        element(driver, "#activate-accept").await?.click().await?;
        // Displayed, not merely present: the done panel is in the DOM
        // from page load, only hidden, so a presence wait returns while
        // the activation POST is still in flight — and navigating away
        // then abandons the request and drops the flow's terminal
        // telemetry event. Visible means the response landed.
        wait_for_displayed(driver, "#activate-done").await?;
        // Activation is what unblocks the deferred account work, and the
        // custody-cell publish in that queue is what every later ceremony
        // (unlock, CLI approval, legacy link) resolves. The dashboard
        // publishes it in the background of its load, so returning as
        // soon as the page renders hands a race to whatever the caller
        // does next: navigating away kills the in-flight publish, and a
        // profile rotation orphans it — both of which surfaced in CI as
        // "no account custody is published for this passkey". Stay on
        // the Hub until the queue says the backup settled.
        goto(driver, env.tonk_web.as_str()).await?;
        wait_for_backup_done(driver).await?;
        // Back to where the caller was: activation is a detour, not a
        // navigation the caller asked for.
        goto(driver, account.as_str()).await?;
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
        raise_cluster_from_hub(&second, &env).await?;
        // The address is taken, so going on logs in rather than creates.
        type_into_register_dialog(&second, EMAIL).await?;

        // What this test exists for: a row naming the outstanding step,
        // not a failure. The panel stays up, because the thing it waits
        // on has not happened yet.
        if let Err(error) = await_row_value(&second, "email", "awaiting confirmation").await {
            dump_browser_log(&second, &env).await;
            return Err(error);
        }
        let status = await_narrator_containing(&second, "confirmation link").await?;
        assert!(
            !status.contains("couldn't finish"),
            "and must not report a failure for a wait: {status:?}"
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
        let notice = await_narrator_containing(&driver, "confirmation link").await?;
        assert!(
            !notice.contains("hydration") && !notice.contains("could not be synchronized"),
            "pending setup should not expose account-state implementation terms: {notice:?}"
        );
        assert!(
            !notice.contains("reload /settings"),
            "opening the emailed link should be the only requested next step: {notice:?}"
        );

        // Behind the cluster the account is registered and nothing more:
        // the worker says so, and the emailed link is what changes it.
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("read the customer state", &customer)["status"],
            "Registered",
            "an unconfirmed account is registered, not active"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_fabb_account_tasks_in_space(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let key = create_space(&driver, "Account task space").await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;
        let original = driver.current_url().await?;

        enter_guest(&driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last;
        loop {
            let opened = driver
                .execute(
                    r#"const bar=document.querySelector('tonk-fab');
                       const root=bar?.shadowRoot;
                       const space=root?.querySelector('.space');
                       const account=root?.querySelector('.login');
                       if (!bar || !root || !space || !account || account.hidden) {
                         return {opened:false, bar:!!bar, root:!!root, space:!!space,
                           account:!!account, accountHidden:account?.hidden ?? null};
                       }
                       space.click();
                       account.click();
                       return {opened:true};"#,
                    Vec::new(),
                )
                .await?;
            last = opened.json().clone();
            if last["opened"].as_bool() == Some(true) {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the FABB account entry did not become available: {last}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        // The panel is seated over the bar, in the space's own page.
        await_register_dialog(&driver).await?;
        wait_for_displayed(&driver, "account-task #tonk-register").await?;
        assert_eq!(driver.current_url().await?, original);
        let form = driver
            .execute(
                r#"const host = document.querySelector('account-task #tonk-register');
                   const visible = (selector) => [...host.querySelectorAll(selector)]
                     .find((node) => node.checkVisibility());
                   return {
                     label: host.querySelector('.orow.editing .k')?.textContent?.trim() || '',
                     status: host.querySelector('.oexp p')?.textContent?.trim() || '',
                     dismiss: host.querySelector('button.ghost')?.textContent?.trim() || '',
                     action: visible('#tonk-register-action')?.textContent?.trim() || '',
                   };"#,
                Vec::new(),
            )
            .await?;
        let form = form.json();
        anyhow::ensure!(
            form["label"] == "email address"
                && form["status"]
                    == "Enter your email to continue. We’ll check whether you already have a Tonk account."
                && form["dismiss"] == "cancel"
                && form["action"] == "continue",
            "the contained account form drifted: {form}"
        );

        dismiss_register_dialog(&driver).await?;
        driver.enter_default_frame().await?;
        assert_eq!(driver.current_url().await?, original);
        enter_guest(&driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let restored = driver
                .execute(
                    r#"const bar=document.querySelector('tonk-fab');
                       return !!bar && !bar.hasAttribute('data-task-hosted') &&
                         !bar.hasAttribute('aria-busy');"#,
                    Vec::new(),
                )
                .await?;
            if restored.json() == true {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the FABB did not resume after cancellation"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        driver.enter_default_frame().await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_styles_account_activation_as_a_fabb_ceremony(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let mut activation = env.tonk_web.join("activate")?;
        activation.set_query(Some("ucan=AA"));
        goto(&driver, activation.as_str()).await?;
        enter_guest(&driver).await?;
        element(&driver, "account-activate #activate-confirm").await?;

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
                    const host = document.querySelector('account-activate');
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
                      ceremonyTop: Math.round(ceremony.top),
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
        assert_eq!(desktop["ceremonyWidth"], 576);
        assert_eq!(desktop["ceremonyTop"], 148);
        assert_eq!(desktop["logoWidth"], 132);
        assert_eq!(desktop["actionHeight"], 36);
        assert_eq!(desktop["heading"], "activate your account");
        assert_eq!(desktop["overflow"], false);

        let done = driver
            .execute(
                r#"document.querySelector('#activate-confirm').hidden = true;
                    document.querySelector('#activate-done').hidden = false;
                    const actions = document.querySelectorAll('#activate-done a, #activate-done button');
                    return {
                      heading: document.querySelector('#activate-done-title').textContent.trim(),
                      actions: actions.length
                    };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(done.json()["heading"], "account verified");
        assert_eq!(done.json()["actions"], 0);

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
        // Above that floor the 576px account shell, rather than the requested
        // compact viewport, controls the measurement, so the assertion below
        // would be measuring the wrong rule rather than a broken layout.
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

    /// The hub's chrome travels with its view, not with an injected
    /// stylesheet.
    ///
    /// `tonk-ui/styles.css` used to carry these rules and was handed to
    /// every sealed guest; they are now the `space` view's own
    /// `style: ui`, embedded by `with:src`. So the values asserted here
    /// can ONLY have arrived by the embed resolving and pointing the
    /// view's own `<link rel=stylesheet>` at the minted content — if it
    /// silently does nothing, the link keeps an empty `href`,
    /// `.hub-page` falls back to a transparent background and default
    /// text color, and this fails.
    ///
    /// `/settings` renders the same chrome and embeds the same style
    /// cross-view (`ui@space`), which is the arrangement that keeps one
    /// declaration dressing both pages.
    #[dialog_common::test]
    async fn it_dresses_the_hub_from_the_view_that_declares_its_style(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        hub_style_applied(&driver).await?;
        let hub = driver
            .execute(
                r#"const page = document.querySelector('.hub-page');
                   const style = getComputedStyle(page);
                   const carriers = document.querySelectorAll('link[data-tonk-embed]');
                   return {
                     background: style.backgroundColor,
                     color: style.color,
                     dark: window.matchMedia('(prefers-color-scheme: dark)').matches,
                     carriers: carriers.length,
                     hrefs: [...carriers].map((link) => link.getAttribute('href')),
                     minted: document.querySelectorAll('link[data-tonk-embed-src]').length,
                   };"#,
                Vec::new(),
            )
            .await?;
        let hub = hub.json();
        // The view declares both twins — `--page: #dedbd8` / `--ink:
        // #38182a`, and a `@media (prefers-color-scheme: dark)` pair —
        // so the values to expect depend on the scheme the browser is
        // actually in. Pinning the light literals would fail on a dark
        // runner for a style that arrived perfectly well. Either way a
        // transparent background means the style never arrived at all.
        let (page, ink) = if hub["dark"] == true {
            ("rgb(22, 19, 19)", "rgb(226, 223, 221)")
        } else {
            ("rgb(222, 219, 216)", "rgb(56, 24, 42)")
        };
        assert_eq!(
            hub["background"], page,
            "the hub must wear the page token its view declares; got {hub}",
        );
        assert_eq!(
            hub["color"], ink,
            "the hub must wear the ink token its view declares; got {hub}",
        );
        // The author's own `<link rel=stylesheet>` is what carries the
        // style; the embed pass only fills in its `href`. So a carrier
        // whose href is not a blob URL means the pass ran and gave it
        // nothing, which is the failure the computed values above
        // would also catch but not name.
        assert!(
            hub["carriers"].as_u64().is_some_and(|count| count >= 1),
            "a view declares its `<link with:src>`; got {hub}",
        );
        assert!(
            hub["hrefs"].as_array().is_some_and(|hrefs| {
                hrefs
                    .iter()
                    .all(|href| href.as_str().is_some_and(|href| href.starts_with("blob:")))
            }),
            "the embed pass must point every carrier at minted content; got {hub}",
        );
        assert_eq!(
            hub["minted"], 1,
            "one blob per content, however often the view re-renders; got {hub}",
        );

        // The settings route reads the same declaration cross-view.
        driver.enter_default_frame().await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        enter_hub(&driver).await?;
        hub_style_applied(&driver).await?;
        let settings = driver
            .execute(
                r#"const style = getComputedStyle(document.querySelector('.hub-page'));
                   const carriers = document.querySelectorAll('link[data-tonk-embed]');
                   return {
                     background: style.backgroundColor,
                     carriers: carriers.length,
                     hrefs: [...carriers].map((link) => link.getAttribute('href')),
                     minted: document.querySelectorAll('link[data-tonk-embed-src]').length,
                   };"#,
                Vec::new(),
            )
            .await?;
        let settings = settings.json();
        assert_eq!(
            settings["background"], page,
            "/settings embeds the same style as the hub (`ui@space`); got {settings}",
        );
        assert!(
            settings["hrefs"].as_array().is_some_and(|hrefs| {
                hrefs
                    .iter()
                    .all(|href| href.as_str().is_some_and(|href| href.starts_with("blob:")))
            }),
            "the cross-view embed must resolve to minted content; got {settings}",
        );
        assert_eq!(
            settings["minted"], 1,
            "the cross-view embed mints one blob, not a second copy; got {settings}",
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_renders_the_responsive_hub_collection(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        hub_style_applied(&driver).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let count = driver
                .execute(
                    "return document.querySelectorAll('[data-new-space]').length;",
                    Vec::new(),
                )
                .await?
                .json()
                .as_u64()
                .unwrap_or_default();
            if count == 3 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the Hub never rendered all three create controls; found {count}",
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        driver.set_window_rect(0, 0, 1200, 900).await?;
        let desktop = driver
            .execute(
                r#"const header = document.querySelector('.hub-header').getBoundingClientRect();
                   const main = document.querySelector('.hubcol').getBoundingClientRect();
                   const mobile = getComputedStyle(document.querySelector('.mobile-nav'));
                   const logo = document.querySelector('.hub-logo');
                   const account = document.querySelector('.mobile-nav a[href="/settings"]');
                   return {
                     headerWidth: Math.round(header.width),
                     mainWidth: Math.round(main.width),
                     mobileDisplay: mobile.display,
                     logoHref: logo?.getAttribute('href'),
                     accountHref: account?.getAttribute('href'),
                     newSpaceForms: document.querySelectorAll('[data-new-space]').length,
                     overflow: document.documentElement.scrollWidth > innerWidth,
                   };"#,
                Vec::new(),
            )
            .await?;
        let desktop = desktop.json();
        assert_eq!(desktop["headerWidth"], 1200);
        assert_eq!(desktop["mainWidth"], 1116);
        assert_eq!(desktop["mobileDisplay"], "none");
        assert_eq!(desktop["logoHref"], "/");
        assert_eq!(desktop["accountHref"], "/settings");
        assert_eq!(desktop["newSpaceForms"], 3);
        assert_eq!(desktop["overflow"], false);

        driver.set_window_rect(0, 0, 390, 844).await?;
        let mobile = driver
            .execute(
                r#"const viewport = innerWidth;
                   const nav = document.querySelector('.mobile-nav');
                   const navRect = nav.getBoundingClientRect();
                   const card = document.querySelector('.space-card .srow');
                   const cardStyle = card ? getComputedStyle(card) : null;
                   const visibleNew = [...document.querySelectorAll('[data-new-space]')]
                     .filter(element => element.getClientRects().length > 0);
                   return {
                     viewport,
                     viewportHeight: innerHeight,
                     navDisplay: getComputedStyle(nav).display,
                     navBottom: Math.round(navRect.bottom),
                     navItems: nav.querySelectorAll(':scope > a, :scope > space-create').length,
                     cardColumns: cardStyle?.gridTemplateColumns ?? null,
                     visibleNew: visibleNew.length,
                     overflow: document.documentElement.scrollWidth > innerWidth,
                   };"#,
                Vec::new(),
            )
            .await?;
        let mobile = mobile.json();
        if mobile["viewport"].as_i64().unwrap_or_default() <= 600 {
            assert_eq!(mobile["navDisplay"], "grid");
            assert_eq!(mobile["navBottom"], mobile["viewportHeight"]);
            assert_eq!(mobile["navItems"], 3);
            assert_eq!(mobile["visibleNew"], 1);
            assert_eq!(mobile["overflow"], false);
            if !mobile["cardColumns"].is_null() {
                assert!(
                    mobile["cardColumns"]
                        .as_str()
                        .is_some_and(|columns| columns.starts_with("64px ")),
                    "mobile cards must place a 64px preview beside the caption: {mobile}"
                );
            }
        }

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_hub_card_actions_beside_the_mobile_caption(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        submit_hub_wizard_with(
            &driver,
            "Layout notes",
            "A description that wraps beside the action",
        )
        .await?;
        let key = await_new_space(&driver, &before).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        hub_style_applied(&driver).await?;
        let card = format!(".space-card[data-space-subject='{key}']");
        element(&driver, &card).await?;

        for width in [1200, 500] {
            driver.set_window_rect(0, 0, width, 844).await?;
            // The hub is in a frame of its own process, which takes the
            // window's new size a beat after the window has it: measured
            // before then, it is still laid out for the size before.
            let resized = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                let framed = driver.execute("return innerWidth", Vec::new()).await?;
                if framed
                    .json()
                    .as_u64()
                    .is_some_and(|framed| framed <= width as u64)
                {
                    break;
                }
                anyhow::ensure!(
                    tokio::time::Instant::now() < resized,
                    "the hub's frame never took the window's width of {width}: it is {}",
                    framed.json()
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let layout = driver.execute(
                r#"const card = document.querySelector(arguments[0]);
                   const link = card.querySelector('.srow').getBoundingClientRect();
                   const action = card.querySelector('[data-space-actions-open]').getBoundingClientRect();
                   const preview = card.querySelector('.space-preview').getBoundingClientRect();
                   return {
                     toolbar: !!document.querySelector('.collection-heading, .collection-search'),
                     overflow: document.documentElement.scrollWidth > innerWidth,
                     separate: !card.querySelector('a button'),
                     beside: action.left >= link.right && action.top >= link.top && action.bottom <= link.bottom,
                     previewWidth: Math.round(preview.width),
                   };"#,
                vec![serde_json::json!(card)],
            ).await?;
            let layout = layout.json();
            assert_eq!(layout["toolbar"], false);
            assert_eq!(layout["overflow"], false);
            assert_eq!(layout["separate"], true);
            if width == 500 {
                assert_eq!(layout["beside"], true, "{layout}");
                assert_eq!(layout["previewWidth"], 64);
            }
        }
        element(&driver, &format!("{card} [data-space-actions-open]"))
            .await?
            .click()
            .await?;
        element(&driver, &format!("{card} [data-space-rename-open]"))
            .await?
            .click()
            .await?;
        element(&driver, &format!("{card} [data-space-rename-input]")).await?;
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_bare_join_visits_home(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;

        goto(&driver, env.tonk_web.join("join")?.as_str()).await?;
        await_url_path(&driver, "/").await?;
        enter_hub(&driver).await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_back_into_the_same_account_after_signing_out(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let retained = create_space_awaiting_remote(&driver, "Same Account Draft", false).await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let active_before = successful_body("profiles before sign-out", &profiles)["active"]
            .as_str()
            .context("profiles omitted the active profile")?
            .to_string();
        let root = get_json(&driver, "/api/identity/root").await?;
        let root_before = successful_body("root before sign-out", &root)["rootDid"]
            .as_str()
            .context("root status omitted rootDid")?
            .to_string();
        let devices = get_json(&driver, "/api/account/devices").await?;
        let device_count_before = successful_body("devices before sign-out", &devices)
            .as_array()
            .context("device list was not an array")?
            .len();

        open_hub_settings(&driver, &env).await?;
        click(&driver, "[data-sign-out-open]").await?;
        driver.enter_default_frame().await?;
        let before_sign_out = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        enter_hub(&driver).await?;
        click(&driver, "[data-sign-out-submit]").await?;
        // Signing out rebuilds the page on a rootless local profile so the
        // signed-out account's spaces are preserved but no longer displayed.
        wait_for_top_reload(&driver, &before_sign_out, "sign-out").await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        assert_ne!(
            successful_body("profiles immediately after sign-out", &profiles)["active"],
            active_before,
            "sign-out must leave the retained account profile inactive"
        );
        assert!(
            !space_keys(&driver).await?.contains(&retained),
            "the post-sign-out Hub must not display the account profile's spaces"
        );
        raise_cluster_from_hub(&driver, &env).await?;

        run_cluster_login(&driver, EMAIL).await?;
        // The trigger wears the account's name, not its address; the
        // address is on the settings page.
        let signed_in = async {
            enter_hub(&driver).await?;
            wait_for_text_without(&driver, "[data-account-trigger]", "add an account").await?;
            driver.enter_default_frame().await?;
            open_hub_settings(&driver, &env).await?;
            wait_for_text_containing(&driver, "[data-settings-email]", EMAIL).await?;
            driver.enter_default_frame().await?;
            Ok::<(), anyhow::Error>(())
        };
        if let Err(wait_error) = signed_in.await {
            driver.enter_default_frame().await?;
            let mode = String::from("hub");
            let error = get_json(&driver, "/api/account").await?.to_string();
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
            let answer = get_json(&driver, "/api/account").await;
            eprintln!("PROBE /api/account: {answer:?}");
            let answer = account_summary(&driver).await;
            eprintln!("PROBE account summary: {answer:?}");
            dump_browser_log(&driver, &env).await;
            return Err(wait_error).context(format!(
                "same-account re-login stopped in mode {mode:?}: {error:?}"
            ));
        }

        let summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("account summary after re-login", &summary)["email"],
            EMAIL
        );
        let devices = get_json(&driver, "/api/account/devices").await?;
        let devices = successful_body("device list after re-login", &devices)
            .as_array()
            .context("device list was not an array")?;
        assert_eq!(
            devices.len(),
            device_count_before,
            "re-login must not duplicate the device"
        );
        let profiles = get_json(&driver, "/api/profiles").await?;
        assert_eq!(
            successful_body("profiles after re-login", &profiles)["active"],
            active_before,
            "same-account login must keep the active profile"
        );
        let root = get_json(&driver, "/api/identity/root").await?;
        assert_eq!(
            successful_body("root after re-login", &root)["rootDid"],
            root_before,
            "same-account login must keep its historical root"
        );
        assert!(
            space_keys(&driver).await?.contains(&retained),
            "same-account login must retain the profile's local space catalogue"
        );

        driver.quit().await?;
        Ok(())
    }

    /// An address someone holds goes on to log in, and one nobody holds to
    /// naming a new account. Nothing is minted for the wrong one.
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
        raise_cluster_from_hub(&driver, &env).await?;

        // The taken address logs in...
        type_into_register_dialog(&driver, existing_email).await?;
        await_log_in_started(&driver).await?;
        assert_eq!(
            credential_count(&driver, &authenticator_id).await?,
            0,
            "an address that already has an account must mint nothing",
        );

        // ...and a free one asks for a name, with nothing minted in between.
        dismiss_register_dialog(&driver).await?;
        raise_cluster_from_hub(&driver, &env).await?;
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
        raise_cluster_from_hub(&driver, &env).await?;
        type_into_register_dialog(&driver, "one-action@example.com").await?;
        await_register_action(&driver, "create a passkey").await?;

        // A click and an Enter in one turn: the form goes on twice.
        driver
            .execute(
                r#"const form = document.querySelector('#tonk-register form');
                   form.elements.name.value = 'One Action';
                   document.querySelector('#tonk-register-action').click();
                   form.requestSubmit();"#,
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
    async fn it_offers_to_try_again_after_a_refused_passkey_ceremony(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        wait_for_service_worker(&driver).await?;
        raise_cluster_from_hub(&driver, &env).await?;

        // The ceremony runs in the top page, so that is where it is refused.
        driver.enter_default_frame().await?;
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
        type_into_settled_row(&driver, "display name", "Retry Name").await?;
        // A prompt the browser would not show is offered again on the page's
        // own card; turning that down is what ends the ceremony.
        driver.enter_default_frame().await?;
        wait_for_displayed(&driver, "#tonk-custody-dismiss").await?;
        let calls = driver
            .execute("return window.__registerCreateCalls", Vec::new())
            .await?;
        assert_eq!(calls.json(), &serde_json::json!(1));
        click(&driver, "#tonk-custody-dismiss").await?;
        await_registration_stage(&driver, "failed").await?;
        let said = await_narrator_containing(&driver, "").await?;
        assert!(
            !said.is_empty(),
            "a refused ceremony must say what happened"
        );

        // Trying again goes back to the address, ready to go on.
        await_register_action(&driver, "try again").await?;
        click_register_action(&driver).await?;
        await_registration_stage(&driver, "address").await?;
        await_register_action(&driver, "continue").await?;

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
        enter_guest(&driver).await?;
        // The button is in the view before the element around it is
        // installed, and until then a press reaches nothing.
        element(&driver, "account-activate:defined #activate-accept").await?;

        let count = driver
            .execute_async(
                r#"const done = arguments[arguments.length - 1];
                   const original = window.fetch;
                   let requests = 0;
                   // Activating opens a watch for the worker's answer
                   // before it asks: one that never answers holds the
                   // request pending.
                   window.fetch = (...args) => {
                     const url = String(args[0]?.url || args[0]);
                     if (url.endsWith('/query')) {
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
        let mut stdout_rest = String::new();
        let mut stderr_text = String::new();
        let completion = async {
            let (status, _, _) = tokio::try_join!(
                child.wait(),
                async {
                    loop {
                        let mut line = String::new();
                        if stdout.read_line(&mut line).await? == 0 {
                            break;
                        }
                        stdout_rest.push_str(&line);
                    }
                    Ok::<(), std::io::Error>(())
                },
                async {
                    let mut reader = BufReader::new(stderr);
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await? == 0 {
                            break;
                        }
                        stderr_text.push_str(&line);
                    }
                    Ok::<(), std::io::Error>(())
                },
            )?;
            Ok::<_, std::io::Error>(status)
        };
        match tokio::time::timeout(Duration::from_secs(60), completion).await {
            Ok(result) => {
                let status = result?;
                Ok(CliOutput {
                    status,
                    stdout: format!("{prefix}{stdout_rest}"),
                    stderr: stderr_text,
                })
            }
            Err(_) => {
                child.kill().await?;
                Err(anyhow!(
                    "timed out waiting for CLI completion; stdout={stdout_rest}; stderr={stderr_text}"
                ))
            }
        }
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
        enter_profile(driver).await?;
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
        driver.enter_default_frame().await?;
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
                                    "the": "xyz.tonk.command.check-email/email",
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
            "/api/repository/profile:tonk/branch/main/transact",
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
            "/api/repository/profile:tonk/branch/main/transact",
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
        let endpoint = format!(
            "/api/repository/profile:tonk/branch/{}/query",
            active_branch(driver).await?
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let rows = post_json(
                driver,
                &endpoint,
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
    async fn it_creates_exactly_one_space_from_the_collection_card(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        enter_hub(&driver).await?;
        click(&driver, ".stack > space-create [data-space-create-open]").await?;
        wait_for_displayed(&driver, ".stack > space-create [data-space-create-dialog]").await?;
        element(&driver, ".stack > space-create input[name=name]")
            .await?
            .send_keys("One card creation")
            .await?;
        element(&driver, ".stack > space-create textarea[name=description]")
            .await?
            .send_keys("One submit, one space")
            .await?;
        click(&driver, ".stack > space-create [data-space-create-submit]").await?;
        driver.enter_default_frame().await?;
        await_url_containing(&driver, "/space/").await?;
        // Both handlers finish asynchronously; allow a second creation to surface.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let after = space_keys(&driver).await?;
        let created: Vec<_> = after.iter().filter(|key| !before.contains(key)).collect();
        assert_eq!(created.len(), 1, "one card submit created: {created:?}");
        driver.quit().await?;
        Ok(())
    }

    /// SPACE-14: the Hub creates one independently identified copy.
    #[dialog_common::test]
    async fn it_duplicates_a_space_from_the_hub_menu(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let source = create_space(&driver, "Original space").await?;
        await_url_containing(&driver, &format!("/space/{source}")).await?;
        let before = space_keys(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        let card = format!(".space-card[data-space-subject='{source}']");
        click(&driver, &format!("{card} [data-space-actions-open]")).await?;
        click(&driver, &format!("{card} [data-space-duplicate-open]")).await?;
        let form = format!("{card} [data-space-duplicate]");
        let input = wait_for_displayed(&driver, &format!("{form} input[name=name]")).await?;
        assert_eq!(
            input.value().await?.as_deref(),
            Some("Copy of Original space")
        );
        click(&driver, &format!("{form} [data-space-create-submit]")).await?;
        driver.enter_default_frame().await?;
        await_url_containing(&driver, "/space/").await?;
        let after = space_keys(&driver).await?;
        let created: Vec<_> = after.iter().filter(|key| !before.contains(key)).collect();
        assert_eq!(
            created.len(),
            1,
            "one duplicate submit must create one space"
        );
        assert_ne!(created[0], &source);
        assert!(after.contains(&source), "the original must remain");
        assert!(driver.current_url().await?.path().contains(created[0]));
        driver.quit().await?;
        Ok(())
    }

    /// SPACE-15: Discover copies a bundled template in place, using the same receipt
    /// lifecycle as duplication. Cancellation and a failed seed allocate nothing.
    #[dialog_common::test]
    async fn it_copies_a_remote_template_from_discover(env: TestEnvironment) -> Result<()> {
        let catalog = include_str!("../../tonk-worker/tests/fixtures/discover/catalog.json");
        let base = serve_cross_origin(vec![
            ("/catalog.json", catalog.to_owned()),
            ("/model.yaml", include_str!("../../tonk-worker/tests/fixtures/discover/model.yaml").to_owned()),
            ("/view.yaml", include_str!("../../tonk-worker/tests/fixtures/discover/view.yaml").to_owned()),
            ("/preview.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\"><rect width=\"200\" height=\"100\" fill=\"gray\"/></svg>".to_owned()),
        ])?;
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        enter_hub(&driver).await?;
        driver
            .execute(
                "document.querySelector('hub-discover').setAttribute('catalog-url', arguments[0])",
                vec![serde_json::json!(format!("{base}/missing.json"))],
            )
            .await?;
        click(&driver, "[data-collection=discover]").await?;
        wait_for_displayed(&driver, "[data-discover-retry]").await?;
        driver
            .execute(
                "document.querySelector('hub-discover').setAttribute('catalog-url', arguments[0])",
                vec![serde_json::json!(format!("{base}/catalog.json"))],
            )
            .await?;
        click(&driver, "[data-discover-retry]").await?;
        wait_for_displayed(&driver, "[data-template=remote-demo]").await?;
        click(&driver, "[data-collection=spaces]").await?;
        let collection = driver.find(By::Css(".discover-collection")).await?;
        assert!(
            !collection.is_displayed().await?,
            "templates stay off Your spaces, even when empty"
        );
        click(&driver, "[data-collection=discover]").await?;
        assert!(collection.is_displayed().await?);
        click(&driver, "[data-collection=spaces]").await?;
        assert!(!collection.is_displayed().await?);
        click(&driver, "[data-collection=discover]").await?;
        let card = "[data-template=remote-demo]";
        click(&driver, &format!("{card} [data-template-details-open]")).await?;
        assert!(
            !driver
                .find(By::Css(format!("{card} input[name=name]")))
                .await?
                .is_displayed()
                .await?
        );
        click(&driver, &format!("{card} [data-template-image-open]")).await?;
        wait_for_displayed(&driver, &format!("{card} [data-template-image] img")).await?;
        click(&driver, &format!("{card} [data-template-image-close]")).await?;
        click(&driver, &format!("{card} [data-space-create-open]")).await?;
        let name = wait_for_displayed(&driver, &format!("{card} input[name=name]")).await?;
        assert_eq!(name.value().await?.as_deref(), Some("Remote demo"));
        click(&driver, &format!("{card} [data-template-back]")).await?;
        wait_for_displayed(&driver, &format!("{card} [data-template-image-open]")).await?;
        driver.enter_default_frame().await?;
        assert_eq!(space_keys(&driver).await?, before);
        enter_hub(&driver).await?;
        click(&driver, &format!("{card} [data-space-create-open]")).await?;
        // Refuse a missing asset without creating a partially seeded space.
        driver.execute(
            "document.querySelector('[data-template=remote-demo] input[name=template]').value = arguments[0]",
            vec![serde_json::json!(format!("{base}/missing.json#remote-demo"))],
        ).await?;
        click(&driver, &format!("{card} [data-space-create-submit]")).await?;
        wait_for_text_containing(
            &driver,
            &format!("{card} [data-space-create-error]"),
            "Couldn't use those definitions",
        )
        .await?;
        driver.enter_default_frame().await?;
        assert_eq!(space_keys(&driver).await?, before);
        enter_hub(&driver).await?;
        // Repair the injected failure. A hidden input's value reflects its
        // attribute, so form.reset() cannot undo the test's seed substitution.
        driver.execute(
            "document.querySelector('[data-template=remote-demo] input[name=template]').value = arguments[0]",
            vec![serde_json::json!(format!("{base}/catalog.json#remote-demo"))],
        ).await?;
        click(&driver, &format!("{card} [data-template-back]")).await?;
        wait_for_displayed(&driver, &format!("{card} [data-template-image-open]")).await?;
        click(&driver, &format!("{card} [data-space-create-open]")).await?;
        click(&driver, &format!("{card} [data-space-create-submit]")).await?;
        driver.enter_default_frame().await?;
        await_url_containing(&driver, "/space/").await?;
        let after = space_keys(&driver).await?;
        assert_eq!(
            after.len(),
            before.len() + 1,
            "one template copy creates one space"
        );
        enter_space_view(&driver).await?;
        wait_for_displayed(&driver, ".remote-copy").await?;
        driver.quit().await?;
        Ok(())
    }

    /// Serve `files` (path → body) over plain HTTP on a loopback port,
    /// as some other site would, and return the base URL. Every response
    /// allows any origin to read it: a seed is fetched by the service
    /// worker, cross-origin, and a server that does not say so cannot be
    /// read at all.
    fn serve_cross_origin(files: Vec<(&'static str, String)>) -> Result<String> {
        use std::io::{BufRead as _, BufReader, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}", listener.local_addr()?);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let Ok(clone) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(clone);
                let mut request = String::new();
                let _ = reader.read_line(&mut request);
                let path = request.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let response = match files.iter().find(|(file, _)| *file == path) {
                    Some((_, body)) => format!(
                        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\n\
                         Content-Type: text/plain; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nAccess-Control-Allow-Origin: *\r\n\
                             Content-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned(),
                };
                let mut stream = stream;
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(base)
    }

    /// A `/seed/<url>` link creates a space from the definitions at <url>,
    /// served by another site, and only once the person confirms.
    ///
    /// The seed declares a `/seeded` route whose view template is a file it
    /// `!include`s from beside it, so the page rendering in the new space is
    /// the proof that every hop happened: the link reached the seed page,
    /// its dialog submitted the URL as the seed, the worker fetched it
    /// cross-origin and inlined its include, and the space it created
    /// carries the result. First, a seed that does not exist must say so in
    /// the dialog and create nothing.
    #[dialog_common::test]
    async fn it_creates_a_space_from_a_seed_link(env: TestEnvironment) -> Result<()> {
        let seed = concat!(
            "route!: &route/seeded\n",
            "  this: id:tonk:e2e/route/seeded\n",
            "  path: \"/seeded\"\n",
            "  concept: tonk:e2e/seeded\n",
            "\n",
            "concept!: &e2e/seeded\n",
            "  this: tonk:e2e/seeded\n",
            "  description: A page defined by a seed.\n",
            "  with:\n",
            "    path:\n",
            "      description: The active path, picked off the site.\n",
            "      the: xyz.tonk.site/path\n",
            "      cardinality: one\n",
            "      as: text\n",
            "\n",
            "view!:\n",
            "  this: tonk:e2e/seeded\n",
            "  show:\n",
            "    ui: !include/text ./seeded.html\n",
        );
        let base = serve_cross_origin(vec![
            ("/lib/seed.yaml", seed.to_owned()),
            (
                "/lib/seeded.html",
                "<p class=\"seeded-mark\">seeded from another site</p>\n".to_owned(),
            ),
        ])?;

        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;

        // A seed that is not there: the dialog says so, nothing is created.
        let missing = format!("{base}/lib/missing.yaml");
        driver
            .goto(env.tonk_web.join(&format!("seed/{missing}"))?.as_str())
            .await?;
        enter_guest(&driver).await?;
        // The dialog opens itself on arrival; its field is usable once shown.
        wait_for_displayed(&driver, "space-create[autoopen] input[name=name]")
            .await?
            .send_keys("Not seeded")
            .await?;
        click(&driver, "space-create[autoopen] [data-space-create-submit]").await?;
        wait_for_text_containing(
            &driver,
            "space-create[autoopen] [data-space-create-error]",
            "Couldn't use those definitions",
        )
        .await?;
        let refusal = element(&driver, "space-create[autoopen] [data-space-create-error]")
            .await?
            .text()
            .await?;
        assert!(refusal.contains("404"), "the refusal says why: {refusal}");
        driver.enter_default_frame().await?;
        assert_eq!(
            space_keys(&driver).await?,
            before,
            "a seed that cannot be used must not create a space"
        );

        // The real seed: the page names where it comes from, and asks.
        let source = format!("{base}/lib/seed.yaml");
        driver
            .goto(env.tonk_web.join(&format!("seed/{source}"))?.as_str())
            .await?;
        enter_guest(&driver).await?;
        let name = wait_for_displayed(&driver, "space-create[autoopen] input[name=name]").await?;
        wait_for_text_containing(
            &driver,
            "space-create[autoopen] .space-create-help",
            &source,
        )
        .await?;
        let carried = element(&driver, "space-create[autoopen] input[name=seed]")
            .await?
            .prop("value")
            .await?;
        assert_eq!(
            carried.as_deref(),
            Some(source.as_str()),
            "the dialog must submit the URL it shows"
        );
        name.send_keys("Seeded from a link").await?;
        click(&driver, "space-create[autoopen] [data-space-create-submit]").await?;
        driver.enter_default_frame().await?;
        await_url_containing(&driver, "/space/").await?;
        let key = await_new_space(&driver, &before).await?;

        // The seed's own page, in the new space, rendered from the file it
        // included.
        driver
            .goto(env.tonk_web.join(&format!("space/{key}/seeded"))?.as_str())
            .await?;
        enter_space_view(&driver).await?;
        let mark = wait_for_displayed(&driver, ".seeded-mark").await?;
        assert_eq!(mark.text().await?, "seeded from another site");

        driver.quit().await?;
        Ok(())
    }

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
        submit_hub_wizard_with(
            &driver,
            "Offline notes",
            "Created locally before account registration",
        )
        .await?;

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
        await_url_containing(&driver, &format!("/space/{key}")).await?;

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
        // The bar offers an account gate: nothing is registered.
        // That is the visible half of the account
        // subscription — when its query failed, no frame ever arrived
        // and the bar kept requiring an account even after someone registered.
        await_share_action(&driver, "account").await?;
        open_register_dialog(&driver).await?;
        assert_eq!(register_action_label(&driver).await?, "continue");

        type_into_register_dialog(&driver, "nobody@example.com").await?;
        let label = await_register_action(&driver, "create a passkey").await?;
        assert_eq!(
            label, "create a passkey",
            "an address nobody registered is the create branch",
        );

        // Reading the address must NOT have run a ceremony.
        let typed = credential_count(&driver, &authenticator).await?;
        assert_eq!(
            typed, 0,
            "typing an address must not mint a passkey, got {typed}",
        );

        // Naming the account must actually RUN a ceremony. The command
        // being accepted only means the worker asked the page for WebAuthn,
        // so the credential count is what tells the difference.
        type_into_settled_row(&driver, "display name", "Nobody").await?;
        let after = await_credential_count(&driver, &authenticator, 1).await?;
        assert_eq!(after, 1, "the ceremony mints a passkey");
        await_registration_stage(&driver, "confirming").await?;

        // The share cannot finish until the address is confirmed: the
        // access service refuses to provision a customer that still
        // awaits activation, so minting before this is asking for a
        // refusal, not for a link.
        activate_in_another_tab(&driver, &env, "nobody@example.com").await?;

        // Confirmation reaches the waiting panel as a fact and puts it
        // away, and the bar offers the share it interrupted.
        await_registration_stage(&driver, "").await?;
        await_share_action(&driver, "link").await?;
        click_share_action(&driver, "link").await?;

        // ...and the share finishes, which is the feature: the space gains
        // the remote it refused to share without, and the invite link
        // arrives.
        await_share_link(&driver, &key).await?;

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_agent_invitation_out_of_the_blank_canvas(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        submit_hub_wizard(&driver).await?;
        let key = await_new_space(&driver, &before).await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;
        enter_space_view(&driver).await?;
        wait_for_displayed(&driver, ".blank-canvas").await?;
        let canvas = driver
            .execute(
                r#"return {
                    inviteMounts: document.querySelectorAll('.blank-canvas page-mount').length,
                    deeplinks: document.querySelectorAll('.blank-canvas__deeplink').length,
                    handoffStatuses: document.querySelectorAll('[data-agent-handoff-status]').length
                };"#,
                Vec::new(),
            )
            .await?;
        assert!(
            canvas.json()["inviteMounts"] == 0
                && canvas.json()["deeplinks"] == 0
                && canvas.json()["handoffStatuses"] == 0,
            "the blank canvas must not start an agent invitation: {}",
            canvas.json()
        );
        driver.enter_default_frame().await?;
        await_share_action(&driver, "account").await?;
        enter_guest(&driver).await?;
        let gate = driver
            .execute(
                r#"const root = document.querySelector('tonk-fab')?.shadowRoot;
                   root?.querySelector('.space')?.click();
                   root?.querySelector('.agent')?.click();
                   return {
                     hidden: root?.querySelector('#agent-panel')?.hasAttribute('hidden'),
                     prompt: root?.querySelector('.agent-continue span')?.textContent?.trim()
                   };"#,
                Vec::new(),
            )
            .await?;
        assert!(
            gate.json()["hidden"] == false
                && gate.json()["prompt"] == "add an account to connect an agent",
            "the FABB must offer the account gate for agent invitations: {}",
            gate.json()
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_renames_a_space_from_the_hub(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let before = space_keys(&driver).await?;
        submit_hub_wizard_with(&driver, "Before rename", "A retained description").await?;
        let key = await_new_space(&driver, &before).await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        hub_style_applied(&driver).await?;
        driver.set_window_rect(0, 0, 390, 480).await?;
        let card = format!(".space-card[data-space-subject='{key}']");
        wait_for_displayed(&driver, &format!("{card} [data-space-actions-open]")).await?;
        driver
            .execute(
                "document.querySelector(arguments[0]).scrollIntoView({block: 'center'});",
                vec![serde_json::json!(format!(
                    "{card} [data-space-actions-open]"
                ))],
            )
            .await?;
        click(&driver, &format!("{card} [data-space-actions-open]")).await?;
        wait_for_displayed(&driver, &format!("{card} .space-menu")).await?;
        let menu_bounds = driver
            .execute(
                "const menu = document.querySelector(arguments[0]); \
                       const bounds = menu.getBoundingClientRect(); \
                       const viewport = window.visualViewport; \
                       const nav = document.querySelector('.mobile-nav')?.getBoundingClientRect(); \
                       return {open: menu.getAttribute('open'), left: bounds.left, \
                       right: bounds.right, top: bounds.top, bottom: bounds.bottom, \
                       viewportLeft: viewport?.offsetLeft ?? 0, \
                       viewportRight: (viewport?.offsetLeft ?? 0) + (viewport?.width ?? innerWidth), \
                       viewportTop: viewport?.offsetTop ?? 0, \
                       viewportBottom: Math.min((viewport?.offsetTop ?? 0) + \
                         (viewport?.height ?? innerHeight), nav?.height > 0 ? nav.top : Infinity)};",
                vec![serde_json::json!(format!("{card} .space-menu"))],
            )
            .await?;
        let bounds = menu_bounds.json();
        assert_eq!(
            bounds["open"], "true",
            "the card menu must stay open: {bounds}"
        );
        for (side, edge, direction) in [
            ("left", "viewportLeft", 1.0),
            ("top", "viewportTop", 1.0),
            ("right", "viewportRight", -1.0),
            ("bottom", "viewportBottom", -1.0),
        ] {
            let distance = (bounds[side].as_f64().unwrap_or_default()
                - bounds[edge].as_f64().unwrap_or_default())
                * direction;
            assert!(
                distance >= 7.0,
                "the menu crosses the {side} viewport edge: {bounds}"
            );
        }
        click(&driver, &format!("{card} [data-space-rename-open]")).await?;
        wait_for_displayed(&driver, &format!("{card} [data-space-rename-dialog]")).await?;

        let input = element(&driver, &format!("{card} [data-space-rename-input]")).await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Command + "a"
        } else {
            Key::Control + "a"
        };
        input.send_keys(select_all).await?;
        input.send_keys("After rename").await?;
        click(&driver, &format!("{card} [data-space-rename-submit]")).await?;
        wait_for_text(&driver, &format!("{card} .n"), "After rename").await?;

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
        let remove = format!("space-remove[data-space-subject='{key}']");
        let opener_selector = format!("{remove} [data-space-remove-open]");
        let dialog_selector = format!("{remove} tonk-dialog[data-space-remove-dialog]");
        let submit_selector = format!("{dialog_selector} .m-go");
        let row = wait_for_displayed(&driver, &format!(".srow-wrap:has({remove})")).await?;
        // Park on the row once it has held still: its stylesheet lands a
        // beat after it renders, and a pointer aimed at where the row was
        // hovers nothing once it moves.
        let measure = |driver: &WebDriver, row: &WebElement| {
            let driver = driver.clone();
            let row = row.clone();
            async move {
                driver
                    .execute(
                        "const r = arguments[0].getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map(Math.round);",
                        vec![row.to_json()?],
                    )
                    .await
                    .map(|value| value.json().clone())
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let parked = loop {
            let before = measure(&driver, &row).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let after = measure(&driver, &row).await?;
            if before == after || tokio::time::Instant::now() >= deadline {
                break after;
            }
        };
        driver
            .action_chain()
            .move_to_element_center(&row)
            .perform()
            .await?;
        click(
            &driver,
            &format!(".srow-wrap:has({remove}) [data-space-actions-open]"),
        )
        .await?;
        let opener = match wait_for_displayed(&driver, &opener_selector).await {
            Ok(opener) => opener,
            Err(error) => {
                // What the pointer is over now, against where the row was
                // when the pointer parked on it.
                let hover = driver
                    .execute(
                        "const row = arguments[0]; const r = row.getBoundingClientRect();
                         const at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
                         const verbs = row.querySelector('.verbs');
                         return {
                           connected: row.isConnected, hovered: row.matches(':hover'),
                           rect: [r.left, r.top, r.width, r.height].map(Math.round),
                           under: at ? at.tagName + '.' + at.className : null,
                           underInRow: !!(at && row.contains(at)),
                           verbsOpacity: verbs ? getComputedStyle(verbs).opacity : null,
                           rows: document.querySelectorAll('.srow-wrap').length,
                           scrollY: window.scrollY,
                         };",
                        vec![row.to_json()?],
                    )
                    .await
                    .map(|value| value.json().clone())
                    .unwrap_or(serde_json::Value::Null);
                return Err(error.context(format!(
                    "the remove verb never showed; parked={parked} now={hover}"
                )));
            }
        };
        opener.click().await?;
        wait_for_displayed(&driver, &dialog_selector).await?;

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
                    r#"const dialog = document.querySelector(arguments[0]);
                       const active = document.activeElement;
                       return {
                         open: dialog?.open || false,
                         inside: !!dialog && (active === dialog || dialog.contains(active))
                       };"#,
                    vec![serde_json::json!(dialog_selector)],
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
                     open: document.querySelector(arguments[0])?.open || false,
                     opener: document.activeElement?.matches('[data-space-remove-open]') || false
                   };"#,
                vec![serde_json::json!(dialog_selector)],
            )
            .await?;
        assert_eq!(restored.json()["open"], false);
        assert_eq!(
            restored.json()["opener"],
            true,
            "Escape must restore the remove opener"
        );

        click(&driver, &opener_selector).await?;
        wait_for_displayed(&driver, &dialog_selector).await?;
        let association = driver
            .execute(
                r#"const button = document.querySelector(arguments[0]).querySelector('.m-go');
                   const form = document.querySelector(arguments[0]).querySelector('form[data-remove]');
                   return {
                     attribute: button?.getAttribute('form') || null,
                     associated: button?.form?.id || null,
                     expected: form?.id || null
                   };"#,
                vec![serde_json::json!(dialog_selector)],
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
        click(&driver, &submit_selector).await?;

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
        wait_for_absent(&driver, &remove).await?;
        let mut remaining = space_keys(&driver).await?;
        let mut expected = before;
        remaining.sort();
        expected.sort();
        assert_eq!(remaining, expected, "removal preserves the other spaces");

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
    #[dialog_common::test]
    async fn it_signs_up_to_share_and_hands_over_the_link(env: TestEnvironment) -> Result<()> {
        let (driver, authenticator) = driver_with_prf_authenticator(&env).await?;

        // 1–2. The Hub starts without spaces.
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
        open_space_actions(&driver).await?;
        await_share_action(&driver, "account").await?;

        // 7–8. The panel comes up over the bar.
        click_share_action(&driver, "account").await?;
        await_register_dialog(&driver).await?;

        // 9–10. An address nobody has asks for a name. The stage IS the
        // routing decision, so asserting it covers the whole loop: command
        // dispatched, answer written, subscription delivered.
        let email = "alice@web.mail";
        type_into_register_dialog(&driver, email).await?;
        await_register_action(&driver, "create a passkey").await?;

        // 11–12. Naming it runs the ceremony, which mints a passkey.
        let before = credential_count(&driver, &authenticator).await?;
        type_into_settled_row(&driver, "display name", "Alice").await?;
        await_credential_count(&driver, &authenticator, before + 1).await?;

        // 13–14. And the panel asks for the emailed link.
        await_registration_stage(&driver, "confirming").await?;
        await_narrator_containing(&driver, "confirmation link").await?;

        // 15–17. Open it, accept, and come back, in another tab.
        activate_in_another_tab(&driver, &env, email).await?;

        // 18–22. Confirmation puts the panel away, and the bar offers the
        // share it interrupted; taking it copies the invite.
        let staged: Result<String> = async {
            await_registration_stage(&driver, "").await?;
            await_share_action(&driver, "link").await?;
            watch_guest_clipboard(&driver).await?;
            click_share_action(&driver, "link").await?;
            guest_copied_text(&driver).await
        }
        .await;
        let invite = match staged {
            Ok(invite) => invite,
            Err(error) => {
                dump_browser_log(&driver, &env).await;
                return Err(error);
            }
        };

        // 23–25. It really is an invite: a fresh profile opening it lands
        // in the same space.
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

    /// The Hub offers to add an account, and does it in one step.
    ///
    /// It used to read "log in" and navigate to `/settings`, which put
    /// two surfaces between the label and the ceremony — press it, land
    /// on a panel, press "add an account" there, meet the cluster only
    /// then. It also named the wrong act: the address decides whether it
    /// creates a passkey or signs you in, so half of "log in"'s readers
    /// were told something untrue before they had typed anything.
    ///
    /// Both halves are asserted from the page, because both are what a
    /// person sees: the word on the control, and what one press does.
    #[dialog_common::test]
    async fn it_adds_an_account_from_the_hub_in_one_step(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        // The word on the control, before anything is linked — a plain
        // action, so it must not dress up as a dropdown: no caret ever
        // (the cell is a tab of the hub bar), no menu-button ARIA.
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, "[data-account-trigger]", "add an account").await?;
        let affordance = driver
            .execute(
                r##"
                const trigger = document.querySelector("[data-account-trigger]");
                return {
                    haspopup: trigger ? trigger.getAttribute("aria-haspopup") : "no trigger",
                    caret: !!(trigger && trigger.querySelector(".g")),
                    accountHeight: trigger.getBoundingClientRect().height,
                    newHeight: document.querySelector('.header-new .snew').getBoundingClientRect().height,
                    accountTop: trigger.getBoundingClientRect().top,
                    newTop: document.querySelector('.header-new .snew').getBoundingClientRect().top,
                };
                "##,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            affordance.json()["haspopup"],
            serde_json::Value::Null,
            "the add-an-account trigger is not a menu button",
        );
        assert_eq!(
            affordance.json()["caret"],
            false,
            "the account cell draws no dropdown caret",
        );

        assert_eq!(
            affordance.json()["accountHeight"],
            affordance.json()["newHeight"]
        );
        assert_eq!(affordance.json()["accountTop"], affordance.json()["newTop"]);

        // One press. The cell is the account tab: it pushes `/account`
        // into the same document, and the panel is the Hub's own, so the
        // press must not reload anything.
        // The top document's own clock: the hub frame has one of its own.
        driver.enter_default_frame().await?;
        let before = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        await_register_dialog(&driver).await?;

        let presentation = driver
            .execute(
                r#"const dialog = document.querySelector('#tonk-register');
                   const panel = dialog.querySelector('.ocol').getBoundingClientRect();
                   return {
                     radius: getComputedStyle(dialog.querySelector('.ocol')).borderTopLeftRadius,
                     width: panel.width,
                     anchored: dialog.hasAttribute('data-anchored'),
                     heading: getComputedStyle(dialog.querySelector('.m-head')).display,
                     cancel: getComputedStyle(dialog.querySelector('button.ghost')).display,
                     action: dialog.querySelector('#tonk-register-action').textContent,
                     right: innerWidth - panel.right,
                     top: panel.top,
                     height: panel.height,
                     bottom: innerHeight - panel.bottom,
                   };"#,
                Vec::new(),
            )
            .await?;
        let presentation = presentation.json();
        assert_eq!(presentation["anchored"], true, "{presentation}");
        assert_eq!(presentation["radius"], "25px", "{presentation}");
        assert_eq!(presentation["width"], 360, "{presentation}");
        assert_ne!(presentation["heading"], "none", "{presentation}");
        assert_ne!(presentation["cancel"], "none", "{presentation}");
        assert_eq!(presentation["action"], "continue", "{presentation}");
        assert!(
            presentation["right"].as_f64().unwrap_or(-1.0) >= 8.0,
            "{presentation}"
        );
        assert!(
            presentation["top"].as_f64().unwrap_or(f64::INFINITY) < 200.0,
            "the account form should open below its header action: {presentation}"
        );
        assert!(
            presentation["height"].as_f64().unwrap_or(f64::INFINITY) < 550.0,
            "the email step should be a compact form: {presentation}"
        );
        assert!(
            presentation["bottom"].as_f64().unwrap_or(-1.0) >= 8.0,
            "{presentation}"
        );
        driver.enter_default_frame().await?;
        let landed = driver.current_url().await?;
        assert_eq!(
            landed.path(),
            "/account",
            "the account cell is the account page, got {landed}",
        );
        assert_eq!(
            driver
                .execute("return performance.timeOrigin", Vec::new())
                .await?
                .json(),
            &before,
            "adding an account happens in place, with no reload in between",
        );

        // Finish the ceremony the cluster raised. The Hub is never
        // reloaded from here on, so what the trigger shows next can only
        // come from its live account-name subscription.
        type_into_register_dialog(&driver, "hub-one-step@example.com").await?;
        await_register_action(&driver, "create a passkey").await?;
        type_into_settled_row(&driver, "display name", "Hub Owner").await?;
        await_registration_stage(&driver, "confirming").await?;
        await_narrator_containing(&driver, "confirmation link").await?;

        enter_hub(&driver).await?;
        let background = driver
            .execute(
                r#"return {
                    settingsVisible: !!document.querySelector('[data-settings-view]')?.getClientRects().length,
                    spacesVisible: !!document.querySelector('hub-collection[data-spaces-view]')?.getClientRects().length,
                    noticeVisible: !!document.querySelector('.account-email-notice')?.getClientRects().length,
                };"#,
                Vec::new(),
            )
            .await?;
        assert_eq!(
            background.json()["settingsVisible"],
            false,
            "{background:?}"
        );
        assert_eq!(background.json()["spacesVisible"], true, "{background:?}");
        assert_eq!(background.json()["noticeVisible"], false, "{background:?}");

        // The label flips from the offer to the member's name without a
        // reload, and the cell stays the account tab: no menu grows on it.
        enter_hub(&driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let state = driver
                .execute(
                    r##"
                    const trigger = document.querySelector("[data-account-trigger]");
                    const label = trigger && trigger.querySelector("[data-account-label]");
                    return {
                        label: label ? label.textContent : "",
                        haspopup: trigger ? trigger.getAttribute("aria-haspopup") : null,
                    };
                    "##,
                    Vec::new(),
                )
                .await?;
            let label = state.json()["label"].as_str().unwrap_or("").to_owned();
            if !label.is_empty() && label != "add an account" {
                assert_eq!(
                    state.json()["haspopup"].as_str(),
                    None,
                    "a linked trigger is still the account tab, not a menu button",
                );
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the trigger still reads {label:?}: the account-name subscription \
                 never delivered the linked name",
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        driver.enter_default_frame().await?;
        driver.goto(env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_displayed(&driver, "[data-account-trigger] [data-account-name]").await?;
        click(&driver, "[data-account-trigger]").await?;
        await_url_path(&driver, "/settings").await?;
        enter_hub(&driver).await?;
        wait_for_displayed(&driver, ".details-panel").await?;
        wait_for_displayed(&driver, ".passkeys-panel").await?;
        wait_for_displayed(&driver, "[data-settings-passkey-created]").await?;
        wait_for_displayed(&driver, ".signout-panel").await?;
        #[cfg(feature = "connection-invites")]
        wait_for_displayed(&driver, "[data-agent-connections]").await?;
        let settings = driver
            .execute(
                r#"const details = document.querySelector('.details-panel').getBoundingClientRect();
                   const passkeys = document.querySelector('.passkeys-panel').getBoundingClientRect();
                   const signout = document.querySelector('.signout-panel').getBoundingClientRect();
                   const danger = document.querySelector('.danger-panel').getBoundingClientRect();
                   const email = document.querySelector('[data-settings-email]').getBoundingClientRect();
                   const save = document.querySelector('[data-profile-rename-submit]');
                   return {
                     saveAfterEmail: save.getBoundingClientRect().top >= email.bottom,
                     saveOwnsName: !!save.form?.querySelector('[data-settings-name]'),
                     agentAccess: !!document.querySelector('[data-agent-connections]'),
                     agentAccessVisible: !document.querySelector('[data-agent-connections]')?.hidden,
                     pageBottom: document.querySelector('.hub-page').getBoundingClientRect().bottom,
                     contentBottom: document.querySelector('.hubcol').getBoundingClientRect().bottom,
                     signoutLeft: signout.left,
                     signoutTop: signout.top,
                     signoutRight: signout.right,
                     dangerLeft: danger.left,
                     dangerTop: danger.top,
                     detailsLeft: details.left,
                     title: document.querySelector('.settings-title')?.textContent,
                     switchPanel: !!document.querySelector('.switch-panel'),
                     passkeyDate: document.querySelector('[data-settings-passkey-created]')?.textContent,
                     detailsTop: details.top,
                     detailsRight: details.right,
                     passkeysTop: passkeys.top,
                     passkeysLeft: passkeys.left,
                   };"#,
                Vec::new(),
            )
            .await?;
        let settings = settings.json();
        assert_eq!(settings["title"], "account settings", "{settings}");
        assert_eq!(settings["switchPanel"], false, "{settings}");
        assert_eq!(settings["saveAfterEmail"], true, "{settings}");
        assert_eq!(settings["saveOwnsName"], true, "{settings}");
        assert_eq!(
            settings["signoutLeft"], settings["detailsLeft"],
            "{settings}"
        );
        assert_eq!(settings["agentAccess"], true, "{settings}");
        assert!(
            settings["dangerLeft"].as_f64().unwrap() > settings["signoutRight"].as_f64().unwrap(),
            "delete account should sit beside sign out: {settings}"
        );
        assert_eq!(settings["dangerTop"], settings["signoutTop"], "{settings}");
        assert_eq!(
            settings["agentAccessVisible"],
            cfg!(feature = "connection-invites"),
            "{settings}"
        );
        assert!(
            settings["pageBottom"].as_f64().unwrap()
                >= settings["contentBottom"].as_f64().unwrap() + 119.0,
            "the footer margin escaped the page background: {settings}"
        );
        assert!(
            settings["passkeyDate"]
                .as_str()
                .is_some_and(|date| date.chars().any(char::is_alphabetic)),
            "passkey creation date should be readable rather than raw seconds: {settings}"
        );
        assert!(
            (settings["detailsTop"].as_f64().unwrap_or_default()
                - settings["passkeysTop"].as_f64().unwrap_or_default())
            .abs()
                <= 2.0,
            "details and passkeys should share the first settings row: {settings}"
        );
        assert!(
            settings["passkeysLeft"].as_f64().unwrap_or_default()
                > settings["detailsRight"].as_f64().unwrap_or(f64::INFINITY),
            "details and passkeys should be separate columns: {settings}"
        );

        #[cfg(feature = "connection-invites")]
        {
            driver.enter_default_frame().await?;
            driver.set_window_rect(0, 0, 1200, 1200).await?;
            capture_handoff_page(&driver, "account-grid").await?;
        }

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
            "/api/repository/profile:tonk/branch/main/query",
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
    /// The rendered half of the same subscription. Asserting on the action
    /// the user sees rather than on the fact behind it is what catches a
    /// query that silently answers nothing: the fact was right the whole
    /// time the bar was wrong.
    #[dialog_common::test]
    async fn it_offers_the_copy_action_once_an_account_exists(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        // `create_space` answers with the full DID; prefixing `did:key:`
        // again names nothing, and the bar then has no space to answer
        // about.
        let key = create_space(&driver, "Shareable").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/{key}"))?.as_str())
            .await?;
        await_share_action(&driver, "account").await?;

        sign_up(&driver, &env, "bar-flips@example.com").await?;
        driver
            .goto(env.tonk_web.join(&format!("space/{key}"))?.as_str())
            .await?;

        await_share_action(&driver, "link").await?;

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
            "/api/repository/profile:tonk/branch/main/transact",
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
                "/api/repository/profile:tonk/branch/main/transact",
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

    /// An address that already has an account must SIGN IN.
    ///
    /// Sending someone with an account through a creation ceremony
    /// leaves an orphan passkey in their authenticator and fails at the
    /// end, so going on has to route on the answer rather than assume.
    #[dialog_common::test]
    async fn it_offers_sign_in_for_an_address_that_already_has_an_account(
        env: TestEnvironment,
    ) -> Result<()> {
        // Register the address in a profile of its own, then ask about
        // it from a fresh one. The share action that raises the panel is
        // only offered while THIS browser has no account.
        let owner = driver_with_prf(&env).await?;
        let taken = "taken@example.com";
        sign_up(&owner, &env, taken).await?;
        owner.quit().await?;

        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;

        open_register_dialog_from_a_space(&driver, &env, "Signed In").await?;
        type_into_register_dialog(&driver, taken).await?;
        await_log_in_started(&driver).await?;

        driver.quit().await?;
        Ok(())
    }

    /// Wait for the panel to go on to log in, never to naming a new
    /// account. Reads the guest frame.
    async fn await_log_in_started(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let at = driver
                .execute(
                    r##"const panel = document.querySelector("#tonk-register");
                       return panel ? `${panel.dataset.registration}:${panel.dataset.kind ?? ""}` : "";"##,
                    Vec::new(),
                )
                .await?;
            let at = at.json().as_str().unwrap_or_default().to_owned();
            anyhow::ensure!(
                !at.starts_with("naming"),
                "a registered address must log in, not sign up a second time"
            );
            if at == "ceremony:log-in" || at == "failed:log-in" {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the address never went on to log in; the panel is at {at:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Mobile WebAuthn providers require the assertion to begin while a
    /// person's tap still counts. The tap is in the guest and the passkey
    /// is asked for in the top page after the worker routes the address:
    /// the page asks at once while that tap counts, and otherwise on its own
    /// card's tap. Either way `credentials.get` starts under a live tap.
    #[dialog_common::test]
    async fn it_starts_login_passkey_before_the_activating_click_returns(
        env: TestEnvironment,
    ) -> Result<()> {
        let owner = driver_with_prf(&env).await?;
        let taken = "tap-bound@example.com";
        sign_up(&owner, &env, taken).await?;
        owner.quit().await?;

        let driver = driver_with_prf(&env).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        open_register_dialog_from_a_space(&driver, &env, "Tap Bound").await?;

        driver.enter_default_frame().await?;
        driver
            .execute(
                r##"window.__tonkGetActive = null;
                   Object.defineProperty(navigator.credentials, "get", {
                     configurable: true,
                     value: () => {
                       window.__tonkGetActive = navigator.userActivation.isActive;
                       return Promise.reject(new DOMException(
                         "controlled passkey rejection", "NotAllowedError"
                       ));
                     }
                   });"##,
                Vec::new(),
            )
            .await?;
        // A script the driver runs carries a user gesture, as a tap does.
        type_into_register_dialog(&driver, taken).await?;

        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut tapped = false;
        let started_in_click = loop {
            let seen = driver
                .execute(
                    r##"const go = document.querySelector("#tonk-custody-continue");
                       return { active: window.__tonkGetActive, card: !!go && go.checkVisibility() };"##,
                    Vec::new(),
                )
                .await?;
            let seen = seen.json().clone();
            if !seen["active"].is_null() {
                break seen["active"].clone();
            }
            if seen["card"] == true && !tapped {
                tapped = true;
                element(&driver, "#tonk-custody-continue")
                    .await?
                    .click()
                    .await?;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the page was never asked for the passkey"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(
            started_in_click,
            serde_json::Value::Bool(true),
            "credentials.get must start while the activating tap still counts"
        );

        driver.quit().await?;
        Ok(())
    }

    /// A passkey login from a fresh browser finishes only once the account's
    /// portable name is available, then returns straight to the complete Hub.
    /// Login leaves the ceremony automatically, without a return-action click.
    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn it_returns_a_new_browser_login_to_the_synced_hub(env: TestEnvironment) -> Result<()> {
        const EMAIL: &str = "return-to-hub@example.com";
        const NAME: &str = "Jack";

        let (owner, authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&owner, &env, EMAIL).await?;
        let renamed = post_json(
            &owner,
            "/api/account/display-name",
            serde_json::json!({ "name": NAME }),
        )
        .await?;
        assert_eq!(successful_body("name the account", &renamed)["name"], NAME);
        successful_body(
            "publish the account name",
            &post_json(&owner, "/api/sync", serde_json::json!({})).await?,
        );

        let (second, _second_authenticator) =
            second_device_with_same_passkey(&env, &owner, &authenticator).await?;
        wait_for_service_worker(&second).await?;
        raise_cluster_from_hub(&second, &env).await?;
        type_into_register_dialog(&second, EMAIL).await?;
        // Login must put the panel away on its own.
        await_registration_stage(&second, "").await?;
        wait_for_service_worker(&second).await?;
        await_url_path(&second, "/").await?;
        await_account_name(&second, NAME).await?;

        enter_hub(&second).await?;
        assert!(
            element(&second, "[data-spaces-view]")
                .await?
                .is_displayed()
                .await?,
            "return to hub must restore the spaces stack without a second click"
        );
        assert_eq!(
            element(&second, "[data-return-spaces]")
                .await?
                .attr("aria-current")
                .await?
                .as_deref(),
            Some("page")
        );
        wait_for_text(&second, "[data-account-label]", NAME).await?;

        owner.quit().await?;
        second.quit().await?;
        Ok(())
    }

    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn profile_library_repairs_claims_from_an_old_account_writer(
        env: TestEnvironment,
    ) -> Result<()> {
        const EMAIL: &str = "profile-library-upgrade@example.com";
        const NAME: &str = "Profile Library Owner";
        const SPACE: &str = "Profile Library Sentinel";

        let (generation_a, generation_b) =
            crate::service_worker_upgrade::tests::prepare_profile_library_generations(&env)?;
        let (owner, authenticator) = driver_with_prf_authenticator(&env).await?;
        crate::service_worker_upgrade::tests::wait_for_complete_generation(
            &owner,
            &generation_a,
            None,
            None,
        )
        .await?;
        sign_up(&owner, &env, EMAIL).await?;
        successful_body(
            "name the account",
            &post_json(
                &owner,
                "/api/account/display-name",
                serde_json::json!({ "name": NAME }),
            )
            .await?,
        );
        let _space = create_space(&owner, SPACE).await?;
        successful_body(
            "publish the populated historical profile",
            &post_json(&owner, "/api/sync", serde_json::json!({})).await?,
        );
        goto(&owner, env.tonk_web.as_str()).await?;
        enter_hub(&owner)
            .await
            .context("old-writer scenario: mount owner Hub")?;
        wait_for_text_containing(&owner, "body", "no spaces yet").await?;
        wait_for_text_containing(&owner, "body", SPACE).await?;
        owner.enter_default_frame().await?;

        let (old_writer, _old_authenticator) =
            second_device_with_same_passkey(&env, &owner, &authenticator).await?;
        crate::service_worker_upgrade::tests::wait_for_complete_generation(
            &old_writer,
            &generation_a,
            None,
            None,
        )
        .await?;
        // This scenario needs a genuinely historical writer even after the
        // other device upgrades. Pin only this browser's deployment assets,
        // on the app's origin and on the profile's, whose worker is the
        // writer; account traffic and the real worker lifecycle remain
        // untouched.
        old_writer.enter_default_frame().await?;
        old_writer
            .add_cookie(Cookie::new("tonk-test-generation", "a"))
            .await?;
        enter_profile(&old_writer).await?;
        old_writer
            .add_cookie(Cookie::new("tonk-test-generation", "a"))
            .await?;
        old_writer.enter_default_frame().await?;
        raise_cluster_from_hub(&old_writer, &env).await?;
        run_cluster_login(&old_writer, EMAIL).await?;
        crate::service_worker_upgrade::tests::wait_for_site_generation(&old_writer, &generation_a)
            .await
            .context("the competing writer did not remain on generation A")?;

        crate::service_worker_upgrade::tests::promote_second_generation(&env)?;
        owner.enter_default_frame().await?;
        owner.refresh().await?;
        crate::service_worker_upgrade::tests::wait_for_complete_generation(
            &owner,
            &generation_b,
            None,
            Some(&generation_a.build),
        )
        .await?;
        goto(&owner, env.tonk_web.as_str()).await?;
        enter_hub(&owner)
            .await
            .context("old-writer scenario: mount owner Hub")?;
        wait_for_text_without(&owner, "body", "no spaces yet").await?;
        wait_for_text_containing(&owner, "body", SPACE).await?;
        owner.enter_default_frame().await?;

        successful_body(
            "publish stale profile claims from generation A",
            &post_json(&old_writer, "/api/sync", serde_json::json!({})).await?,
        );
        crate::service_worker_upgrade::tests::wait_for_site_generation(&old_writer, &generation_a)
            .await
            .context("the competing writer left generation A after the deploy")?;
        goto(&old_writer, env.tonk_web.as_str()).await?;
        enter_hub(&old_writer)
            .await
            .context("old-writer scenario: mount historical writer Hub after current deployment")?;
        wait_for_text_containing(&old_writer, "body", "no spaces yet").await?;
        wait_for_text_containing(&old_writer, "body", SPACE).await?;
        old_writer.enter_default_frame().await?;
        successful_body(
            "repair stale profile claims on generation B",
            &post_json(&owner, "/api/sync", serde_json::json!({})).await?,
        );
        goto(&owner, env.tonk_web.as_str()).await?;
        enter_hub(&owner)
            .await
            .context("old-writer scenario: mount owner Hub")?;
        wait_for_text_without(&owner, "body", "no spaces yet").await?;
        wait_for_text_containing(&owner, "body", SPACE).await?;
        owner.enter_default_frame().await?;
        await_account_name(&owner, NAME).await?;

        old_writer.quit().await?;
        successful_body(
            "settle the first unchanged current sweep",
            &post_json(&owner, "/api/sync", serde_json::json!({})).await?,
        );
        successful_body(
            "settle the second unchanged current sweep",
            &post_json(&owner, "/api/sync", serde_json::json!({})).await?,
        );
        owner.quit().await?;
        Ok(())
    }

    /// The tap-bound assertion hands clone-safe PRF bytes to the worker,
    /// never `CryptoKey` handles. Keep the real virtual-authenticator
    /// ceremony and intercept only the final service-worker post.
    #[cfg(feature = "integration-tests")]
    #[dialog_common::test]
    async fn it_posts_prf_bytes_after_the_tap_bound_assertion(env: TestEnvironment) -> Result<()> {
        const EMAIL: &str = "byte-handoff@example.com";

        let (owner, authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&owner, &env, EMAIL).await?;
        let (driver, _authenticator) =
            second_device_with_same_passkey(&env, &owner, &authenticator).await?;
        owner.quit().await?;

        wait_for_service_worker(&driver).await?;
        raise_cluster_from_hub(&driver, &env).await?;

        // The ceremony runs in the top page, which hands what it derived
        // to the profile's worker through the profile's frame.
        driver.enter_default_frame().await?;
        driver
            .execute(
                r#"
                window.__tonkCustodyHandoff = null;
                const original = window.tonkProfileWorker;
                window.tonkProfileWorker = function(message, transfer) {
                    if (message && message.type === "custody") {
                        const port = transfer && transfer[0];
                        window.__tonkCustodyHandoff = {
                            keyIsBytes: message.key instanceof Uint8Array,
                            kekIsBytes: message.kek instanceof Uint8Array,
                            keyLength: message.key && message.key.length,
                            kekLength: message.kek && message.kek.length,
                            keyIsCryptoKey: typeof CryptoKey !== "undefined"
                                && message.key instanceof CryptoKey,
                            kekIsCryptoKey: typeof CryptoKey !== "undefined"
                                && message.kek instanceof CryptoKey,
                            replyPortPresent: port instanceof MessagePort,
                            transferCount: transfer ? transfer.length : 0,
                        };
                        port.postMessage({ ok: { credentialId: message.credentialId } });
                        return;
                    }
                    return original.apply(this, arguments);
                };
                "#,
                Vec::new(),
            )
            .await?;

        type_into_register_dialog(&driver, EMAIL).await?;
        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let observed = loop {
            let observed = driver
                .execute("return window.__tonkCustodyHandoff;", Vec::new())
                .await?;
            if !observed.json().is_null() {
                break observed.json().clone();
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the real passkey assertion never posted a custody handoff"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        assert_eq!(observed["keyIsBytes"], true);
        assert_eq!(observed["kekIsBytes"], true);
        assert_eq!(observed["keyLength"], 32);
        assert_eq!(observed["kekLength"], 32);
        assert_eq!(observed["keyIsCryptoKey"], false);
        assert_eq!(observed["kekIsCryptoKey"], false);
        assert_eq!(observed["replyPortPresent"], true);
        assert_eq!(observed["transferCount"], 1);

        driver.quit().await?;
        Ok(())
    }

    /// Wait for the top document to land on `path`, whatever the query.
    /// Wait for the account's chosen name to replicate.
    ///
    /// Login now ends at CUSTODY RECOVERY: the account's own facts arrive
    /// behind it, and the Hub's account cell subscribes to the name and
    /// holds a skeleton until it lands. Asserting the name the instant
    /// the ceremony leaves tests the old contract, where login blocked
    /// until the whole account had hydrated.
    async fn await_account_name(driver: &WebDriver, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let summary = account_summary(driver).await?;
            if summary["body"]["displayName"] == expected {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the account name never replicated; last read {summary}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn await_url_path(driver: &WebDriver, path: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            driver.enter_default_frame().await?;
            let url = driver.current_url().await?;
            if url.path() == path {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the page never landed on {path}; it is at {url}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
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
                let panel = driver
                    .execute(
                        r##"const panel = document.querySelector("#tonk-register");
                           return {
                               stage: panel?.dataset.registration ?? null,
                               kind: panel?.dataset.kind ?? null,
                               text: panel?.innerText.replace(/\s+/g, " ").trim().slice(0, 400) ?? null,
                           };"##,
                        Vec::new(),
                    )
                    .await
                    .map(|value| value.json().to_string())
                    .unwrap_or_else(|error| format!("panel unreadable: {error}"));
                return Err(anyhow!(
                    "the {noun:?} row never reached {expected:?}; it reads {last:?}; panel={panel}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Read a settled row's value by its noun.
    ///
    /// A settled row is a later stage's record of an earlier answer
    /// (`email  ada@example.com`): it holds text, not an input.
    async fn await_settled_row(driver: &WebDriver, noun: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut last;
        enter_guest(driver).await?;
        loop {
            let value = driver
                .execute(
                    r##"
                    const noun = arguments[0];
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

    /// Type into the row named `noun` and go on, the way Enter does.
    async fn type_into_settled_row(driver: &WebDriver, noun: &str, value: &str) -> Result<()> {
        // The row is the next stage's, which renders a beat after the one
        // before it, so an absent row is waited out rather than failed on
        // the first look.
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const [noun, value] = [arguments[0], arguments[1]];
                    for (const row of document.querySelectorAll("#tonk-register .orow")) {
                        if (row.querySelector(".k")?.textContent.trim() !== noun) continue;
                        const input = row.querySelector("input");
                        if (!input) return { error: noun + " row takes no input" };
                        if (!input.closest("tonk-display")?.hasAttribute("data-bound")) {
                            return { error: "no row named " + noun + " listening yet" };
                        }
                        input.focus();
                        input.value = value;
                        input.dispatchEvent(new Event("input", { bubbles: true }));
                        input.form.requestSubmit();
                        return { ok: true };
                    }
                    return { error: "no row named " + noun };
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

    /// Wait for the panel to say something containing `fragment`.
    async fn await_narrator_containing(driver: &WebDriver, fragment: &str) -> Result<String> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        let mut last;
        loop {
            let text = driver
                .execute(
                    r##"const p = document.querySelector("#tonk-register .oexp p");
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
                    "the panel never said {fragment:?}; it reads {last:?}",
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

    #[cfg(feature = "connection-invites")]
    async fn click_agent_connection_action(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let opened = driver.execute(
                r#"const bar = document.querySelector('tonk-fab');
                   const agent = bar?.querySelector('tonk-agent-panel');
                   const root = bar?.shadowRoot;
                   if (!agent || typeof agent.__tonkReset !== 'function' ||
                       bar.hasAttribute('data-account-required') ||
                       !root?.querySelector('.space') || !root?.querySelector('.agent')) return false;
                   root.querySelector('.space').click();
                   root.querySelector('.agent').click();
                   return !root.querySelector('#agent-panel').hidden;"#,
                Vec::new(),
            ).await?;
            if opened.json() == true {
                driver.enter_default_frame().await?;
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the bar never offered its agent connection action"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[cfg(feature = "connection-invites")]
    async fn await_agent_connection_ready(driver: &WebDriver, space: &str) -> Result<()> {
        await_agent_connection_ready_after(driver, space, None).await
    }

    #[cfg(feature = "connection-invites")]
    async fn await_agent_connection_ready_after(
        driver: &WebDriver,
        space: &str,
        previous_link: Option<&str>,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            enter_guest(driver).await?;
            let state = driver
                .execute(
                    r#"const bar = document.querySelector('tonk-fab');
                   const root = bar?.shadowRoot;
                   const panel = root?.querySelector('#agent-panel');
                   const prompt = panel?.querySelector('.panel-copytext')?.textContent || '';
                   return {
                     visible: !!panel && !panel.hidden,
                     space: bar?.querySelector('tonk-agent-panel')?.getAttribute('space'),
                     ready: ['.agent-copy-link', '.agent-copy-prompt'].every(selector => {
                       const button = panel?.querySelector(selector);
                       return !!button && !button.hidden && !button.disabled;
                     }),
                     scoped: prompt.includes('#tonk-agent-v2='),
                     fresh: !arguments[0] || !prompt.includes(arguments[0]),
                     status: panel?.querySelector('.agent-status')?.textContent
                   };"#,
                    vec![serde_json::json!(previous_link)],
                )
                .await?;
            driver.enter_default_frame().await?;
            let last = state.json();
            if last["visible"] == true
                && last["space"] == space
                && last["ready"] == true
                && last["scoped"] == true
                && last["fresh"] == true
            {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "agent connection did not become ready for {space}: {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[cfg(feature = "connection-invites")]
    async fn copy_agent_connection_text(driver: &WebDriver, selector: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            enter_guest(driver).await?;
            watch_clipboard(driver).await?;
            // Readiness can change after the invitation wait while the guest
            // reconciles its panel. Check the current control and click it in
            // the same script, without carrying a stale ready state forward.
            let clicked = driver
                .execute(
                    r#"const root = document.querySelector('tonk-fab')?.shadowRoot;
                       const panel = root?.querySelector('#agent-panel');
                       const button = root?.querySelector(arguments[0]);
                       const state = {
                         visible: !!panel && !panel.hidden,
                         present: !!button,
                         hidden: button?.hidden ?? null,
                         disabled: button?.disabled ?? null,
                         clicked: false
                       };
                       if (state.visible && button && !button.hidden && !button.disabled) {
                         button.click();
                         state.clicked = true;
                       }
                       return state;"#,
                    vec![serde_json::json!(selector)],
                )
                .await?;
            if clicked.json()["clicked"] == true {
                let text = copied_text(driver).await?;
                driver.enter_default_frame().await?;
                return Ok(text);
            }
            driver.enter_default_frame().await?;
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "agent copy control did not become ready: {selector}: {}",
                clicked.json()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[cfg(feature = "connection-invites")]
    async fn copy_agent_connection_prompt(driver: &WebDriver) -> Result<String> {
        copy_agent_connection_text(driver, "#agent-panel .agent-copy-prompt").await
    }

    #[cfg(feature = "connection-invites")]
    async fn copy_agent_connection_link(driver: &WebDriver) -> Result<String> {
        let link = copy_agent_connection_text(driver, "#agent-panel .agent-copy-link").await?;
        let url = url::Url::parse(&link).context("copied agent link is not a URL")?;
        anyhow::ensure!(
            url.path() == "/agent/"
                && url
                    .fragment()
                    .is_some_and(|fragment| fragment.starts_with("tonk-agent-v2=")),
            "copied agent link has no scoped invitation"
        );
        Ok(link)
    }

    /// The panel's action label, or empty when it has none.
    async fn register_action_label(driver: &WebDriver) -> Result<String> {
        enter_guest(driver).await?;
        let label = driver
            .execute(
                r##"const a = [...document.querySelectorAll("#tonk-register-action")]
                       .find((action) => action.checkVisibility());
                   return a ? (a.textContent || "").trim() : "";"##,
                Vec::new(),
            )
            .await?;
        Ok(label.json().as_str().unwrap_or_default().to_owned())
    }

    /// Click the panel's action.
    async fn click_register_action(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let outcome = driver
            .execute(
                r##"const a = [...document.querySelectorAll("#tonk-register-action")]
                       .find((action) => action.checkVisibility());
                   if (!a) return { error: "no action" };
                   a.click();
                   return { ok: true };"##,
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
    /// Read from PROFILE main, keyed by the space. The row moved off the
    /// space's branch: the Hub renders one share control per row, and a
    /// control subscribed to the space made merely listing spaces query
    /// into each one — which mounts it, so opening the Hub replicated the
    /// whole account. The dialog reads the same row to fill the
    /// clipboard, so this is the row the person ends up with, not a proxy
    /// for it.
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
        let endpoint = format!(
            "/api/repository/profile:tonk/branch/{}/query",
            active_branch(driver).await?
        );
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

    /// Open the bar's space actions before choosing a share action.
    async fn open_space_actions(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const bar = document.querySelector("tonk-fab");
                    if (!bar || !bar.shadowRoot) return false;
                    const cell = bar.shadowRoot.querySelector('.space');
                    if (!cell) return false;
                    if (bar.shadowRoot.querySelector('.run')?.hasAttribute('hidden')) cell.click();
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
                return Err(anyhow!("the bar never showed its space actions"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Take the account gate or the ready copy action in the FABB shadow root.
    async fn click_share_action(driver: &WebDriver, action: &str) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const action = arguments[0];
                    const bar = document.querySelector("tonk-fab");
                    const root = bar?.shadowRoot;
                    const share = root?.querySelector('.share');
                    if (!share) return { error: "no share action" };
                    const accountRequired = bar.hasAttribute('data-account-required');
                    if (action === 'account' && !accountRequired) return { error: 'account already ready' };
                    if (action === 'link' && accountRequired) return { error: 'account still required' };
                    if (root.querySelector('.run')?.hasAttribute('hidden')) root.querySelector('.space').click();
                    share.click();
                    if (action === 'account') {
                        const gate = root.querySelector('#share-panel');
                        const continueButton = root.querySelector('.share-continue');
                        if (!gate || gate.hasAttribute('hidden') || !continueButton) {
                            return { error: 'account gate did not open' };
                        }
                        continueButton.click();
                    }
                    return { ok: true };
                    "##,
                    vec![serde_json::json!(action)],
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
                return Err(anyhow!("could not click the share action: {reason}"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Whether the FABB share action currently opens an account gate or copies.
    async fn share_action_offered(driver: &WebDriver) -> Result<Option<String>> {
        enter_guest(driver).await?;
        let outcome = driver
            .execute(
                r##"
                const bar = document.querySelector("tonk-fab");
                if (!bar) return null;
                if (!bar.shadowRoot?.querySelector('.share') || bar.hasAttribute('data-unknown-space')) return null;
                return bar.hasAttribute('data-account-required') ? "account" : "link";
                "##,
                Vec::new(),
            )
            .await?;
        driver.enter_default_frame().await?;
        Ok(outcome.json().as_str().map(str::to_owned))
    }

    /// Read the copy action's state from its visible shadow-root button.
    async fn share_action_state(driver: &WebDriver) -> Result<Option<String>> {
        enter_guest(driver).await?;
        let outcome = driver
            .execute(
                r##"
                const bar = document.querySelector("tonk-fab");
                return bar?.shadowRoot?.querySelector('.share')?.getAttribute('data-share-state') ?? null;
                "##,
                Vec::new(),
            )
            .await?;
        driver.enter_default_frame().await?;
        Ok(outcome.json().as_str().map(str::to_owned))
    }

    /// Record what the FABB's share control hands the clipboard.
    ///
    /// Two reasons this is not [`watch_clipboard`]. It patches `write`, not
    /// `writeText`: `<tonk-share>` opens a `ClipboardItem` holding a PROMISE
    /// while the user activation is still live and resolves it when the mint
    /// returns, which is the only way to copy the result of an async
    /// operation. And it runs inside the sealed guest, because that is the
    /// document the bar lives in.
    ///
    /// Reading the hook rather than the real clipboard is not a shortcut
    /// around a permission. The write itself is refused here with
    /// `NotAllowedError: Document is not focused` — a headless window is
    /// never focused — so the control settles on `failed` in this harness
    /// however well it works in a real browser. What the control HANDED the
    /// clipboard is the product behaviour under test; whether this particular
    /// Chrome accepted it is the harness's business.
    async fn watch_guest_clipboard(driver: &WebDriver) -> Result<()> {
        enter_guest(driver).await?;
        driver
            .execute(
                r##"
                window.__tonkWrote = "";
                const clipboard = navigator.clipboard;
                const write = clipboard.write.bind(clipboard);
                clipboard.write = (items) => {
                    const item = items && items[0];
                    if (item && item.getType) {
                        item.getType("text/plain")
                            .then((blob) => blob.text())
                            .then((text) => { window.__tonkWrote = text; })
                            .catch(() => {});
                    }
                    // Hand the refusal back unchanged: the control has to see
                    // the same answer it would without this hook, or the test
                    // would be watching a flow nobody ships.
                    return write(items);
                };
                "##,
                Vec::new(),
            )
            .await?;
        driver.enter_default_frame().await?;
        Ok(())
    }

    /// The text the control handed the clipboard, once it has.
    async fn guest_copied_text(driver: &WebDriver) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            enter_guest(driver).await?;
            let text = driver
                .execute(r##"return window.__tonkWrote || "";"##, Vec::new())
                .await?;
            driver.enter_default_frame().await?;
            let text = text.json().as_str().unwrap_or_default().to_owned();
            if !text.is_empty() {
                return Ok(text);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("the bar never handed the clipboard anything"));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Wait for the copy action to leave its resting state.
    ///
    /// This is the assertion the FABB share regression needed and did not
    /// have. A share control bound to no space returns before dispatching
    /// anything, so the action sits on `idle` forever: no mint, no spinner,
    /// no refusal. Every other test in this file reached a share link
    /// through the registration ceremony's own button, which drives a
    /// different control, so all of them stayed green while picking
    /// "copy share link" from the bar did nothing at all.
    async fn await_share_action_working(driver: &WebDriver) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last;
        loop {
            last = share_action_state(driver).await?;
            match last.as_deref() {
                Some(state) if state != "idle" => return Ok(state.to_owned()),
                _ => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the copy action never answered the click; it is showing {last:?}",
                ));
            }
            // Tighter than the usual 250ms: `copied` reverts to `idle`
            // after `COPIED_LINGER_MS`, so a slow poll could sample either
            // side of the whole answer and read an idle action as a dead one.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait for the bar to offer `expected` (`account` or `link`).
    async fn await_share_action(driver: &WebDriver, expected: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last;
        loop {
            last = share_action_offered(driver).await?;
            if last.as_deref() == Some(expected) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let url = driver.current_url().await.ok().map(|url| url.to_string());
                enter_guest(driver).await?;
                let guest = driver
                    .execute(
                        r#"const bars = [...document.querySelectorAll('tonk-fab')];
                        return {
                            hub: !!document.querySelector('.hub-page'),
                            site: !!document.querySelector('tonk-site'),
                            fabDefined: !!customElements.get('tonk-fab'),
                            bars: bars.map((bar) => ({
                                connected: bar.isConnected,
                                attributes: Object.fromEntries([...bar.attributes].map(({name, value}) => [name, value])),
                                children: bar.childElementCount,
                                html: bar.innerHTML.slice(0, 500),
                                parent: bar.parentElement?.tagName ?? null,
                                ancestors: (() => {
                                    const tags = [];
                                    for (let node = bar.parentElement; node; node = node.parentElement) {
                                        tags.push(node.tagName.toLowerCase());
                                    }
                                    return tags;
                                })(),
                            })),
                            text: (document.body?.innerText || '').slice(0, 500),
                        };"#,
                        Vec::new(),
                    )
                    .await
                    .map(|value| value.json().clone())
                    .unwrap_or(serde_json::Value::Null);
                driver.enter_default_frame().await?;
                return Err(anyhow!(
                    "the bar never offered {expected:?}; it is showing {last:?}; url={url:?}; guest={guest}",
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Get to the registration cluster through the FABB's share account gate.
    async fn open_register_dialog(driver: &WebDriver) -> Result<()> {
        click_share_action(driver, "account").await?;
        await_register_dialog(driver).await
    }

    /// Raise the cluster from a space of its own.
    ///
    /// The bar is a space's control, so the Hub has no `tonk-fab` and no
    /// share action to take — reaching for one there fails with "no bar".
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
        // Open the space actions before taking the account gate.
        open_space_actions(driver).await?;
        await_share_action(driver, "account").await?;
        open_register_dialog(driver).await
    }

    /// Wait for the panel that adds an account, in the guest frame.
    ///
    /// The panel is the profile's: the Hub and a space's bar both render
    /// in the guest, and the top page only runs the passkey ceremony the
    /// worker asks it for. Returns with the driver in the guest.
    async fn await_register_dialog(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            enter_guest(driver).await?;
            if !registration_stage(driver).await?.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                let guest = driver
                    .execute(
                        r##"
                        const bar = document.querySelector("tonk-fab");
                        return {
                            url: location.href,
                            seats: document.querySelectorAll(".registration-seat").length,
                            task: !!document.querySelector("account-task"),
                            settings: !!document.querySelector("account-settings"),
                            bar: !!bar,
                            accountRequired: bar?.hasAttribute('data-account-required') ?? null,
                        };
                        "##,
                        Vec::new(),
                    )
                    .await
                    .map(|value| value.json().to_string())
                    .unwrap_or_else(|error| format!("guest diagnostic failed: {error}"));
                return Err(anyhow!(
                    "the panel that adds an account never showed: {guest}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Type an address into the panel and go on, the way Enter does.
    ///
    /// The worker reads the address on submit: a free one moves the panel
    /// to naming the account, a taken one runs the log-in ceremony. A
    /// submit before the panel's display listens is lost, so this waits for
    /// the display to be bound.
    async fn type_into_register_dialog(driver: &WebDriver, address: &str) -> Result<()> {
        enter_guest(driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let outcome = driver
                .execute(
                    r##"
                    const field = document.querySelector('#tonk-register input[name="email"]');
                    if (!field) return { error: "no address field" };
                    if (!field.closest("tonk-display")?.hasAttribute("data-bound")) return { wait: true };
                    field.focus();
                    field.value = arguments[0];
                    field.dispatchEvent(new Event("input", { bubbles: true }));
                    field.form.requestSubmit();
                    return { ok: true };
                    "##,
                    vec![serde_json::json!(address)],
                )
                .await?;
            let value = outcome.json().clone();
            if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
                return Err(anyhow!("could not type the address: {error}"));
            }
            if value.get("ok").is_some() {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the panel's display never started listening"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait for the panel's action to offer `expected`.
    ///
    /// The action is the stage's: "continue" for the address, "create a
    /// passkey" once a free address asks for a name, "waiting for device"
    /// while the page runs the ceremony.
    async fn await_register_action(driver: &WebDriver, expected: &str) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut last;
        loop {
            last = register_action_label(driver).await?;
            if last == expected {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the panel never offered {expected:?}; it shows {last:?}",
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
        submit_hub_wizard_with(driver, "Untitled", "").await
    }

    async fn submit_hub_wizard_with(
        driver: &WebDriver,
        name: &str,
        description: &str,
    ) -> Result<()> {
        enter_hub(driver).await?;
        click(driver, ".header-new [data-space-create-open]").await?;
        wait_for_displayed(driver, ".header-new [data-space-create-dialog]").await?;
        element(driver, ".header-new input[name=name]")
            .await?
            .send_keys(name)
            .await?;
        if !description.is_empty() {
            element(driver, ".header-new textarea[name=description]")
                .await?
                .send_keys(description)
                .await?;
        }
        // Through `click`: the dialog's `space/create` binding must be wired
        // before the press, or the display resolves nothing and the space
        // is never asked for.
        click(driver, ".header-new [data-space-create-submit]").await?;
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
    pub(crate) async fn create_space(driver: &WebDriver, name: &str) -> Result<String> {
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
        let branch = active_branch(driver).await?;
        let before = space_keys(driver).await?;
        // `name` alone: where a space syncs is the worker's to resolve
        // from the account's registration, and template seeding went
        // with the template libraries.
        let claim = tonk_worker_api::create_space_claim_json(name);
        let dispatched = post_json(
            driver,
            &format!("/api/repository/profile:tonk/branch/{branch}/transact"),
            claim,
        )
        .await?;
        successful_body("dispatch space/create", &dispatched);

        // Subscribe for the replica rather than re-reading the profile
        // listing on a timer: the space lands as a `Replica` fact on
        // profile main, and the subscription delivers it on commit.
        let known = serde_json::to_string(&before).unwrap_or_else(|_| "[]".to_owned());
        await_subscription(
            driver,
            &format!("/api/repository/profile:tonk/branch/{branch}/query"),
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
                &format!("/api/repository/profile:tonk/branch/{branch}/query"),
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
        // The API is the profile's worker's, on the profile's own origin:
        // ask from the profile's frame.
        enter_profile(driver).await?;
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
        driver.enter_default_frame().await?;
        Ok(result.json().clone())
    }

    /// POST a YAML document, the way `/evaluate` takes source.
    ///
    /// The JSON routes cannot make a replica write real content, and a
    /// replica with nothing to write never presigns — which is what made
    /// every status-code assertion in this file vacuous.
    async fn post_yaml(driver: &WebDriver, path: &str, body: &str) -> Result<serde_json::Value> {
        // The API is the profile's worker's, on the profile's own origin:
        // ask from the profile's frame.
        enter_profile(driver).await?;
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
        driver.enter_default_frame().await?;
        Ok(result.json().clone())
    }

    /// The account facts `/api/account/summary` used to return, read the
    /// way the app reads them now: a query against the profile branch.
    ///
    /// The route is gone — it answered by reading these very attributes
    /// off this very branch, so it was a second copy of what a query
    /// returns. Shaped like the old response (`status` + `body` with
    /// camelCase keys) so the assertions that consumed it still read.
    /// The branch the profile is on, as the worker reports it. Every
    /// account lives on a branch of its own, so a test that adds, leaves
    /// or switches accounts asks instead of assuming `main`.
    async fn active_branch(driver: &WebDriver) -> Result<String> {
        let profiles = get_json(driver, "/api/profiles").await?;
        successful_body("active branch", &profiles)["active"]
            .as_str()
            .map(str::to_owned)
            .context("profiles omitted the active branch")
    }

    async fn account_summary(driver: &WebDriver) -> Result<serde_json::Value> {
        let endpoint = format!(
            "/api/repository/profile:tonk/branch/{}/query",
            active_branch(driver).await?
        );
        let query = serde_json::json!({
            "predicate": { "with": {
                "email": {
                    "the": "xyz.tonk.account/customer-email",
                    "as": "Text", "cardinality": "one"
                }
            } },
            "terms": {
                "this": { "?": { "name": "this" } },
                "email": { "?": { "name": "email" } }
            }
        });
        let rows = post_json(driver, &endpoint, query).await?;
        let email = rows["body"]
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["fields"]["email"].as_str())
            .filter(|email| !email.trim().is_empty());

        let query = serde_json::json!({
            "predicate": { "with": {
                "name": {
                    "the": "xyz.tonk.account/display-name",
                    "as": "Text", "cardinality": "one"
                }
            } },
            "terms": {
                "this": { "?": { "name": "this" } },
                "name": { "?": { "name": "name" } }
            }
        });
        let rows = post_json(driver, &endpoint, query).await?;
        let display_name = rows["body"]
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(|row| row["fields"]["name"].as_str())
            .filter(|name| !name.trim().is_empty());

        let query = serde_json::json!({
            "predicate": { "with": {
                "created_on": {
                    "the": "xyz.tonk.recovery/created-on",
                    "as": "Text", "cardinality": "one"
                },
                "created_at": {
                    "the": "xyz.tonk.recovery/created-at",
                    "as": "UnsignedInteger", "cardinality": "one"
                }
            } },
            "terms": {
                "this": { "?": { "name": "this" } },
                "created_on": { "?": { "name": "created_on" } },
                "created_at": { "?": { "name": "created_at" } }
            }
        });
        let rows = post_json(driver, &endpoint, query).await?;
        // Newest first, as the panel lists them.
        let passkey = rows["body"].as_array().and_then(|rows| {
            rows.iter()
                .max_by_key(|row| row["fields"]["created_at"].as_u64().unwrap_or(0))
                .map(|row| {
                    serde_json::json!({
                        "createdOn": row["fields"]["created_on"],
                        "createdAt": row["fields"]["created_at"],
                    })
                })
        });

        Ok(serde_json::json!({
            "status": 200,
            "body": {
                "email": email,
                "displayName": display_name,
                "passkey": passkey,
            }
        }))
    }

    async fn get_json(driver: &WebDriver, path: &str) -> Result<serde_json::Value> {
        // The API is the profile's worker's, on the profile's own origin:
        // ask from the profile's frame.
        enter_profile(driver).await?;
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
        driver.enter_default_frame().await?;
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
    /// Copy a share link from the FABB, on an account that can mint one.
    ///
    /// The one path nothing covered. Every other share test here reaches a
    /// link through the registration ceremony's own "copy share link"
    /// button, or asserts the bar's row LABEL without picking it — so the
    /// bar could offer `copy link`, take the click, and do nothing, with
    /// the whole suite green. It did: the bar stamped `<tonk-share>` with
    /// its space once, before the route had resolved one, and never again,
    /// leaving the control bound to nothing for the life of the page.
    ///
    /// The order of the assertions is the point. First the action has to
    /// ANSWER — leave `idle` — because that is the half a control bound to
    /// no space skips; only then is it worth asking whether a link came
    /// back.
    #[dialog_common::test]
    async fn it_copies_a_share_link_from_the_bar(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "barsharer@example.com").await?;

        // Waiting for the remote: a space with none refuses to mint, and
        // this test is about the control, not about that refusal.
        let key = create_space_awaiting_remote(&driver, "Shared From The Bar", true).await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;

        // An active account offers the copy action without an account gate.
        open_space_actions(&driver).await?;
        await_share_action(&driver, "link").await?;

        watch_guest_clipboard(&driver).await?;
        click_share_action(&driver, "link").await?;

        let state = await_share_action_working(&driver).await?;
        assert!(
            matches!(state.as_str(), "copying" | "copied" | "failed"),
            "the action must report what the click did, got {state:?}",
        );

        // What the person ends up holding. Asserted on the text the control
        // handed the clipboard rather than on the invite row, because the row
        // is evicted the moment a copy succeeds — the url carries a
        // membership seed in its fragment and is not left sitting in a
        // subscribable overlay — so reading it back would race that eviction
        // on exactly the runs that went best.
        let invite = guest_copied_text(&driver).await?;
        let (address, seed) = invite
            .split_once('#')
            .ok_or_else(|| anyhow!("an invite carries its seed in a fragment, got {invite:?}"))?;
        assert!(
            address.contains("/join?") && address.contains("access="),
            "the copied link must be a join address carrying a delegation, got {address:?}",
        );
        assert!(
            !seed.is_empty(),
            "the copied link must carry a membership seed, got {invite:?}",
        );

        driver.quit().await?;
        Ok(())
    }

    /// The account customer row has no provider until activation. The FABB
    /// used to require that optional field in its query, so this exact state
    /// resolved as no account: the space offered the account gate and raised the
    /// signup ceremony even though the account already existed.
    #[dialog_common::test]
    async fn it_names_pending_activation_consistently_in_a_space(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "pending-space@example.com";
        enroll_only(&driver, &env, email).await?;
        dismiss_register_dialog(&driver).await?;
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("read the customer state", &customer)["status"],
            "Registered",
            "the account waits on email confirmation"
        );

        let key = create_space(&driver, "Waiting for Email").await?;
        await_url_containing(&driver, &format!("/space/{key}")).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            enter_guest(&driver).await?;
            let condition = driver
                .execute(
                    r#"const action = document.querySelector('tonk-fab')?.shadowRoot?.querySelector('.condition');
                       return { hidden: action?.hasAttribute('hidden'), text: action?.textContent?.trim() };"#,
                    vec![],
                )
                .await?;
            if condition.json()["hidden"] == false
                && condition.json()["text"] == "confirm your email"
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the FAB must name the existing account's pending step: {}",
                condition.json(),
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        driver.enter_default_frame().await?;

        open_space_actions(&driver).await?;
        await_share_action(&driver, "link").await?;
        enter_guest(&driver).await?;
        let share_copy = driver
            .execute(
                r#"const bar = document.querySelector('tonk-fab');
                   const root = bar?.shadowRoot;
                   return {
                     accountHidden: root?.querySelector('.login')?.hasAttribute('hidden'),
                     link: (root?.querySelector('.share span')?.textContent || '').trim()
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
                .is_some_and(|text| text.contains("copy share link")),
            "the ready share action must remain available: {}",
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
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("read the customer state", &customer)["status"],
            "Active"
        );

        // Again no remote: if the worker did not read the recorded
        // provider, this space would come up local-only.
        let key = create_space(&driver, "Made After Activation").await?;

        // The handler creates the space, navigates the client, and
        // attaches the remote AFTER — deliberately, so the navigation
        // does not wait on the network. The space existing is therefore
        // not the attach having landed, so poll rather than read once.
        await_subscription(
            &driver,
            "/api/repository/profile:tonk/branch/main/query",
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

    /// A space created before activation becomes syncable once the account
    /// has fully reconciled, and not before.
    ///
    /// Creation still withholds the remote while the service would refuse
    /// provisioning. Once activation is observed and the account pull has
    /// converged, the worker scans local replicas, provisions every one with
    /// zero remotes, and attaches the account provider without another user
    /// action.
    #[dialog_common::test]
    async fn it_syncs_a_local_only_space_once_the_account_reconciles(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let email = "optin@example.com";
        enroll_only(&driver, &env, email).await?;
        dismiss_register_dialog(&driver).await?;
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("read the customer state", &customer)["status"],
            "Registered",
            "activation is pending until the emailed link is opened"
        );

        let key = create_space(&driver, "Opted In").await?;

        // Creating a space navigates into it — the handler posts a
        // navigate effect to the originating client — so come back to
        // the Hub before confirming the email.
        driver.goto(env.tonk_web.as_str()).await?;

        // Confirm the email, so a provider exists to attach to.
        activate(&driver, &env, email).await?;
        let customer = get_json(&driver, "/api/customer").await?;
        assert_eq!(
            successful_body("read the customer state", &customer)["status"],
            "Active"
        );

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
            let info = successful_body("read the space configuration", &info);
            if info["remote"]["origin"].is_object()
                && info["branch"]["main"]["upstream"]["remote"].as_str() == Some("origin")
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "account reconciliation did not attach origin/main: {info}",
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

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

        let (claimer, authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&claimer, &env, "claimer@example.com").await?;
        let visited = post_json(
            &claimer,
            "/api/profile/join",
            serde_json::json!({ "url": invite_url }),
        )
        .await?;
        successful_body("join shared space", &visited);

        // A sync poke schedules work; it is not a publication barrier. Keep
        // the source browser alive until an independent device has recovered
        // the account directory, exactly as a second-device sign-in does.
        let (recovered, _) =
            second_device_with_same_passkey(&env, &claimer, &authenticator).await?;
        wait_for_service_worker(&recovered).await?;
        raise_cluster_from_hub(&recovered, &env).await?;
        run_cluster_login(&recovered, "claimer@example.com").await?;
        // The trigger wearing the account's name is the sign-in; the
        // address is a fact this second device only holds once the
        // account has synced, so it is not what proves the link.
        let signed_in = async {
            enter_hub(&recovered).await?;
            wait_for_text_without(&recovered, "[data-account-trigger]", "add an account").await?;
            recovered.enter_default_frame().await?;
            Ok::<(), anyhow::Error>(())
        };
        if let Err(wait_error) = signed_in.await {
            recovered.enter_default_frame().await?;
            let status = get_json(&recovered, "/api/account").await?;
            return Err(wait_error).context(format!(
                "second-device sign-in never showed the account: {status}"
            ));
        }

        // Sign-in success precedes the account content pull that
        // carries the directory rows, and the on-demand mount needs
        // those rows. The Hub renders from a live subscription, so
        // arrival is eventually consistent by design; poll the load
        // the same way a page would re-render.
        let mut restored = get_json(&recovered, &format!("/api/repository/{key}")).await?;
        for _ in 0..30 {
            if restored["status"].as_u64().is_some_and(|s| s == 200) {
                break;
            }
            let _ = post_json(&claimer, "/api/sync", serde_json::json!({})).await;
            let _ = post_json(&recovered, "/api/sync", serde_json::json!({})).await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            restored = get_json(&recovered, &format!("/api/repository/{key}")).await?;
        }
        let restored = successful_body("load claimed space on second device", &restored);
        assert_eq!(restored["subject"], key);
        claimer.quit().await?;

        let pulled = post_json(
            &recovered,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull claimed space on second device", &pulled);
        let hydrated = get_json(&recovered, &format!("/api/repository/{key}")).await?;
        assert_eq!(
            successful_body("load pulled space on second device", &hydrated)["label"],
            "Shared Garden"
        );

        recovered.quit().await?;
        Ok(())
    }

    /// The full deletion stack under the button, the way a person runs
    /// it: the reviewed dialog on the Hub's settings page, the typed
    /// address, the acknowledgement, the passkey the worker asks for
    /// through the consent card, and one purge presented to the service.
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

        // Wait for the created space's provider record before reviewing deletion.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let reply = get_json(&driver, "/api/account/deletion/plan").await?;
            let plan = successful_body("review the deletion plan", &reply);
            assert_eq!(plan["email"], email, "plan reveals the verified email");
            if plan["spaces"].as_array().map(Vec::len) == Some(1) {
                assert_eq!(plan["joinedSpaces"], 0);
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "Doomed Garden must be hosted before deletion: {plan}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Creating a space navigates the page into it, so go back to
        // where the deletion controls live.
        open_hub_settings(&driver, &env).await?;
        click(&driver, "[data-delete-account-open]").await?;
        wait_for_text_containing(&driver, "[data-delete-scope]", "1 owned hosted space").await?;

        // The explicit confirmation phrase is the gate: a mistyped one leaves the solid
        // verb off, and nothing is asked of the worker.
        let confirmation = element(&driver, "[data-delete-confirm]").await?;
        confirmation.send_keys("delete").await?;
        let verb = element(&driver, "[data-delete-account-submit]").await?;
        assert!(
            verb.attr("disabled").await?.is_some(),
            "a partial confirmation must not arm the deletion"
        );

        // The real thing: the exact phrase arms the verb, and
        // the passkey gesture the virtual authenticator answers on the
        // consent card the worker's ask raises finishes it.
        driver
            .execute(
                r#"const field = document.querySelector("[data-delete-confirm]");
                   field.value = "";
                   field.dispatchEvent(new Event("input", { bubbles: true }));"#,
                Vec::new(),
            )
            .await
            .ok();
        confirmation.send_keys("delete account").await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while verb.attr("disabled").await?.is_some() {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the exact confirmation phrase must arm the deletion"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // The passkey is asked for on this click, with no card between.
        click(&driver, "[data-delete-account-submit]").await?;

        // The purge retires this profile and rotates onto a fresh one;
        // the top page leaves for the Hub, which offers to add an
        // account again.
        if let Err(error) = await_url_path(&driver, "/").await {
            let consent = custody_consent_diagnostic(&driver).await;
            return Err(error).context(format!("deletion consent={consent}"));
        }
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, "[data-account-trigger]", "add an account").await?;
        driver.enter_default_frame().await?;

        // The profile is unlinked: the deletion plan is no longer
        // reviewable because there is no account to review.
        let after = get_json(&driver, "/api/account/deletion/plan").await?;
        assert_eq!(
            after["status"], 404,
            "a deleted account leaves nothing to plan against: {after}"
        );
        // The retired profile held no joined spaces, so it was forgotten
        // rather than left behind as a ghost local workspace.
        let profiles = get_json(&driver, "/api/profiles").await?;
        assert_eq!(
            successful_body("list profiles after deletion", &profiles)["profiles"]
                .as_array()
                .map(Vec::len),
            Some(1),
            "only the fresh profile remains: {profiles}"
        );

        // The released email creates a genuinely new account on the
        // fresh profile.
        sign_up(&driver, &env, email).await?;
        let recreated = account_summary(&driver).await?;
        assert_eq!(
            successful_body("load the recreated account", &recreated)["email"],
            email
        );

        driver.quit().await?;
        Ok(())
    }

    /// A tab left open on another account reloads itself when the
    /// profile switches under it.
    ///
    /// The worker binds each client to the active-profile generation
    /// and answers a stale client's profile-scoped requests with `409
    /// profile changed; reload required`, so the old tab cannot ACT on
    /// the account it is showing. Without a reload it goes on
    /// DISPLAYING it, which is the worse state: a page that looks
    /// signed in as someone it can no longer be.
    ///
    /// The reload comes from `tonk_host::navigate`, which listens for
    /// the worker's `profile-changed` broadcast and reloads the TOP
    /// page — the guest that may have asked for the switch is an
    /// opaque document whose own reload would fix nothing. That path
    /// had unit coverage but nothing end to end, so a change to the
    /// broadcast, the listener, or the message shape could break every
    /// open tab and no test would notice.
    ///
    /// Proven by a marker planted on the document: a reload discards
    /// it. Asserting on rendered account text instead would pass for
    /// the wrong reason, since a tab that merely re-rendered would
    /// satisfy it too.
    #[dialog_common::test]
    async fn it_reloads_a_tab_whose_profile_switched_in_another_tab(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "watcher@example.com").await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        driver.enter_default_frame().await?;
        let observer = driver.window().await?;

        // Plant the marker. It lives on `window`, so only a genuine
        // document reload clears it.
        driver
            .execute("window.__tonkReloadProbe = 'original';", Vec::new())
            .await?;

        // A second tab switches the profile out from under the first.
        let switcher = driver.new_tab().await?;
        driver.switch_to_window(switcher).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let added = post_json(&driver, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add test profile", &added);
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        await_register_dialog(&driver).await?;

        // Back to the observer: its marker must be gone.
        driver.switch_to_window(observer).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let probe = driver
                .execute("return window.__tonkReloadProbe ?? null;", Vec::new())
                .await
                .ok()
                .map(|ret| ret.json().clone());
            let stale = matches!(
                probe.as_ref().and_then(|value| value.as_str()),
                Some("original")
            );
            if !stale {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the tab kept its pre-switch document; it never reloaded on `profile-changed`"
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

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
        let (first_profile, _) = active_profile_and_label(profiles_before_add)?;

        // The real Hub frame renders the first account's space.
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text_containing(&driver, ".stack", "First Garden").await?;
        driver.enter_default_frame().await?;

        // Prepare a fresh profile through the API, then use the unlinked
        // Hub's account action. Settings no longer exposes account switching.
        let added = post_json(&driver, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add test profile", &added);
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        await_register_dialog(&driver).await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let before_submit = successful_body("list profiles before add submit", &profiles);
        assert_eq!(
            before_submit["profiles"]
                .as_array()
                .context("profile roster is not an array")?
                .len(),
            profile_count_before_add + 1,
            "Add account lands on a fresh profile"
        );
        assert_ne!(
            active_profile_and_label(before_submit)?.0,
            first_profile,
            "Add account switches onto the fresh profile"
        );

        run_cluster_ceremony(&driver, "second@example.com").await?;
        activate(&driver, &env, "second@example.com").await?;

        // The second account sees none of the first account's spaces.
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("list second account's spaces", &listed)).is_empty(),
            "a fresh account must not see the other account's spaces"
        );
        let summary = account_summary(&driver).await?;
        let passkey_created_on =
            successful_body("read second account summary", &summary)["passkey"]["createdOn"]
                .as_str()
                .context("second account summary omitted passkey creation device")?
                .to_string();

        // The second account's sealed Hub has its own empty roster.
        // Signup chose this name. The profile roster can still contain its
        // generated name until account-state convergence projects the choice.
        // Do not save that transient label as the Hub's expected account name.
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        if let Err(error) =
            wait_for_text_containing(&driver, "[data-account-trigger]", "Tab Owner").await
        {
            let diagnostic = driver
                .execute(
                    r#"return {
                        trigger: document.querySelector('[data-account-trigger]')?.textContent,
                        error: document.querySelector('[data-account-error]')?.textContent,
                        errorHidden: document.querySelector('[data-account-error]')?.hidden,
                        tab: document.querySelector('hub-bar')?.getAttribute('path'),
                        linking: document.querySelector('hub-bar')?.getAttribute('linking'),
                        barRegistered: !!customElements.get('hub-bar'),
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
            create_action.contains("new space"),
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

        // The sealed Hub's settings row is a link to the /settings route:
        // the same chrome with the account settings section open and
        // unsupported Devices/Usage/Syncing surfaces absent.
        click(&driver, "[data-account-trigger]").await?;
        await_url_path(&driver, "/settings").await?;
        driver.enter_default_frame().await?;
        enter_hub(&driver).await?;
        element(&driver, "account-settings").await?;
        wait_for_text(&driver, "[data-settings-email]", "second@example.com").await?;
        wait_for_text(
            &driver,
            "[data-settings-passkey-device]",
            passkey_created_on.as_str(),
        )
        .await?;
        assert!(
            driver
                .find_all(By::Css("account-settings [data-pane=\"devices\"]"))
                .await?
                .is_empty(),
            "settings must not expose a devices tab or pane"
        );
        let settings_text = element(&driver, "account-settings")
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

        // The authoritative display-name write repaints the bar's account
        // cell and remains in the field after the page is reloaded.
        let display_name = element(&driver, "[data-settings-name]").await?;
        let select_all = if cfg!(target_os = "macos") {
            Key::Command + "a"
        } else {
            Key::Control + "a"
        };
        display_name.send_keys(select_all).await?;
        display_name.send_keys("Second Hub").await?;
        display_name.send_keys(Key::Enter).await?;
        // Settings hides the header. Observe the fact-backed label's DOM
        // content here; WebDriver's visible text is intentionally empty.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let label = driver.execute(
                "return document.querySelector('[data-account-trigger] [data-account-name]')?.textContent?.trim() || '';",
                vec![],
            ).await?;
            if label.json() == "Second Hub" {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the renamed account did not reach the header: {}",
                label.json()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        driver.enter_default_frame().await?;
        let settings = driver.current_url().await?;
        goto(&driver, settings.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_value(&driver, "[data-settings-name]", "Second Hub").await?;
        driver.enter_default_frame().await?;

        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text(&driver, "[data-account-label]", "Second Hub").await?;

        // Profile switching remains an API capability, without a settings roster.
        let switched = post_json(
            &driver,
            "/api/profiles/activate",
            serde_json::json!({ "profile": first_profile }),
        )
        .await?;
        successful_body("restore first test profile", &switched);
        goto(&driver, &format!("{}settings", env.tonk_web)).await?;
        // The switch was made from the account page and the reload lands
        // there, where the stack is not shown; the spaces tab is the way
        // back to it, pushed in place.
        enter_hub(&driver).await?;
        click(&driver, ".settings-back").await?;
        wait_for_text_containing(&driver, ".stack", "First Garden").await?;
        driver.enter_default_frame().await?;
        let listed = get_json(&driver, "/api/profile").await?;
        assert!(
            space_keys(successful_body("relist first account's spaces", &listed)).contains(&key),
            "switching back must restore the first account's space list"
        );
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        wait_for_displayed(&driver, ".signout-panel").await?;
        assert!(driver.find_all(By::Css(".switch-panel")).await?.is_empty());

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_signs_into_another_account_without_rebinding_retained_local_spaces(
        env: TestEnvironment,
    ) -> Result<()> {
        const FIRST: &str = "retained-first@example.com";
        const SECOND: &str = "retained-second@example.com";

        let (driver, authenticator) = driver_with_prf_authenticator(&env).await?;
        sign_up(&driver, &env, FIRST).await?;
        let first_credential = credential_ids(&driver, &authenticator)
            .await?
            .into_iter()
            .next()
            .context("the first signup minted no passkey")?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let profiles = successful_body("profiles after first signup", &profiles);
        let first_profile = profiles["active"]
            .as_str()
            .context("the first profile has no active handle")?
            .to_string();

        let added = post_json(&driver, "/api/profiles/add", serde_json::json!({})).await?;
        successful_body("add test profile", &added);
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        click(&driver, "[data-account-trigger]").await?;
        driver.enter_default_frame().await?;
        run_cluster_ceremony(&driver, SECOND).await?;
        activate(&driver, &env, SECOND).await?;
        let profiles = get_json(&driver, "/api/profiles").await?;
        let profiles = successful_body("profiles after second signup", &profiles);
        let second_profile = profiles["active"]
            .as_str()
            .context("the second profile has no active handle")?
            .to_string();
        let profile_count = profiles["profiles"]
            .as_array()
            .context("profile roster is not an array")?
            .len();
        assert_ne!(first_profile, second_profile);

        let switched = post_json(
            &driver,
            "/api/profiles/activate",
            serde_json::json!({ "profile": first_profile.clone() }),
        )
        .await?;
        successful_body("switch back to first profile", &switched);
        goto(&driver, env.tonk_web.as_str()).await?;
        open_hub_settings(&driver, &env).await?;
        click(&driver, "[data-sign-out-open]").await?;
        driver.enter_default_frame().await?;
        let before_sign_out = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        enter_hub(&driver).await?;
        click(&driver, "[data-sign-out-submit]").await?;
        wait_for_top_reload(&driver, &before_sign_out, "sign-out").await?;

        // Signing out lands on a branch that follows no account, minted
        // for it: the other account is not switched onto, since nothing
        // on it was asked for, but it stays listed for the switcher.
        let profiles = get_json(&driver, "/api/profiles").await?;
        let profiles = successful_body("profiles after first sign-out", &profiles);
        let landing = profiles["active"]
            .as_str()
            .context("profiles omitted the active profile")?
            .to_owned();
        assert_ne!(
            landing, first_profile,
            "sign-out leaves the account's branch"
        );
        assert_ne!(
            landing, second_profile,
            "sign-out does not switch onto another account unasked"
        );
        let signed_out_count = profiles["profiles"]
            .as_array()
            .context("profile roster is not an array")?
            .len();
        assert_eq!(
            signed_out_count,
            profile_count + 1,
            "sign-out lands on a branch of its own, beside both accounts'"
        );

        // The signed-out profile remains explicitly reachable for its local
        // work, but is no longer the default Hub landing view.
        let switched = post_json(
            &driver,
            "/api/profiles/activate",
            serde_json::json!({ "profile": first_profile.clone() }),
        )
        .await?;
        successful_body("open signed-out first profile", &switched);
        goto(&driver, env.tonk_web.as_str()).await?;

        let retained = create_space_awaiting_remote(&driver, "Retained Draft", false).await?;
        let repository = get_json(&driver, &format!("/api/repository/{retained}")).await?;
        assert!(
            successful_body("retained local space", &repository)["remote"]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "a space created after sign-out must stay local-only"
        );

        remove_credential(&driver, &authenticator, &first_credential).await?;

        // Bind a sibling top-level document to A's current generation. It
        // must be reloaded by the worker before it can observe B.
        let ceremony_tab = driver.window().await?;
        let sibling_tab = driver.new_tab().await?;
        driver.switch_to_window(sibling_tab.clone()).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        wait_for_service_worker(&driver).await?;
        let _ = get_json(&driver, "/api/profile").await?;
        let sibling_before = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        driver.switch_to_window(ceremony_tab.clone()).await?;

        raise_cluster_from_hub(&driver, &env).await?;
        run_cluster_login(&driver, SECOND).await?;

        let profiles = get_json(&driver, "/api/profiles").await?;
        let profiles = successful_body("profiles after routed login", &profiles);
        assert_eq!(profiles["active"], second_profile);
        assert_eq!(
            profiles["profiles"]
                .as_array()
                .context("profile roster is not an array")?
                .len(),
            signed_out_count,
            "routing to an existing account must not create another branch"
        );
        let summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("second account summary", &summary)["email"],
            SECOND
        );
        assert!(
            !space_keys(&driver).await?.contains(&retained),
            "the second account must not inherit the first profile's retained space"
        );

        driver.switch_to_window(sibling_tab).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if driver
                .execute("return performance.timeOrigin", Vec::new())
                .await
                .is_ok_and(|current| current.json() != &sibling_before)
            {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "the sibling tab did not reload after account profile routing"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        wait_for_service_worker(&driver).await?;
        let sibling_summary = account_summary(&driver).await?;
        assert_eq!(
            successful_body("second account in reloaded sibling", &sibling_summary)["email"],
            SECOND,
            "the sibling may render B only after receiving a new client context"
        );
        driver.switch_to_window(ceremony_tab).await?;

        let switched = post_json(
            &driver,
            "/api/profiles/activate",
            serde_json::json!({ "profile": first_profile.clone() }),
        )
        .await?;
        successful_body("return to signed-out workspace", &switched);
        goto(&driver, env.tonk_web.as_str()).await?;
        assert!(space_keys(&driver).await?.contains(&retained));
        let second_local =
            create_space_awaiting_remote(&driver, "Retained Follow-up", false).await?;
        assert!(space_keys(&driver).await?.contains(&second_local));

        driver.quit().await?;
        Ok(())
    }

    /// ACCT-C14 / HANDOFF-21: app chrome keeps person invitations and tool
    /// connections explicit, and the harness-built CLI accepts only the latter.
    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    async fn tool_connection_rejects_person_links_and_confirms_the_cli(
        env: TestEnvironment,
    ) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        sign_up(&browser, &env, "tool-connection@example.com").await?;
        let key = create_space_awaiting_remote(&browser, "Tool connection", true).await?;
        await_url_containing(&browser, &format!("/space/{key}")).await?;

        // The share action remains an ordinary person invite. Exercise the
        // actual clipboard handoff, then prove the CLI refuses it without
        // producing its registry.
        watch_guest_clipboard(&browser).await?;
        await_share_action(&browser, "link").await?;
        click_share_action(&browser, "link").await?;
        let _ = await_share_action_working(&browser).await?;
        let person_link = guest_copied_text(&browser).await?;
        let profile = tempfile::tempdir()?;
        let rejected = run_cli(
            &env,
            &profile,
            &[
                "join".into(),
                person_link,
                "--name".into(),
                "not-a-tool".into(),
            ],
        )
        .await?;
        anyhow::ensure!(
            !rejected.status.success(),
            "person invite reached CLI import"
        );
        anyhow::ensure!(
            rejected
                .stderr
                .contains("This link invites a person to the space.")
                && rejected.stderr.contains("connect agent"),
            "wrong-kind error was not actionable: {}",
            rejected.stderr
        );
        anyhow::ensure!(
            !profile.path().join("spaces/spaces.json").exists(),
            "rejected person invite created a space registry"
        );

        // Connect agent issues one scoped link. Copying the prompt again
        // must retain that identity rather than minting another grant.
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let tool_link = copy_agent_connection_link(&browser)
            .await
            .context("copy initial tool link")?;
        anyhow::ensure!(
            tool_link.contains("#tonk-agent-v2="),
            "tool action copied a non-scoped link"
        );
        let prompt = copy_agent_connection_prompt(&browser).await?;
        anyhow::ensure!(
            prompt.matches(&tool_link).count() == 1
                && prompt.contains("Agent connection confirmed"),
            "tool prompt did not retain the one scoped link"
        );
        assert_prompt_command(&prompt, &env.tonk_web, &tool_link)?;

        // Publish the source branch before the isolated CLI pulls it.
        let pushed = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("publish tool connection source", &pushed);
        let registry = import_agent_bearer(&env, &profile, &tool_link, "tool-space").await?;
        assert!(
            registry
                .get("account")
                .is_none_or(serde_json::Value::is_null),
            "tool connection created a CLI account"
        );

        // Returning to the same space may recover its session invitation.
        // Copying the displayed prompt must not mint another grant.
        goto(
            &browser,
            env.tonk_web.join(&format!("space/{key}"))?.as_str(),
        )
        .await?;
        wait_for_service_worker(&browser).await?;
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let returning = copy_agent_connection_link(&browser)
            .await
            .context("copy returning tool link")?;
        assert_eq!(
            copy_agent_connection_link(&browser)
                .await
                .context("copy returning tool link again")?,
            returning,
            "copying the returning agent prompt minted another grant"
        );

        // Changing spaces replaces the whole app-owned surface. Its explicit
        // target and copied bearer must both belong to the new space, never to
        // the modal that was open on the previous route.
        let second = create_space_awaiting_remote(&browser, "Second tool space", true).await?;
        await_url_containing(&browser, &format!("/space/{second}")).await?;
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &second).await?;
        let switched = copy_agent_connection_link(&browser)
            .await
            .context("copy tool link after switching spaces")?;
        anyhow::ensure!(
            switched != returning && switched != tool_link,
            "space switch exposed a stale tool bearer"
        );

        browser.quit().await?;
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    async fn fabb_connect_agent_receives_a_minted_invitation(env: TestEnvironment) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        sign_up(&browser, &env, "fabb-agent@example.com").await?;
        let key = create_space_awaiting_remote(&browser, "FAB agent", true).await?;
        await_url_containing(&browser, &format!("/space/{key}")).await?;
        enter_guest(&browser).await?;
        let open_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let opened = browser
                .execute(
                    r#"const bar = document.querySelector('tonk-fab');
                       const agent = bar?.querySelector('tonk-agent-panel');
                       const barClass = customElements.get('tonk-fab');
                       const agentClass = customElements.get('tonk-agent-panel');
                       const root = bar?.shadowRoot;
                       if (!barClass || !agentClass || !(bar instanceof barClass) ||
                           !(agent instanceof agentClass) ||
                           typeof agent.__tonkReset !== 'function' ||
                           bar.hasAttribute('data-account-required') ||
                           agent.getAttribute('space') !== arguments[0] ||
                           !root?.querySelector('.space') || !root?.querySelector('.agent'))
                         return false;
                       root.querySelector('.space').click();
                       root.querySelector('.agent').click();
                       return !root.querySelector('#agent-panel').hidden;"#,
                    vec![serde_json::json!(key)],
                )
                .await?;
            if opened.json() == true {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < open_deadline,
                "FAB agent panel did not become ready to open"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            let state = browser
                .execute(
                    r#"const bar = document.querySelector('tonk-fab');
                       const root = bar?.shadowRoot;
                       return {
                         status: root?.querySelector('.agent-status')?.textContent,
                         copyHidden: root?.querySelector('#agent-panel .panel-copy')?.hidden,
                         retryHidden: root?.querySelector('#agent-panel .agent-retry')?.hidden,
                         agentSpace: bar?.querySelector('tonk-agent-panel')?.getAttribute('space'),
                         accountRequired: bar?.hasAttribute('data-account-required')
                       };"#,
                    Vec::new(),
                )
                .await?;
            if state.json()["copyHidden"] == false {
                break;
            }
            if state.json()["retryHidden"] == false || tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "FAB agent invitation did not become ready: {}",
                    state.json()
                );
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        browser.quit().await?;
        Ok(())
    }

    /// ACCT-C14 / HANDOFF-21: an actual browser-issued bearer works after its issuing browser exits.
    /// The test never imports browser/account signing material into the CLI.
    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    // Storybook HANDOFF-21: recover signup into the original scoped invitation.
    async fn it_returns_from_agent_invite_signup_and_connects_the_original_space(
        env: TestEnvironment,
    ) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        goto(&browser, env.tonk_web.as_str()).await?;
        wait_for_service_worker(&browser).await?;
        let key = create_space_awaiting_remote(&browser, "Before signup", false).await?;
        await_url_containing(&browser, &format!("/space/{key}")).await?;
        let original = browser.current_url().await?;
        enter_guest(&browser).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let opened = browser
                .execute(
                    r#"const bar=document.querySelector('tonk-fab');
                       const root=bar?.shadowRoot;
                       const actions=root?.querySelector('.run');
                       if (!bar?.hasAttribute('data-account-required') || !actions) return false;
                       if (actions.hidden) root.querySelector('.space')?.click();
                       const panel=root.querySelector('#agent-panel');
                       if (panel?.hidden) root.querySelector('.agent')?.click();
                       const gate=root.querySelector('.agent-continue');
                       if (panel?.hidden || !gate) return false;
                       gate.click();
                       return true;"#,
                    vec![],
                )
                .await?;
            if opened.json() == true {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "agent account gate did not open"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        browser.enter_default_frame().await?;
        run_cluster_ceremony(&browser, "agent-recovery@example.com").await?;
        activate_in_another_tab(&browser, &env, "agent-recovery@example.com").await?;
        await_registration_stage(&browser, "").await?;
        assert_eq!(browser.current_url().await?, original);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            let info = get_json(&browser, &format!("/api/repository/{key}")).await?;
            let info = successful_body("read recovered space", &info);
            if info["remote"]["origin"].is_object() {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the recovered space never attached a remote: {info}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let pushed = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("publish recovered space", &pushed);
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let invite = copy_agent_connection_link(&browser)
            .await
            .context("copy recovered invitation")?;
        let profile = tempfile::tempdir()?;
        let output = tonk_command_in(&env, &profile)
            .args(["join", &invite, "--name", "recovered-space"])
            .env("TONK_CONNECTION_ORIGIN", env.tonk_web.as_str())
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "agent import failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.path().join("spaces/spaces.json"))?)?;
        assert!(
            registry["spaces"]
                .as_object()
                .unwrap()
                .values()
                .any(|entry| entry["connection"]["subject"]
                    .as_str()
                    .is_some_and(|subject| subject.ends_with(&key)))
        );
        browser.quit().await?;
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    async fn it_connects_with_an_ordinary_bearer_after_the_issuer_closes(
        env: TestEnvironment,
    ) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        sign_up(&browser, &env, "ordinary-agent@example.com").await?;
        let key = create_space_awaiting_remote(&browser, "Ordinary agent", true).await?;
        await_url_containing(&browser, &format!("/space/{key}")).await?;
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let invite = copy_agent_connection_link(&browser).await?;
        let prompt = copy_agent_connection_prompt(&browser).await?;
        assert!(!prompt.contains("--switch-account"));
        assert_eq!(prompt.matches(&invite).count(), 1);
        assert_prompt_command(&prompt, &env.tonk_web, &invite)?;
        assert!(invite.contains("#tonk-agent-v2="));
        browser.enter_default_frame().await?;
        let groups = get_json(&browser, "/api/account/connections").await?;
        let groups = successful_body("list issued connection", &groups);
        assert_eq!(
            groups.as_array().context("expected connection list")?.len(),
            1
        );
        assert_eq!(groups[0]["confirmed"], false);
        assert_eq!(groups[0]["targets"].as_array().unwrap().len(), 6);
        assert!(!serde_json::to_string(groups)?.contains("tonk-agent-v"));
        let group_id = groups[0]["id"]
            .as_str()
            .context("missing grant group id")?
            .to_owned();
        let browser_profile = std::fs::read_dir(&env.browser_profile_root)?
            .next()
            .context("browser profile directory missing")??
            .path();
        let pushed = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("publish invitation source", &pushed);
        assert_agent_browser_logs_redacted(&env, &browser).await?;
        browser.quit().await?;

        let profile = tempfile::tempdir()?;
        let mut command = tonk_command_in(&env, &profile);
        command
            .args([
                "join",
                &invite,
                "--name",
                "ordinary-agent",
                "--agent-name",
                "Codex on work laptop",
            ])
            .env("TONK_CONNECTION_ORIGIN", env.tonk_web.as_str())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(120), command.output())
            .await
            .context("scoped import timed out")??;
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        assert!(!stdout.contains(&invite) && !stderr.contains(&invite));
        assert!(!stdout.contains("tonk-agent-v") && !stderr.contains("tonk-agent-v"));
        assert!(output.status.success(), "scoped import failed: {stderr}");
        assert!(stdout.contains("Agent connection confirmed"));
        assert!(!stdout.contains("Open this URL"));
        let registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(profile.path().join("spaces/spaces.json"))?)?;
        assert!(
            registry
                .get("account")
                .is_none_or(serde_json::Value::is_null)
        );
        let document = "attribute!: &agent-built\n  description: Built after browser shutdown\n  the: test.agent/built\n  as: text\n  cardinality: one\n";
        let built = run_cli(
            &env,
            &profile,
            &[
                "--space".into(),
                "ordinary-agent".into(),
                "eval".into(),
                "-c".into(),
                document.into(),
                "--no-sync".into(),
            ],
        )
        .await?;
        assert!(built.status.success(), "{}", built.stderr);
        let resumed = run_cli(
            &env,
            &profile,
            &["--space".into(), "ordinary-agent".into(), "join".into()],
        )
        .await?;
        assert!(resumed.status.success(), "{}", resumed.stderr);
        assert!(resumed.stdout.contains("Agent connection confirmed"));

        // Reopen the same browser's on-disk profile, with no live issuer process
        // during either CLI connection. No exported passkey or root key is used.
        let caps = env.chrome_capabilities_for_profile(&browser_profile)?;
        let browser = WebDriver::new(env.chromedriver.as_str(), caps).await?;
        goto(&browser, env.tonk_web.join("settings")?.as_str()).await?;
        wait_for_service_worker(&browser).await?;
        let pulled = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("read CLI setup acknowledgement", &pulled);
        let groups = get_json(&browser, "/api/account/connections").await?;
        let groups = successful_body("read retained grant group", &groups);
        assert_eq!(groups[0]["id"], group_id);
        assert_eq!(groups[0]["confirmed"], true);
        assert_eq!(groups[0]["spaceName"], "Ordinary agent");
        assert_eq!(groups[0]["installations"].as_array().unwrap().len(), 1);
        assert_eq!(
            groups[0]["installations"][0]["name"],
            "Codex on work laptop"
        );
        let revoked = post_json(
            &browser,
            &format!("/api/account/connections/{group_id}/revoke"),
            serde_json::json!({}),
        )
        .await?;
        let revoked = successful_body("revoke issued invitation", &revoked);
        assert!(
            revoked["targets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|target| target["acknowledged"] == true)
        );
        browser.quit().await?;
        let denied = run_cli(
            &env,
            &profile,
            &["--space".into(), "ordinary-agent".into(), "join".into()],
        )
        .await?;
        assert!(!denied.status.success());
        assert!(denied.stderr.contains("revoked"), "{}", denied.stderr);
        assert!(!denied.stdout.contains("Agent connection confirmed"));
        assert!(!denied.stderr.contains("account login"));
        let retained = run_cli(&env, &profile, &[
            "--space".into(), "ordinary-agent".into(), "eval".into(), "-c".into(),
            "attribute!: &offline-after-revoke\n  description: Retained offline work\n  the: test.agent/retained\n  as: text\n  cardinality: one\n".into(),
            "--no-sync".into(),
        ]).await?;
        assert!(retained.status.success(), "{}", retained.stderr);
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    async fn settings_hides_unconfirmed_agent_invitations(env: TestEnvironment) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        sign_up(&browser, &env, "settings-agent-filter@example.com").await?;
        goto(&browser, env.tonk_web.join("settings")?.as_str()).await?;
        enter_hub(&browser).await?;
        wait_for_displayed(&browser, "[data-connections-refresh]").await?;
        let result = browser.execute_async(r#"
            const done = arguments[arguments.length - 1];
            const settings = document.querySelector('account-settings');
            const original = settings.api;
            const pending = Array.from({length: 20}, (_, i) => ({id: String(i).padStart(64, '0'), confirmed: false}));
            settings.api = () => Promise.resolve(pending);
            settings.connectionsRefresh();
            const until = async predicate => {
                const deadline = performance.now() + 5000;
                while (!predicate()) {
                    if (performance.now() > deadline) throw new Error('settings did not finish loading');
                    await new Promise(resolve => requestAnimationFrame(resolve));
                }
            };
            (async () => {
                await until(() => settings.querySelector('[data-connections-status]').textContent !== 'Loading connections…');
                const hidden = settings.querySelectorAll('[data-connection-id]').length;
                settings.api = () => Promise.resolve([...pending, {
                    id: 'a'.repeat(64), subject: 'did:key:fixture', label: 'confirmed fixture',
                    confirmed: true, status: 'active', expiresAt: 2000000000,
                    targets: [{cid: 'fixture', acknowledged: false}]
                }]);
                settings.connectionsRefresh();
                await until(() => settings.querySelectorAll('[data-connection-id]').length === 1);
                settings.api = original;
                done({hidden, confirmed: settings.querySelectorAll('[data-connection-id]').length});
            })().catch(error => { settings.api = original; done({error: error.message}); });
        "#, vec![]).await?;
        assert_eq!(result.json()["hidden"], 0);
        assert_eq!(result.json()["confirmed"], 1);
        browser.quit().await?;
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    async fn settings_groups_named_agent_access_and_retries_removal(
        env: TestEnvironment,
    ) -> Result<()> {
        let browser = driver_with_prf(&env).await?;
        sign_up(&browser, &env, "settings-named-agents@example.com").await?;
        goto(&browser, env.tonk_web.join("settings")?.as_str()).await?;
        enter_hub(&browser).await?;
        wait_for_displayed(&browser, "[data-connections-refresh]").await?;
        let result = browser.execute_async(r#"
            const done = arguments[arguments.length - 1];
            const settings = document.querySelector('account-settings');
            const original = settings.api;
            const until = async predicate => {
                const deadline = performance.now() + 5000;
                while (!predicate()) {
                    if (performance.now() > deadline) throw new Error('settings did not settle');
                    await new Promise(resolve => requestAnimationFrame(resolve));
                }
            };
            const group = (id, subject, status = 'active') => ({
                id: id.repeat(64), subject, recipient: 'did:key:recipient', label: 'issued label',
                spaceName: 'Same display name', confirmed: true, status, expiresAt: 2000000000,
                targets: [{cid: 'one', acknowledged: false}, {cid: 'two', acknowledged: false}]
            });
            const shared = group('a', 'did:key:first');
            shared.installations = [{id: '1'.repeat(32), name: 'Codex on work laptop'},
                {id: '2'.repeat(32), name: '<img src=x onerror=alert(1)>'}];
            const legacy = group('b', 'did:key:second');
            const partial = {...group('c', 'did:key:first', 'partial'),
                targets: [{cid: 'one', acknowledged: true}, {cid: 'two', acknowledged: false, error: 'offline'}]};
            const expired = group('d', 'did:key:inactive-only', 'expired');
            const revoked = {...group('e', 'did:key:second', 'revoked'),
                targets: [{cid: 'one', acknowledged: true}]};
            const pending = {...group('f', 'did:key:hidden'), confirmed: false};
            const terminal = {...group('0', 'did:key:terminal'), requestId: 'terminal'};
            const fixtures = [shared, legacy, partial, expired, revoked, pending, terminal];
            settings.api = () => Promise.resolve(fixtures);
            settings.connectionsRefresh();
            (async () => {
                await until(() => settings.querySelectorAll('[data-connection-id]').length === 3);
                const initial = {
                    spaces: [...settings.querySelectorAll('[data-connection-space]')].map(node => node.dataset.connectionSpace),
                    names: [...settings.querySelectorAll('.connection-space__name')].map(node => node.textContent),
                    sharedButtons: settings.querySelectorAll('[data-connection-id="' + shared.id + '"] button').length,
                    disclosures: settings.querySelectorAll('[data-connection-id] details').length,
                    labels: [...settings.querySelectorAll('.connection-installations li')].map(node => node.textContent),
                    injected: settings.querySelectorAll('[data-connections-list] img').length,
                    legacy: settings.querySelector('[data-connection-id="' + legacy.id + '"]').textContent,
                    inactiveRows: settings.querySelectorAll('[data-connection-id="' + expired.id + '"], [data-connection-id="' + revoked.id + '"]').length,
                    retry: settings.querySelector('[data-connection-revoke="' + partial.id + '"]').textContent
                };
                settings.api = () => Promise.resolve({...partial, status: 'revoked',
                    targets: partial.targets.map(target => ({...target, acknowledged: true, error: null}))});
                settings.querySelector('[data-connection-revoke="' + partial.id + '"]').click();
                await until(() => !settings.querySelector('[data-connection-id="' + partial.id + '"]'));
                initial.removed = !settings.querySelector('[data-connection-revoke="' + partial.id + '"]');
                settings.api = () => Promise.resolve({...legacy, status: 'revoked',
                    targets: legacy.targets.map(target => ({...target, acknowledged: true}))});
                settings.querySelector('[data-connection-revoke="' + legacy.id + '"]').click();
                await until(() => !settings.querySelector('[data-connection-space="did:key:second"]'));
                initial.emptySpaceHidden = true;
                settings.api = () => Promise.resolve([expired, revoked]);
                settings.connectionsRefresh();
                await until(() => settings.querySelector('[data-connections-status]').textContent === 'No active connections.');
                initial.inactiveOnlySpaces = settings.querySelectorAll('[data-connection-space]').length;
                // New names come from the current projection, not the issue-time label.
                settings.api = () => Promise.resolve([{...shared, spaceName: 'Renamed space'}, {...legacy, spaceName: null}]);
                settings.connectionsRefresh();
                await until(() => settings.querySelectorAll('[data-connection-id]').length === 2);
                initial.renamed = [...settings.querySelectorAll('.connection-space__name')].map(node => node.textContent);
                // A late response must not repaint after the next refresh.
                let late;
                settings.api = () => new Promise(resolve => { late = resolve; });
                settings.connectionsRefresh();
                settings.api = () => Promise.reject(new Error('offline'));
                settings.connectionsRefresh();
                await until(() => settings.querySelector('[data-connections-status]').textContent.includes('could not be loaded'));
                late(fixtures);
                await new Promise(resolve => requestAnimationFrame(resolve));
                initial.error = settings.querySelector('[data-connections-status]').textContent;
                initial.staleRows = settings.querySelectorAll('[data-connection-id]').length;
                initial.errorVisible = !settings.querySelector('[data-agent-connections]').hidden;
                // Retain a rendered fixture for desktop/mobile visual inspection.
                settings.api = () => Promise.resolve(fixtures);
                settings.connectionsRefresh();
                await until(() => settings.querySelectorAll('[data-connection-id]').length === 3);
                done(initial);
            })().catch(error => { settings.api = original; done({error: error.message}); });
        "#, vec![]).await?;
        let state = result.json();
        assert_eq!(
            state["spaces"],
            serde_json::json!(["did:key:first", "did:key:second"]),
            "{state}"
        );
        assert_eq!(
            state["names"],
            serde_json::json!(["Same display name", "Same display name"])
        );
        assert_eq!(state["sharedButtons"], 1);
        assert_eq!(state["disclosures"], 0);
        assert_eq!(
            state["labels"],
            serde_json::json!(["Codex on work laptop", "<img src=x onerror=alert(1)>"])
        );
        assert_eq!(state["injected"], 0);
        assert!(
            state["legacy"]
                .as_str()
                .unwrap()
                .contains("Name unavailable")
        );
        assert_eq!(state["inactiveRows"], 0);
        assert_eq!(state["emptySpaceHidden"], true);
        assert_eq!(state["inactiveOnlySpaces"], 0);
        assert_eq!(state["retry"], "retry removal");
        assert_eq!(state["removed"], true);
        assert_eq!(
            state["renamed"],
            serde_json::json!(["Renamed space", "did:key:second"])
        );
        assert_eq!(state["staleRows"], 0);
        assert_eq!(state["errorVisible"], true);
        assert!(
            state["error"]
                .as_str()
                .unwrap()
                .contains("could not be loaded")
        );
        capture_connection_management(&browser, &"a".repeat(64), "named-access").await?;
        browser.quit().await?;
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    async fn capture_handoff_page(driver: &WebDriver, name: &str) -> Result<()> {
        if let Some(directory) = std::env::var_os("TONK_HANDOFF_TEST_ARTIFACTS") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory)?;
            // Optional review capture waits for the shell's entrance animation;
            // test readiness and actions do not depend on this delay.
            tokio::time::sleep(Duration::from_millis(500)).await;
            driver
                .screenshot(&directory.join(format!("{name}.png")))
                .await?;
        }
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    async fn capture_connection_management(
        browser: &WebDriver,
        group_id: &str,
        prefix: &str,
    ) -> Result<()> {
        if std::env::var_os("TONK_HANDOFF_TEST_ARTIFACTS").is_some() {
            for (name, width, height, dark) in [
                ("desktop", 1200, 900, false),
                ("narrow", 390, 844, false),
                ("short-dark", 390, 540, true),
            ] {
                browser.enter_default_frame().await?;
                browser.set_window_rect(0, 0, width, height).await?;
                ChromeDevTools::new(browser.handle.clone()).execute_cdp_with_params(
                    "Emulation.setDeviceMetricsOverride", serde_json::json!({
                        "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false,
                    })).await?;
                ChromeDevTools::new(browser.handle.clone()).execute_cdp_with_params(
                    "Emulation.setEmulatedMedia", serde_json::json!({ "features": [
                        { "name": "prefers-reduced-motion", "value": "reduce" },
                        { "name": "prefers-color-scheme", "value": if dark { "dark" } else { "light" } },
                    ] })).await?;
                let outer=browser.execute("return {clientWidth:document.documentElement.clientWidth, scrollWidth:document.documentElement.scrollWidth}", vec![]).await?;
                assert_eq!(outer.json()["clientWidth"], serde_json::json!(width));
                assert!(
                    outer.json()["scrollWidth"].as_u64().unwrap() <= u64::from(width),
                    "outer document overflows: {}",
                    outer.json()
                );
                enter_hub(browser).await?;
                let before = browser.execute(r#"const node=document.querySelector('[data-connections-refresh]');node.focus();
                    window.__connectionKeys=[];
                    document.addEventListener('keydown',event=>{const key={target:event.target.tagName,cls:event.target.className,key:event.key};window.__connectionKeys.push(key);setTimeout(()=>key.prevented=event.defaultPrevented,0);},{once:true});
                    return {focused:document.activeElement === node, documentFocus:document.hasFocus(),
                        rect:node.getBoundingClientRect().toJSON(), disabled:node.disabled,
                        hiddenAncestor:!!node.closest('[hidden]'), activeTag:document.activeElement.tagName,
                        activeClass:document.activeElement.className,
                        revokes:[...document.querySelectorAll('[data-connection-revoke]')].map(item=>({disabled:item.disabled,tabIndex:item.tabIndex,rect:item.getBoundingClientRect().toJSON(),hiddenAncestor:!!item.closest('[hidden]')}))};"#, vec![]).await?;
                browser
                    .find(By::Css("[data-connections-refresh]"))
                    .await?
                    .send_keys(Key::Tab)
                    .await?;
                let focused = browser.execute(r#"const node=document.activeElement;const style=getComputedStyle(node);
                    return {settingsHidden:document.querySelector('[data-settings-view]')?.hidden, accountExpanded:document.querySelector('.account-trigger')?.getAttribute('aria-expanded'), keys:window.__connectionKeys, tag:node.tagName, class:node.className, refresh:node.matches('[data-connections-refresh]'), revoke:node.matches('[data-connection-revoke]'), visible:node.matches(':focus-visible'),
                        ring:style.outlineStyle !== 'none' || style.boxShadow !== 'none',
                        height:node.getBoundingClientRect().height, animation:style.animationName,
                        clientWidth:document.documentElement.clientWidth, scrollWidth:document.documentElement.scrollWidth };"#, vec![]).await?;
                let state = focused.json();
                assert_eq!(
                    state["revoke"],
                    true,
                    "keyboard focus did not reach revoke: {state}; before={}",
                    before.json()
                );
                assert_eq!(
                    state["visible"], true,
                    "keyboard focus was not visible: {state}"
                );
                assert_eq!(
                    state["ring"], true,
                    "keyboard focus ring was absent: {state}"
                );
                assert!(
                    state["height"].as_f64().unwrap() >= 44.0,
                    "small revoke target: {state}"
                );
                assert_eq!(
                    state["animation"], "none",
                    "reduced-motion control animates: {state}"
                );
                assert_eq!(state["clientWidth"], serde_json::json!(width));
                assert!(
                    state["scrollWidth"].as_u64().unwrap()
                        <= state["clientWidth"].as_u64().unwrap(),
                    "management content overflows horizontally: {state}"
                );
                element(browser, &format!("[data-connection-revoke='{group_id}']"))
                    .await?
                    .scroll_into_view()
                    .await?;
                capture_handoff_page(browser, &format!("{prefix}-{name}")).await?;
            }
        }

        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    async fn assert_agent_browser_logs_redacted(
        env: &TestEnvironment,
        browser: &WebDriver,
    ) -> Result<()> {
        if std::env::var_os("TONK_E2E_CHROME_LOG").is_none() {
            return Ok(());
        }
        let endpoint = env
            .chromedriver
            .join(&format!("session/{}/se/log", browser.session_id()))?;
        let logs = reqwest::Client::new()
            .post(endpoint)
            .json(&serde_json::json!({"type":"browser"}))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        anyhow::ensure!(logs["value"].is_array(), "browser log capture unavailable");
        anyhow::ensure!(
            !serde_json::to_string(&logs)?.contains("tonk-agent-v"),
            "browser logs disclosed an agent invitation"
        );
        Ok(())
    }

    #[cfg(feature = "connection-invites")]
    async fn import_agent_bearer(
        env: &TestEnvironment,
        profile: &TempDir,
        link: &str,
        alias: &str,
    ) -> Result<serde_json::Value> {
        let directory = profile.path().join(format!("project-{alias}"));
        std::fs::create_dir_all(&directory)?;
        let mut command = tonk_command_in(env, profile);
        command
            .current_dir(&directory)
            .args(["join", link, "--name", alias])
            .env("TONK_CONNECTION_ORIGIN", env.tonk_web.as_str())
            .kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(120), command.output())
            .await
            .context("bearer import timed out")??;
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        anyhow::ensure!(
            !stdout.contains(link)
                && !stderr.contains(link)
                && !stdout.contains("tonk-agent-v")
                && !stderr.contains("tonk-agent-v"),
            "CLI disclosed an invitation in its output"
        );
        anyhow::ensure!(output.status.success(), "scoped import failed: {stderr}");
        anyhow::ensure!(
            stdout.contains("Agent connection confirmed"),
            "CLI omitted confirmation"
        );
        Ok(serde_json::from_slice(&std::fs::read(
            profile.path().join("spaces/spaces.json"),
        )?)?)
    }

    #[cfg(feature = "connection-invites")]
    #[dialog_common::test]
    // ACCT-C14 / HANDOFF-21: multiple holders, independent groups, retained accounts.
    async fn it_keeps_copied_agent_grants_independent_of_cli_accounts(
        env: TestEnvironment,
    ) -> Result<()> {
        // Emulate a retained pre-upgrade registry without invoking the retired
        // account-login command. Exact ambient-authority isolation is also
        // covered by the native connection import tests.
        let retained_profile = tempfile::tempdir()?;
        let registry_file = retained_profile.path().join("spaces/spaces.json");
        std::fs::create_dir_all(registry_file.parent().context("registry parent")?)?;
        let retained = serde_json::json!({
            "spaces": {},
            "account": { "root": "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL" }
        });
        std::fs::write(&registry_file, serde_json::to_vec(&retained)?)?;
        let before: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry_file)?)?;
        anyhow::ensure!(
            before["account"].is_object(),
            "fixture has no existing CLI account"
        );
        let previous_profiles = std::fs::read_dir(&env.browser_profile_root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        let browser = driver_with_prf(&env).await?;
        let browser_profile = std::fs::read_dir(&env.browser_profile_root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .find(|path| !previous_profiles.contains(path))
            .context("issuer profile missing")?;
        sign_up(&browser, &env, "agent-issuer@example.com").await?;
        let key = create_space_awaiting_remote(&browser, "Independent agents", true).await?;
        await_url_containing(&browser, &format!("/space/{key}")).await?;
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let first = copy_agent_connection_link(&browser)
            .await
            .context("copy first tool connection")?;
        let one = poll_json(
            &browser,
            "/api/account/connections",
            "first issued group",
            |body| body.as_array().is_some_and(|rows| rows.len() == 1),
        )
        .await?;
        let first_id = one[0]["id"]
            .as_str()
            .context("first id missing")?
            .to_owned();
        // A browser restart loses the worker-session bearer. Explicitly opening
        // connect agent then creates a separate identity; a page reload does not.
        browser.quit().await?;
        let caps = env.chrome_capabilities_for_profile(&browser_profile)?;
        let browser = WebDriver::new(env.chromedriver.as_str(), caps).await?;
        goto(
            &browser,
            env.tonk_web.join(&format!("space/{key}"))?.as_str(),
        )
        .await?;
        wait_for_service_worker(&browser).await?;
        click_agent_connection_action(&browser).await?;
        await_agent_connection_ready(&browser, &key).await?;
        let second = copy_agent_connection_link(&browser)
            .await
            .context("copy second tool connection")?;
        anyhow::ensure!(
            first != second,
            "explicit tool action reused the first bearer"
        );
        let two = poll_json(
            &browser,
            "/api/account/connections",
            "explicit second issued group",
            |body| body.as_array().is_some_and(|rows| rows.len() == 2),
        )
        .await?;
        let sibling = two
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] != first_id)
            .context("second group missing")?;
        anyhow::ensure!(
            sibling["recipient"] != one[0]["recipient"],
            "new invite reused recipient key"
        );
        let sibling_id = sibling["id"]
            .as_str()
            .context("second id missing")?
            .to_owned();
        let pushed = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/push"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("publish source", &pushed);
        assert_agent_browser_logs_redacted(&env, &browser).await?;
        browser.quit().await?;

        let empty = tempfile::tempdir()?;
        let first_registry = import_agent_bearer(&env, &empty, &first, "holder").await?;
        assert!(
            first_registry
                .get("account")
                .is_none_or(serde_json::Value::is_null)
        );
        let schema = "attribute!: &agent-title\n  description: Agent note title\n  the: test.agent/title\n  as: text\n  cardinality: one\nconcept!: &agent-note\n  description: Agent authored note\n  with:\n    title: agent-title\nattribute!: &agent-html\n  description: Agent page body\n  the: text/html\n  as: text\n  cardinality: many\nconcept!: &agent-page\n  description: Agent authored page\n  with:\n    body: agent-html\n";
        for document in [
            schema,
            "agent-note!: &agent-note-one\n  title: Built with scoped authority\nagent-page!: &agent-page-one\n  body: '<h1>Scoped browser build</h1>'\n",
        ] {
            let built = run_cli(
                &env,
                &empty,
                &[
                    "--space".into(),
                    "holder".into(),
                    "eval".into(),
                    "-c".into(),
                    document.into(),
                ],
            )
            .await?;
            anyhow::ensure!(
                built.status.success(),
                "scoped build failed: {}",
                built.stderr
            );
        }
        let asset = empty.path().join("agent-asset.txt");
        std::fs::write(&asset, "scoped browser blob readback")?;
        let added = run_cli(
            &env,
            &empty,
            &[
                "--space".into(),
                "holder".into(),
                "blob".into(),
                "add".into(),
                asset.to_string_lossy().into_owned(),
            ],
        )
        .await?;
        anyhow::ensure!(
            added.status.success(),
            "scoped blob add failed: {}",
            added.stderr
        );
        let blob = added.stdout.trim().to_owned();
        anyhow::ensure!(
            blob.starts_with("asset:"),
            "blob add omitted its content reference"
        );
        let second_registry =
            import_agent_bearer(&env, &retained_profile, &first, "holder").await?;
        assert_eq!(
            second_registry["spaces"]["holder"]["connection"],
            first_registry["spaces"]["holder"]["connection"]
        );
        let sibling_registry =
            import_agent_bearer(&env, &retained_profile, &second, "sibling").await?;
        anyhow::ensure!(
            sibling_registry["spaces"]["holder"]["site"]
                != sibling_registry["spaces"]["sibling"]["site"],
            "independent grants shared a local replica"
        );
        assert_eq!(sibling_registry["account"], before["account"]);
        for (name, entry) in before["spaces"]
            .as_object()
            .context("prior spaces missing")?
        {
            assert_eq!(&sibling_registry["spaces"][name], entry);
        }
        let readback = run_cli(
            &env,
            &retained_profile,
            &[
                "--space".into(),
                "holder".into(),
                "blob".into(),
                "cat".into(),
                blob,
            ],
        )
        .await?;
        anyhow::ensure!(
            readback.status.success(),
            "blob readback failed: {}",
            readback.stderr
        );
        assert_eq!(readback.stdout, "scoped browser blob readback");
        let caps = env.chrome_capabilities_for_profile(&browser_profile)?;
        let browser = WebDriver::new(env.chromedriver.as_str(), caps).await?;
        goto(&browser, env.tonk_web.join("settings")?.as_str()).await?;
        wait_for_service_worker(&browser).await?;
        // The page-only bearer is deliberately gone after a browser restart.
        // Existing public grant records must make a fresh invitation explicit.
        goto(
            &browser,
            env.tonk_web.join(&format!("space/{key}"))?.as_str(),
        )
        .await?;
        enter_guest(&browser).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let surface = loop {
            let surface = browser.execute(
                r#"const bar=document.querySelector('tonk-fab');
                   const agent=bar?.querySelector('tonk-agent-panel');
                   const panel=bar?.shadowRoot?.querySelector('#agent-panel');
                   if (!panel || typeof agent?.__tonkReset !== 'function') return null;
                   return { hidden:panel.hidden, hasLink:!!panel.querySelector('.panel-copytext')?.textContent };"#,
                vec![],
            ).await?;
            if !surface.json().is_null() {
                break surface;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "restarted agent panel never mounted"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        browser.enter_default_frame().await?;
        anyhow::ensure!(
            surface.json()["hidden"] == true && surface.json()["hasLink"] == false,
            "restart exposed a retained agent bearer: {}",
            surface.json()
        );
        let after_restart = get_json(&browser, "/api/account/connections").await?;
        assert_eq!(
            successful_body("restart did not issue new grants", &after_restart)
                .as_array()
                .unwrap()
                .len(),
            2
        );
        goto(&browser, env.tonk_web.join("settings")?.as_str()).await?;
        let pulled = post_json(
            &browser,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("read both acknowledgements", &pulled);
        let confirmed = get_json(&browser, "/api/account/connections").await?;
        let confirmed = successful_body("confirmed groups", &confirmed);
        assert!(
            confirmed
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["confirmed"] == true)
        );
        let shared = confirmed
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == first_id)
            .unwrap();
        assert_eq!(shared["installations"].as_array().unwrap().len(), 2);
        assert_ne!(
            shared["installations"][0]["id"],
            shared["installations"][1]["id"]
        );
        let revoked = post_json(
            &browser,
            &format!("/api/account/connections/{first_id}/revoke"),
            serde_json::json!({}),
        )
        .await?;
        let revoked = successful_body("revoke issued invitation", &revoked);
        assert!(
            revoked["targets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|target| target["acknowledged"] == true)
        );
        let groups = get_json(&browser, "/api/account/connections").await?;
        let sibling = successful_body("sibling remains active", &groups)
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == sibling_id)
            .context("sibling disappeared")?;
        assert!(
            sibling["targets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|target| target["acknowledged"] == false)
        );
        browser.quit().await?;
        for holder in [&empty, &retained_profile] {
            let denied = run_cli(
                &env,
                holder,
                &["--space".into(), "holder".into(), "join".into()],
            )
            .await?;
            anyhow::ensure!(
                !denied.status.success() && denied.stderr.contains("revoked"),
                "revoked holder was not denied"
            );
            anyhow::ensure!(
                !denied.stdout.contains("Agent connection confirmed")
                    && !denied.stderr.contains("account login"),
                "revoked holder confirmed or requested fallback"
            );
        }
        let survives = run_cli(
            &env,
            &retained_profile,
            &["--space".into(), "sibling".into(), "join".into()],
        )
        .await?;
        anyhow::ensure!(
            survives.status.success(),
            "independent invitation was revoked: {}",
            survives.stderr
        );
        let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry_file)?)?;
        assert_eq!(after["account"], before["account"]);
        Ok(())
    }

    #[dialog_common::test]
    async fn local_space_link_preserves_identity_data_and_uses_the_browser_account(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "local-space-link@example.com").await?;
        let account = get_json(&driver, "/api/account").await?;
        let account_root = successful_body("selected browser account", &account)["rootDid"]
            .as_str()
            .context("account response omitted rootDid")?
            .to_owned();

        let profile = tempfile::tempdir()?;
        let created = run_cli(
            &env,
            &profile,
            &["space".into(), "new".into(), "garden".into()],
        )
        .await?;
        anyhow::ensure!(
            created.status.success(),
            "space new failed: {}",
            created.stderr
        );
        let subject = created
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("DID: "))
            .context("space new omitted its DID")?
            .to_owned();
        let key = subject.clone();
        let marker = r#"attribute!: &local-space-link-proof
  the:         xyz.tonk.e2e/local-space-link-proof
  as:          text
  cardinality: one
  description: local space link e2e marker
"#;
        let wrote = run_cli(
            &env,
            &profile,
            &[
                "--space".into(),
                "garden".into(),
                "eval".into(),
                "-c".into(),
                marker.into(),
                "--no-sync".into(),
            ],
        )
        .await?;
        anyhow::ensure!(
            wrote.status.success(),
            "local write failed: {}",
            wrote.stderr
        );

        // A retained pre-upgrade account is deliberately unrelated. The
        // browser, not ambient CLI state, must choose the owner.
        let registry_file = profile.path().join("spaces/spaces.json");
        let mut registry: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&registry_file)?)?;
        let unrelated = "did:key:z6MkgMn9hDxTd2saBSAouyTpPLWUmzrVTXfS1N5yB4TjJ3qL";
        registry["account"] = serde_json::json!({ "root": unrelated });
        std::fs::write(&registry_file, serde_json::to_vec_pretty(&registry)?)?;

        // Fail the first browser completion request. The settings page must
        // retain the exact request and retry it on reload while the terminal
        // continues waiting on the same callback.
        //
        // The request is made by the profile's frame, to its own worker. A
        // script the debugger preloads into the page's documents does not
        // reach that frame, which is another site's and another process's,
        // so the frame's own `fetch` is wrapped from inside it: on each
        // look, in whichever document the frame then holds, until the
        // refusal shows. The mark that the one refusal was spent is the
        // frame's origin's, so the reloaded document is not refused again.
        // Answers with the status the page shows.
        const FAIL_COMPLETION_ONCE: &str = r#"
            if (!window.__tonkFailLocalLinkOnce) {
              window.__tonkFailLocalLinkOnce = true;
              const asked = window.fetch;
              window.fetch = function (input, init) {
                const url = typeof input === 'string' ? input : input.url;
                if (url.endsWith('/api/local-space-link/complete') &&
                    localStorage.getItem('tonk:test:fail-local-link-once') !== 'done') {
                  localStorage.setItem('tonk:test:fail-local-link-once', 'done');
                  return Promise.resolve(new Response('injected retry', { status: 503 }));
                }
                return asked.call(window, input, init);
              };
            }
            return document.querySelector('[data-ceremony-status]')?.textContent ?? '';
        "#;
        async fn refuse_completion_once(driver: &WebDriver) -> String {
            let status = async {
                enter_guest(driver).await?;
                let status = driver.execute(FAIL_COMPLETION_ONCE, Vec::new()).await?;
                Ok::<_, anyhow::Error>(status.json().as_str().unwrap_or_default().to_owned())
            }
            .await;
            // A frame between documents has nothing to wrap yet.
            status.unwrap_or_default()
        }

        let mut command = tonk_command_in(&env, &profile);
        command.args([
            "space",
            "link",
            "garden",
            "--no-open",
            "--via",
            env.tonk_web.join("settings/link")?.as_str(),
        ]);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().context("CLI stdout was not piped")?);
        let mut stderr = child.stderr.take().context("CLI stderr was not piped")?;
        let mut heading = String::new();
        let mut url_line = String::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            stdout.read_line(&mut heading).await?;
            stdout.read_line(&mut url_line).await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .context("timed out waiting for local-space approval URL")??;
        if heading.is_empty() {
            let status = child.wait().await?;
            let mut stderr_text = String::new();
            use tokio::io::AsyncReadExt as _;
            stderr.read_to_string(&mut stderr_text).await?;
            return Err(anyhow!(
                "the CLI exited before printing the local-space approval URL ({status}); its stderr: {stderr_text}"
            ));
        }
        assert_eq!(heading.trim_end(), "Approve this space in Tonk:");
        let approval_url = url::Url::parse(url_line.trim())?;
        assert_eq!(approval_url.path(), "/settings/link");
        assert_eq!(
            approval_url
                .query_pairs()
                .find(|(key, _)| key == "intent")
                .map(|(_, value)| value.into_owned())
                .as_deref(),
            Some("local-space-link")
        );

        goto(&driver, approval_url.as_str()).await?;
        enter_guest(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"local-link\"]").await?;
        wait_for_text(&driver, "[data-local-link-name]", "garden").await?;
        assert_eq!(
            element(&driver, "[data-local-link-did]")
                .await?
                .text()
                .await?,
            subject
        );
        click(&driver, "[data-local-link-approve]").await?;
        // Approval visits the loopback callback and redirects the top page
        // back to Tonk with the signed approval and targeted invitation.
        // Wait for that round trip before entering the replacement guest;
        // otherwise Chrome can reset us to the top context after we enter the
        // old frame but before the continuation page finishes navigating.
        driver.enter_default_frame().await?;
        let redirect_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let current = driver.current_url().await?;
            let returned = current.path() == "/settings/link"
                && current
                    .query_pairs()
                    .any(|(key, value)| key == "approval" && !value.is_empty())
                && current
                    .query_pairs()
                    .any(|(key, value)| key == "invite" && !value.is_empty());
            if returned {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < redirect_deadline,
                "browser did not return from local-space approval; current URL was {current}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        enter_guest(&driver).await?;
        // Provisioning returns through the same loopback bridge. The CLI
        // publishes the local branch while that callback waits, then redirects
        // the top page to the signed `provisioned` continuation. Re-enter the
        // replacement guest instead of observing the first continuation's
        // iframe after its top-level navigation has been superseded.
        driver.enter_default_frame().await?;
        let provisioned_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let current = driver.current_url().await?;
            if current.path() == "/settings/link"
                && current
                    .query_pairs()
                    .any(|(key, value)| key == "provisioned" && !value.is_empty())
            {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < provisioned_deadline,
                "browser did not return after local-space provisioning; current URL was {current}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The continuation's document asks to complete as soon as it has
        // read the request: its `fetch` is wrapped before then.
        let refused_deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let status = refuse_completion_once(&driver).await;
            if status.contains("Reload to retry") {
                break;
            }
            if tokio::time::Instant::now() >= refused_deadline {
                driver.enter_default_frame().await?;
                let current = driver.current_url().await?;
                let source = driver.source().await.unwrap_or_default();
                return Err(anyhow!(
                    "local-space continuation stopped at {current} without the refusal showing; status={status:?}; page={}",
                    source.chars().take(1_000).collect::<String>()
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        driver.enter_default_frame().await?;
        driver.refresh().await?;

        let completion_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if tokio::time::Instant::now() >= completion_deadline {
                let current = driver.current_url().await?;
                let workspace = if current.path() == "/settings/link" {
                    driver.enter_default_frame().await?;
                    match driver.find(By::Css("tonk-site > iframe")).await {
                        Ok(frame) => {
                            frame.enter_frame().await?;
                            driver
                                .execute(
                                    r#"const status=document.querySelector('[data-ceremony-status]');
                            return {
                              statusText: status?.textContent,
                              statusHidden: status?.hidden,
                              finishing: document.querySelector('account-settings')
                                ?.hasAttribute('data-local-link-finishing'),
                            };"#,
                                    vec![],
                                )
                                .await
                                .map(|value| value.json().clone())
                        }
                        Err(error) => Ok(serde_json::json!({
                            "guestUnavailable": error.to_string()
                        })),
                    }
                } else {
                    Ok(serde_json::json!({ "notSettings": true }))
                };
                return Err(anyhow!(
                    "browser did not deliver local-space completion; current={current}; workspace={workspace:?}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let prefix = format!("{heading}{url_line}");
        let linked = finish_link(&mut child, &mut stdout, &mut stderr, prefix).await?;
        anyhow::ensure!(
            linked.status.success(),
            "space link failed: {}",
            linked.stderr
        );
        anyhow::ensure!(linked.stdout.contains(&format!("DID: {subject}")));
        anyhow::ensure!(
            linked
                .stdout
                .contains(&format!("Linked space 'garden' to {account_root}"))
        );

        // The same browser account can pull the CLI's published content, and
        // the identifiable local fact is present on that exact repository.
        driver.enter_default_frame().await?;
        goto(&driver, env.tonk_web.join("settings")?.as_str()).await?;
        wait_for_service_worker(&driver).await?;
        // Completion records the space in the account directory. A fresh
        // worker mounts directory-listed replicas lazily before branch routes
        // can address their IndexedDB database.
        let mut mounted = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let first_mount = mounted.clone();
        for _ in 0..30 {
            if mounted["status"]
                .as_u64()
                .is_some_and(|status| status == 200)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            mounted = get_json(&driver, &format!("/api/repository/{key}")).await?;
        }
        anyhow::ensure!(
            mounted["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status)),
            "mount linked local space failed; first={first_mount}; final={mounted}"
        );
        let pulled = post_json(
            &driver,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull linked local space", &pulled);
        anyhow::ensure!(
            owner_sees(&driver, &key, "local-space-link-proof").await?,
            "the browser account did not receive the original local fact"
        );

        // A fresh CLI process uses the retained space authority directly;
        // the unrelated legacy account remains untouched and is not exposed
        // by status.
        let pulled = run_cli(
            &env,
            &profile,
            &["--space".into(), "garden".into(), "pull".into()],
        )
        .await?;
        anyhow::ensure!(
            pulled.status.success(),
            "CLI pull failed: {}",
            pulled.stderr
        );
        let status = run_cli(
            &env,
            &profile,
            &[
                "--space".into(),
                "garden".into(),
                "status".into(),
                "--json".into(),
            ],
        )
        .await?;
        anyhow::ensure!(
            status.status.success(),
            "CLI status failed: {}",
            status.stderr
        );
        let status: serde_json::Value = serde_json::from_str(&status.stdout)?;
        assert_eq!(status["schemaVersion"], "tonk.status.v3");
        assert!(status.get("account").is_none());
        let registry: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry_file)?)?;
        assert_eq!(registry["account"]["root"], unrelated);

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn local_space_link_decline_preserves_the_local_space(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, "local-space-link-decline@example.com").await?;
        let profile = tempfile::tempdir()?;
        let created = run_cli(
            &env,
            &profile,
            &["space".into(), "new".into(), "garden".into()],
        )
        .await?;
        anyhow::ensure!(
            created.status.success(),
            "space new failed: {}",
            created.stderr
        );
        let subject = created
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("DID: "))
            .context("space new omitted its DID")?
            .to_owned();
        let registry_file = profile.path().join("spaces/spaces.json");
        let registry_before = std::fs::read(&registry_file)?;

        let mut command = tonk_command_in(&env, &profile);
        command.args([
            "space",
            "link",
            "garden",
            "--no-open",
            "--via",
            env.tonk_web.join("settings/link")?.as_str(),
        ]);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().context("CLI stdout was not piped")?);
        let mut stderr = child.stderr.take().context("CLI stderr was not piped")?;
        let mut heading = String::new();
        let mut url_line = String::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            stdout.read_line(&mut heading).await?;
            stdout.read_line(&mut url_line).await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .context("timed out waiting for local-space approval URL")??;

        goto(&driver, url_line.trim()).await?;
        enter_guest(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"local-link\"]").await?;
        assert_eq!(
            element(&driver, "[data-local-link-did]")
                .await?
                .text()
                .await?,
            subject
        );
        click(&driver, "[data-local-link-decline]").await?;

        let declined = finish_link(
            &mut child,
            &mut stdout,
            &mut stderr,
            format!("{heading}{url_line}"),
        )
        .await?;
        anyhow::ensure!(
            !declined.status.success(),
            "declined link unexpectedly succeeded"
        );
        anyhow::ensure!(
            declined.stderr.contains("space link declined"),
            "decline did not reach the waiting CLI: {}",
            declined.stderr
        );
        assert_eq!(std::fs::read(&registry_file)?, registry_before);
        let status = run_cli(
            &env,
            &profile,
            &[
                "--space".into(),
                "garden".into(),
                "status".into(),
                "--json".into(),
            ],
        )
        .await?;
        anyhow::ensure!(
            status.status.success(),
            "local space became unusable: {}",
            status.stderr
        );
        let status: serde_json::Value = serde_json::from_str(&status.stdout)?;
        assert_eq!(status["space"]["name"], "garden");
        assert!(status.get("account").is_none());
        let listed = run_cli(&env, &profile, &["space".into(), "--json".into()]).await?;
        anyhow::ensure!(
            listed.status.success(),
            "space listing failed: {}",
            listed.stderr
        );
        let listed: serde_json::Value = serde_json::from_str(&listed.stdout)?;
        assert!(
            listed["rows"].as_array().is_some_and(|spaces| spaces
                .iter()
                .any(|space| { space["name"] == "garden" && space["subject"] == subject })),
            "decline changed the local space identity: {listed}"
        );

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn local_space_link_survives_fresh_account_creation(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        let profile = tempfile::tempdir()?;
        let created = run_cli(
            &env,
            &profile,
            &["space".into(), "new".into(), "garden".into()],
        )
        .await?;
        anyhow::ensure!(
            created.status.success(),
            "space new failed: {}",
            created.stderr
        );
        let subject = created
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix("DID: "))
            .context("space new omitted its DID")?
            .to_owned();

        let mut command = tonk_command_in(&env, &profile);
        command.args([
            "space",
            "link",
            "garden",
            "--no-open",
            "--via",
            env.tonk_web.join("settings/link")?.as_str(),
        ]);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let mut stdout = BufReader::new(child.stdout.take().context("CLI stdout was not piped")?);
        let mut stderr = child.stderr.take().context("CLI stderr was not piped")?;
        let mut heading = String::new();
        let mut url_line = String::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            stdout.read_line(&mut heading).await?;
            stdout.read_line(&mut url_line).await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .context("timed out waiting for local-space approval URL")??;

        goto(&driver, url_line.trim()).await?;
        await_register_dialog(&driver).await?;
        let email = "local-space-link-fresh-account@example.com";
        run_cluster_ceremony(&driver, email).await?;
        activate_in_another_tab(&driver, &env, email).await?;

        let continuation_deadline = tokio::time::Instant::now() + Duration::from_secs(45);
        loop {
            let current = driver.current_url().await?;
            let dialog_gone = enter_guest(&driver).await.is_ok()
                && registration_stage(&driver)
                    .await
                    .is_ok_and(|stage| stage.is_empty());
            if current.path() == "/settings/link" && dialog_gone {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < continuation_deadline,
                "account creation lost the local-space continuation; current URL was {current}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        wait_for_service_worker(&driver).await?;
        enter_guest(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"local-link\"]").await?;
        assert_eq!(
            element(&driver, "[data-local-link-did]")
                .await?
                .text()
                .await?,
            subject
        );

        // End the test without publishing; cancellation is already the
        // independently verified terminal path and keeps this case focused on
        // preservation across account creation.
        click(&driver, "[data-local-link-decline]").await?;
        let declined = finish_link(
            &mut child,
            &mut stdout,
            &mut stderr,
            format!("{heading}{url_line}"),
        )
        .await?;
        anyhow::ensure!(
            !declined.status.success(),
            "declined link unexpectedly succeeded"
        );

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
    async fn it_rejects_the_wrong_account_for_an_agent_handoff(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let before = get_json(&driver, "/api/identity/root").await?;
        let (callback, mut delivered) = waiting_cli().await?;
        let expected = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let mut url = env.tonk_web.join("settings/link")?;
        url.query_pairs_mut()
            .append_pair("audience", expected)
            .append_pair("callback", &callback)
            .append_pair("expectedAccount", expected);
        goto(&driver, url.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"link\"]").await?;
        assert_eq!(
            element(&driver, "[data-link-account]")
                .await?
                .text()
                .await?,
            expected
        );
        click(&driver, "[data-link-approve]").await?;
        enter_hub(&driver).await?;
        wait_for_text_containing(
            &driver,
            "[data-ceremony-status]",
            "this handoff requires account",
        )
        .await?;
        assert!(
            matches!(
                delivered.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "wrong-account approval must not deliver a grant"
        );
        let after = get_json(&driver, "/api/identity/root").await?;
        assert_eq!(
            successful_body("before", &before)["rootDid"],
            successful_body("after", &after)["rootDid"]
        );
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_authorizes_a_waiting_cli_from_the_browser(env: TestEnvironment) -> Result<()> {
        use base64::Engine as _;

        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;

        let root = get_json(&driver, "/api/identity/root").await?;
        let expected = successful_body("expected account", &root)["rootDid"]
            .as_str()
            .context("root DID missing")?
            .to_owned();
        let (callback, delivered) = waiting_cli().await?;
        let audience = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let mut url = env.tonk_web.join("settings/link")?;
        url.query_pairs_mut()
            .append_pair("audience", audience)
            .append_pair("callback", &callback)
            .append_pair("expectedAccount", &expected);
        goto(&driver, url.as_str()).await?;

        // The settings page names the device that is waiting, so the
        // user knows what they are approving.
        enter_hub(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"link\"]").await?;
        let shown = element(&driver, "[data-link-did]").await?.text().await?;
        assert_eq!(shown, audience, "the page must name the waiting device");
        assert_eq!(
            element(&driver, "[data-link-account]")
                .await?
                .text()
                .await?,
            expected
        );

        // The passkey is asked for on the approving click itself, so the
        // watch on what it allows goes in before that click.
        driver.enter_default_frame().await?;
        driver
            .execute(
                r#"window.__cliLinkAllowCredentials = "not called";
                   const realGet = navigator.credentials.get.bind(navigator.credentials);
                   navigator.credentials.get = options => {
                     window.__cliLinkAllowCredentials =
                       options?.publicKey?.allowCredentials?.length ?? null;
                     return realGet(options);
                   };"#,
                Vec::new(),
            )
            .await?;
        enter_hub(&driver).await?;
        click(&driver, "[data-link-approve]").await?;
        driver.enter_default_frame().await?;
        // Read on this page: the approval ends by leaving it for the
        // callback, where the watch is gone and reads as nothing.
        let home = env.tonk_web.origin().ascii_serialization();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let allowed = loop {
            let seen = driver
                .execute(
                    "return [location.origin, window.__cliLinkAllowCredentials]",
                    Vec::new(),
                )
                .await?;
            let seen = seen.json();
            anyhow::ensure!(
                seen[0] == home.as_str(),
                "the page left before the passkey was seen being asked for: {seen}"
            );
            if seen[1] != "not called" {
                break seen[1].clone();
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "approving never asked for the passkey"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert_eq!(
            allowed,
            serde_json::Value::Null,
            "CLI linking must let the passkey provider offer any credential for this account"
        );

        // Generous: approving runs a passkey assertion, the unlock, and
        // the device registration before the callback navigation, and a loaded
        // CI runner stretches each of them.
        let (field, value) = tokio::time::timeout(Duration::from_secs(60), delivered)
            .await
            .context("the page never delivered an authorization")??;
        assert_eq!(field, "authorize", "approving must deliver a grant");

        // What arrived is what the CLI decodes: base64 over a payload
        // carrying the delegation (whose signed meta names the sync
        // endpoint), the fallback remote for older CLIs, and the exact
        // service attachment needed for a crash-safe CLI activation.
        let decoded = base64::engine::general_purpose::STANDARD.decode(&value)?;
        let payload: serde_json::Value = serde_json::from_slice(&decoded)?;
        for field in ["delegationHex", "remote", "attachmentId"] {
            assert!(
                payload
                    .get(field)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                "the authorization must carry {field}: {payload}"
            );
        }
        // A browser left running keeps writing its profile while the
        // workspace is being removed — the "Directory not empty"
        // teardown race every other test avoids by quitting.
        driver.quit().await?;
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

        enter_hub(&driver).await?;
        wait_for_displayed(&driver, "account-settings [data-pane=\"link\"]").await?;
        click(&driver, "[data-link-decline]").await?;

        let (field, _) = tokio::time::timeout(Duration::from_secs(60), delivered)
            .await
            .context("cancelling never reached the waiting process")??;
        assert_eq!(
            field, "deny",
            "cancelling must report a denial, not leave the CLI waiting"
        );
        // See the authorize variant: an unquit browser races workspace
        // removal with its profile writes.
        driver.quit().await?;
        Ok(())
    }

    /// Push the space at `key` until its remote accepts it. The service
    /// takes a push only for a space provisioned there, so acceptance is
    /// the proof the space is provided where it syncs.
    async fn push_until_accepted(driver: &WebDriver, key: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let pushed = post_json(
                driver,
                &format!("/api/repository/{key}/branch/main/sync/push"),
                serde_json::json!({}),
            )
            .await?;
            if pushed["status"]
                .as_u64()
                .is_some_and(|status| (200..300).contains(&status))
            {
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the space {key} was never accepted where it syncs: {pushed}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Sign the sibling deployment in through `home`, the way a person
    /// does: Option on its "add an account", name `home` in the dialog that
    /// asks which Tonk, approve on `home` with its passkey, and come back to
    /// the sibling's Hub. Reports the pane's status and the browser log when
    /// the sibling never comes back signed in.
    async fn sign_in_through(driver: &WebDriver, env: &TestEnvironment, home: &str) -> Result<()> {
        let here = env.sibling_web();
        let here_origin = here.origin().ascii_serialization();
        // Option on "add an account" asks which Tonk holds the account.
        goto(driver, here.as_str()).await?;
        enter_hub(driver).await?;
        // Pressed once the bar is live and the cell offers to add an
        // account: before that, the link is the bare page's `/settings`
        // and nothing yet hears Option.
        let trigger = wait_for_displayed(
            driver,
            "hub-bar:defined [data-account-trigger][href=\"/account\"]",
        )
        .await?;
        driver
            .action_chain()
            .key_down(Key::Alt)
            .click_element(&trigger)
            .key_up(Key::Alt)
            .perform()
            .await?;
        await_registration_stage(driver, "via").await?;
        let field = wait_for_displayed(driver, "#tonk-register input[name=\"via\"]").await?;
        field.send_keys(home).await?;
        click(driver, "#tonk-register #tonk-register-action").await?;

        // On the deployment holding the account, the approval names the
        // page the grant would go to.
        await_url_containing(driver, &format!("{home}/settings/link?")).await?;
        enter_hub(driver).await?;
        wait_for_displayed(driver, "account-settings [data-pane=\"link\"]").await?;
        wait_for_text(driver, "[data-link-return]", &here_origin).await?;
        // One step: the click asks for the passkey, with no screen between.
        click(driver, "[data-link-approve]").await?;

        // Back on the asking origin, at home and signed in.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            driver.enter_default_frame().await?;
            let url = driver.current_url().await?;
            if url.origin().ascii_serialization() == here_origin && url.path() == "/" {
                break;
            }
            anyhow::ensure!(
                driver.find(By::Css("#tonk-custody-consent")).await.is_err(),
                "approving put a second passkey screen up instead of asking on the click: {}",
                custody_consent_diagnostic(driver).await
            );
            if tokio::time::Instant::now() >= deadline {
                dump_browser_log(driver, env).await;
                // What the page said last: its status row carries the
                // worker's refusal when finishing failed.
                let said = async {
                    enter_hub(driver).await?;
                    let settings = element(driver, "account-settings").await?;
                    let pane = settings.attr("data-pane").await?;
                    let status = element(driver, "account-settings [data-ceremony-status]")
                        .await?
                        .prop("textContent")
                        .await?;
                    driver.enter_default_frame().await?;
                    Ok::<_, anyhow::Error>(format!("pane {pane:?}, status {status:?}"))
                }
                .await
                .unwrap_or_else(|error| format!("its state is unreadable: {error}"));
                anyhow::bail!(
                    "the asking origin never came back signed in; the page is at {url}, {said}"
                );
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Ok(())
    }

    /// Option after a plain "add an account": the email panel the plain
    /// click raised, put away by going back to the spaces, gives way to the
    /// one asking which Tonk instead of coming back.
    #[dialog_common::test]
    async fn it_asks_which_tonk_after_a_plain_add_an_account(env: TestEnvironment) -> Result<()> {
        let driver = driver_with_prf(&env).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        enter_hub(&driver).await?;
        let cell = "hub-bar:defined [data-account-trigger][href=\"/account\"]";
        wait_for_displayed(&driver, cell).await?.click().await?;
        await_registration_stage(&driver, "address").await?;

        // Back to the spaces: leaving the account page puts the panel away.
        driver.enter_default_frame().await?;
        driver.back().await?;
        await_url_path(&driver, "/").await?;
        await_registration_stage(&driver, "").await?;

        let trigger = wait_for_displayed(&driver, cell).await?;
        driver
            .action_chain()
            .key_down(Key::Alt)
            .click_element(&trigger)
            .key_up(Key::Alt)
            .perform()
            .await?;
        await_registration_stage(&driver, "via").await?;
        anyhow::ensure!(
            driver
                .find(By::Css("#tonk-register input[name=\"email\"]"))
                .await
                .is_err(),
            "the email face is still up beside the one asking which Tonk"
        );
        driver.quit().await?;
        Ok(())
    }

    /// Signing a browser in through another deployment, the browser's
    /// `tonk account login --via`, and then using it. A browser on a second
    /// deployment, which has never seen the account or its passkey, asks
    /// the deployment holding it; the person approves there with that
    /// passkey; and the asking origin comes back signed in to the same
    /// account. The account's name showing there proves it synced through
    /// the deployment that approved.
    ///
    /// Then a space made on the second deployment has to land at home: it
    /// is provisioned and pushed to the account's deployment, whose service
    /// is the only one holding the account, and the home device recovers it
    /// from there. The second deployment runs its own access service, so a
    /// call sent to it instead is refused rather than quietly served.
    #[dialog_common::test(sibling = true)]
    async fn it_signs_a_browser_in_through_another_deployment(env: TestEnvironment) -> Result<()> {
        const EMAIL: &str = "sign-in-via@example.com";
        const NAME: &str = "Via";
        const SPACE: &str = "Made elsewhere";

        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let home = env.tonk_web.origin().ascii_serialization();
        let root = successful_body(
            "the account's root",
            &get_json(&driver, "/api/identity/root").await?,
        )["rootDid"]
            .as_str()
            .context("root DID missing")?
            .to_owned();
        successful_body(
            "name the account",
            &post_json(
                &driver,
                "/api/account/display-name",
                serde_json::json!({ "name": NAME }),
            )
            .await?,
        );
        successful_body(
            "publish the account name",
            &post_json(&driver, "/api/sync", serde_json::json!({})).await?,
        );

        sign_in_through(&driver, &env, &home).await?;
        let here = env.sibling_web();
        let here_origin = here.origin().ascii_serialization();
        let installed = get_json(&driver, "/api/identity/root").await?;
        assert_eq!(
            successful_body("the installed root", &installed)["rootDid"],
            root.as_str(),
            "the asking origin holds a grant from the account that approved"
        );

        // A space made the moment the page lands. Its seed is sealed to the
        // account's key, which this origin can only learn from the account
        // itself, having no passkey to derive it from: the sign-in pulls
        // the account before landing, or this asks for a passkey that can
        // never answer here. And it syncs with the account's deployment.
        let key = create_space_awaiting_remote(&driver, SPACE, true).await?;
        let info = get_json(&driver, &format!("/api/repository/{key}")).await?;
        let info = successful_body("read the new space", &info);
        let remote = info["remote"]["origin"].to_string();
        assert!(
            remote.contains(env.tonk_web.join("ucan/")?.as_str()) && !remote.contains(&here_origin),
            "the space syncs with the deployment holding the account: {info}"
        );
        push_until_accepted(&driver, &key).await?;
        // Making the space opened it; the account's name is on the Hub.
        await_account_name(&driver, NAME).await?;
        goto(&driver, here.as_str()).await?;
        enter_hub(&driver).await?;
        wait_for_text(&driver, "[data-account-label]", NAME).await?;
        driver.enter_default_frame().await?;

        // And the account's own device, on its deployment, finds it. In a
        // tab of its own: this one stays open so its worker keeps running
        // until the account branch carrying the space has gone out, since
        // a sync poke schedules work rather than finishing it.
        let elsewhere = driver.window().await?;
        let home_tab = driver.new_tab().await?;
        driver.switch_to_window(home_tab.clone()).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        wait_for_service_worker(&driver).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut restored = get_json(&driver, &format!("/api/repository/{key}")).await?;
        while restored["status"].as_u64() != Some(200) {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the home device never found the space made on the other deployment: {restored}"
            );
            driver.switch_to_window(elsewhere.clone()).await?;
            let _ = post_json(&driver, "/api/sync", serde_json::json!({})).await;
            driver.switch_to_window(home_tab.clone()).await?;
            let _ = post_json(&driver, "/api/sync", serde_json::json!({})).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            restored = get_json(&driver, &format!("/api/repository/{key}")).await?;
        }
        let pulled = post_json(
            &driver,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull the space at home", &pulled);
        let hydrated = get_json(&driver, &format!("/api/repository/{key}")).await?;
        assert_eq!(
            successful_body("load the space at home", &hydrated)["label"],
            SPACE
        );
        driver.quit().await?;
        Ok(())
    }

    /// Signing in through another deployment on a browser that has work
    /// of its own, to an account that has spaces of its own. Neither side
    /// is lost: the account's space shows up on the browser that just
    /// signed in, and the space that browser made before signing in moves
    /// under the account, syncing with and accepted by the account's
    /// deployment, rather than being left behind or discarded.
    #[dialog_common::test(sibling = true)]
    async fn it_keeps_both_sides_spaces_when_signing_in_through_another_deployment(
        env: TestEnvironment,
    ) -> Result<()> {
        const EMAIL: &str = "sign-in-via-both@example.com";
        const AT_HOME: &str = "Made at home";
        const BEFORE: &str = "Made before signing in";

        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let home = env.tonk_web.origin().ascii_serialization();
        let at_home = create_space_awaiting_remote(&driver, AT_HOME, true).await?;
        push_until_accepted(&driver, &at_home).await?;
        successful_body(
            "publish the account",
            &post_json(&driver, "/api/sync", serde_json::json!({})).await?,
        );

        // Work of its own on the second deployment, before anyone signs in.
        let here = env.sibling_web();
        goto(&driver, here.as_str()).await?;
        wait_for_service_worker(&driver).await?;
        let before = create_space(&driver, BEFORE).await?;

        sign_in_through(&driver, &env, &home).await?;

        let listed = space_keys(&driver).await?;
        assert!(
            listed.contains(&before),
            "signing in discarded the space made before it: {listed:?}"
        );
        // Moved under the account: it now syncs with the account's
        // deployment, which accepts it.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let info = get_json(&driver, &format!("/api/repository/{before}")).await?;
            let info = successful_body("read the space made before signing in", &info);
            let remote = info["remote"]["origin"].to_string();
            if remote.contains(env.tonk_web.join("ucan/")?.as_str()) {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the space made before signing in never moved to the account's deployment: {info}"
            );
            let _ = post_json(&driver, "/api/sync", serde_json::json!({})).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        push_until_accepted(&driver, &before).await?;
        let kept = get_json(&driver, &format!("/api/repository/{before}")).await?;
        assert_eq!(
            successful_body("load the space made before signing in", &kept)["label"],
            BEFORE
        );

        // And the account's own space shows up here.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut found = get_json(&driver, &format!("/api/repository/{at_home}")).await?;
        while found["status"].as_u64().is_none_or(|status| status != 200) {
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "the account's space never showed up after signing in: {found}; listed {:?}",
                space_keys(&driver).await.unwrap_or_default()
            );
            let _ = post_json(&driver, "/api/sync", serde_json::json!({})).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            found = get_json(&driver, &format!("/api/repository/{at_home}")).await?;
        }
        let pulled = post_json(
            &driver,
            &format!("/api/repository/{at_home}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("pull the account's space", &pulled);
        let hydrated = get_json(&driver, &format!("/api/repository/{at_home}")).await?;
        assert_eq!(
            successful_body("load the account's space", &hydrated)["label"],
            AT_HOME
        );
        assert!(
            space_keys(&driver).await?.contains(&at_home),
            "the account's space is not listed on the Hub"
        );
        driver.quit().await?;
        Ok(())
    }

    /// Signing back in through another deployment, after signing out there,
    /// returns the browser to the branch the account kept. The page that
    /// asked lands there too: it loads afresh rather than routing, since a
    /// document still bound to the signed-out profile has every request
    /// refused, and the account's spaces show at once. A space made while
    /// signed out came before the sign-in, so it joins the account: listed
    /// beside the account's own, syncing with and accepted by the account's
    /// deployment, and no empty signed-out workspace is left behind.
    #[dialog_common::test(sibling = true)]
    async fn it_lands_on_the_account_when_signing_back_in_through_another_deployment(
        env: TestEnvironment,
    ) -> Result<()> {
        const EMAIL: &str = "sign-in-via-again@example.com";
        const KEPT: &str = "Kept by the account";
        const MADE: &str = "Made while signed out";

        let driver = driver_with_prf(&env).await?;
        sign_up(&driver, &env, EMAIL).await?;
        let home = env.tonk_web.origin().ascii_serialization();
        sign_in_through(&driver, &env, &home).await?;
        let kept = create_space_awaiting_remote(&driver, KEPT, true).await?;

        let here = env.sibling_web();
        goto(&driver, here.join("settings")?.as_str()).await?;
        enter_hub(&driver).await?;
        click(&driver, "[data-sign-out-open]").await?;
        driver.enter_default_frame().await?;
        let before_sign_out = driver
            .execute("return performance.timeOrigin", Vec::new())
            .await?
            .json()
            .clone();
        enter_hub(&driver).await?;
        click(&driver, "[data-sign-out-submit]").await?;
        wait_for_top_reload(&driver, &before_sign_out, "sign-out").await?;
        let made = create_space(&driver, MADE).await?;

        sign_in_through(&driver, &env, &home).await?;
        // The page it landed on, as it landed: no reload by the test.
        let landed = get_json(&driver, "/api/profile").await?;
        let listed: Vec<String> =
            successful_body("the Hub after signing back in", &landed)["space"]
                .as_array()
                .map(|spaces| {
                    spaces
                        .iter()
                        .filter_map(|space| space["key"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
        assert!(
            listed.contains(&kept),
            "signing back in did not land on the account's spaces: {listed:?}"
        );
        assert!(
            listed.contains(&made),
            "the space made while signed out did not join the account: {listed:?}"
        );

        // Under the account: it syncs with the account's deployment, which
        // accepts it.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let info = get_json(&driver, &format!("/api/repository/{made}")).await?;
            let info = successful_body("read the space made while signed out", &info);
            let remote = info["remote"]["origin"].to_string();
            if remote.contains(env.tonk_web.join("ucan/")?.as_str()) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                dump_browser_log(&driver, &env).await;
                anyhow::bail!(
                    "the space made while signed out never moved to the account's deployment: {info}"
                );
            }
            let _ = post_json(&driver, "/api/sync", serde_json::json!({})).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        push_until_accepted(&driver, &made).await?;

        // Nothing is left on a signed-out workspace to switch back to.
        let roster = get_json(&driver, "/api/profiles").await?;
        let roster = successful_body("profiles after signing back in", &roster).clone();
        let others: Vec<_> = roster["profiles"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry["active"].as_bool() != Some(true))
            .collect();
        assert!(
            others.is_empty(),
            "an emptied signed-out workspace is still listed: {others:?}"
        );
        driver.quit().await?;
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
        let pulled = post_json(
            &guest,
            &format!("/api/repository/{key}/branch/main/sync/pull"),
            serde_json::json!({}),
        )
        .await?;
        successful_body("guest pulls before revocation", &pulled);

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

    /// A device linked to an account without the account's encryption key
    /// still takes a new space into the account's custody: the space's key
    /// is sealed to the account's own DID, so nothing asks the page for a
    /// passkey assertion to learn a key first.
    ///
    /// Signing in through another deployment is such a link: the grant
    /// arrives with no ceremony here that held the account's secret, so
    /// the root is recorded without the key. A link made by a page from
    /// before the key existed has the same shape.
    #[dialog_common::test(sibling = true)]
    async fn it_takes_a_new_space_into_custody_on_a_device_linked_without_a_key(
        env: TestEnvironment,
    ) -> Result<()> {
        let creator = driver_with_prf(&env).await?;
        sign_up(&creator, &env, EMAIL).await?;
        let home = env.tonk_web.origin().ascii_serialization();
        sign_in_through(&creator, &env, &home).await?;

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
        let branch = active_branch(&creator).await?;
        let created = post_json(
            &creator,
            &format!("/api/repository/profile:tonk/branch/{branch}/transact"),
            serde_json::json!({
                "claims": [{
                    "op": "assert",
                    "application": {
                        "predicate": {
                            "kind": "transient",
                            "concept": {
                                "description": "A request to create a new space from the wizard form.",
                                "with": {
                                    "name":   { "the": "xyz.tonk.command.create-space/name", "as": "Text" },
                                    "remote": { "the": "xyz.tonk.command.create-space/remote", "as": "Text" }
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

        // Custody needs no key the device lacks, so the create completes
        // without raising the passkey card.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let key = loop {
            anyhow::ensure!(
                creator
                    .find(By::Css("#tonk-custody-continue"))
                    .await
                    .is_err(),
                "the create asked the page for a passkey assertion"
            );
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
                "the create never recorded a space"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };

        // The new space's key is held for the account: a principal row
        // naming the space, whose `seed` points at the sealed message
        // carrying its key, whose `to` is the account's root DID. The
        // facts follow the create, so poll for them rather than assert on
        // the first read.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut sealed: Vec<serde_json::Value> = Vec::new();
        let recipient = loop {
            let principals = post_json(
                &creator,
                &format!("/api/repository/profile:tonk/branch/{branch}/query"),
                serde_json::json!({
                    "terms": {
                        "this": { "?": { "name": "this" } },
                        "seed": { "?": { "name": "seed" } }
                    },
                    "predicate": {
                        "with": {
                            "seed": { "the": "dialog.secret/seed", "cardinality": "one", "as": "Entity" }
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
                    &format!("/api/repository/profile:tonk/branch/{branch}/query"),
                    serde_json::json!({
                        "terms": {
                            "this": { "?": { "name": "this" } },
                            "to": { "?": { "name": "to" } }
                        },
                        "predicate": {
                            "with": {
                                "to": { "the": "dialog.secret/to", "cardinality": "one", "as": "Entity" }
                            }
                        }
                    }),
                )
                .await?;
                let messages = messages["body"].as_array().cloned().unwrap_or_default();
                sealed = messages.clone();
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
                "the new space's seed was never custodied: principals={principals:?} sealed={sealed:?}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        assert_eq!(
            recipient,
            root["rootDid"]
                .as_str()
                .context("root status omitted rootDid")?,
            "the space's key is held for the account"
        );

        creator.quit().await?;
        Ok(())
    }
}
