//! Frozen browser fixtures shared by UI performance scenarios.
//!
//! The fixture routines prepare and exercise product state only. Timing,
//! tracing, metric readiness, and sample persistence belong to the performance
//! runner so a DOM assertion cannot accidentally stand in for a measurement.

use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub(crate) const S3_SCENARIO: &str = "S3";
pub(crate) const S3_RECIPE: &str = "fresh-anonymous-seeded-welcome-direct-space-v1";
pub(crate) const S3_CACHE_STATE: &str =
    "same-session-after-fixture-provisioning-http-and-worker-warm";
pub(crate) const S3_ROUTE_READY_SELECTOR: &str = "tonk-fab[space]";
pub(crate) const S3_SPACE_CONTENTS_SELECTOR: &str =
    "#fabb-space-menu:not([hidden]) > tonk-mi[data-mi-open]";
pub(crate) const S3_SPACE_SUBSCRIPTION_SELECTOR: &str =
    "#fabb-space-menu tonk-mi[data-space][current]";
pub(crate) const S3_SHARE_CONTENTS_SELECTOR: &str =
    "#fabb-share-menu:not([hidden]) > tonk-mi[data-share-account]:not([hidden])";
pub(crate) const S3_INPUT_SEQUENCE: &str = "navigate-direct-space-link; webdriver-click-space; \
webdriver-escape; webdriver-click-space; webdriver-escape; webdriver-click-share; \
webdriver-escape; webdriver-click-share";

/// Canonical, content-free description of the disposable dataset. Runtime
/// subjects are deliberately excluded: each fresh profile creates a new DID,
/// while the authored seed and the interactions remain identical.
pub(crate) const S3_DATASET_CANONICAL_JSON: &str = r##"{"account_state":"local-anonymous-unregistered","cache_state":"same-session-after-fixture-provisioning-http-and-worker-warm","dataset":{"space_count":1,"space_name":"Welcome to Tonk","source":"built-in-profile-seed"},"input_sequence":["navigate-direct-space-link","webdriver-click-space","webdriver-escape","webdriver-click-space","webdriver-escape","webdriver-click-share","webdriver-escape","webdriver-click-share"],"menu_labels":{"share_primary":"log in to share","share_roster":"1 member","space_primary":"open"},"recipe":"fresh-anonymous-seeded-welcome-direct-space-v1","route_pattern":"/space/{runtime-space-subject}","schema_version":1,"selectors":{"frame":"tonk-site > iframe","route_ready":"tonk-fab[space]","share_contents":"#fabb-share-menu:not([hidden]) > tonk-mi[data-share-account]:not([hidden])","share_opener":"[data-cell=share]","space_contents":"#fabb-space-menu:not([hidden]) > tonk-mi[data-mi-open]","space_opener":"[data-cell=space]","space_subscription":"#fabb-space-menu tonk-mi[data-space][current]"}}"##;

/// SHA-256 of [`S3_DATASET_CANONICAL_JSON`]. Changing any fixture behavior
/// requires an explicit digest update before samples remain comparable.
pub(crate) const S3_DATASET_SHA256: &str =
    "9fb1a62555411a85b6a7ef4cf7471b746a4ed18e45e1465119c51664da9bf815";

/// The request fragment the orchestrator must freeze for S3.
pub(crate) fn s3_request_fixture() -> Value {
    json!({
        "cache_state": S3_CACHE_STATE,
        "dataset_sha256": S3_DATASET_SHA256,
        "input_sequence": S3_INPUT_SEQUENCE,
        "recipe": S3_RECIPE,
        "selector": S3_ROUTE_READY_SELECTOR,
    })
}

/// Reject a request that silently substitutes a different S3 fixture.
pub(crate) fn validate_s3_request(request: &Value) -> Result<()> {
    ensure!(request["schema_version"] == 1, "unsupported request schema");
    ensure!(request["scenario"] == S3_SCENARIO, "request is not S3");
    ensure!(
        request["fixture"] == s3_request_fixture(),
        "unsupported S3 fixture contract; update and revalidate the runner for fixture changes"
    );
    Ok(())
}

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod browser {
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow, ensure};
    use serde_json::{Value, json};
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::prelude::*;
    use url::Url;

    use crate::helpers::{TestEnvironment, TestServers, goto};

    use super::{
        S3_ROUTE_READY_SELECTOR, S3_SHARE_CONTENTS_SELECTOR, S3_SPACE_CONTENTS_SELECTOR,
        S3_SPACE_SUBSCRIPTION_SELECTOR,
    };

    const FRAME_SELECTOR: &str = "tonk-site > iframe";
    const SPACE_MENU_SELECTOR: &str = "#fabb-space-menu";
    const SHARE_MENU_SELECTOR: &str = "#fabb-share-menu";
    const MENU_WAIT: Duration = Duration::from_secs(30);

    /// The runtime-only address of the frozen semantic fixture.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct S3Fixture {
        pub(crate) route_key: String,
        pub(crate) space_subject: String,
        pub(crate) direct_url: Url,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum S3Menu {
        Space,
        Share,
    }

    impl S3Menu {
        fn opener_selector(self) -> &'static str {
            match self {
                Self::Space => "[data-cell=space]",
                Self::Share => "[data-cell=share]",
            }
        }

        fn menu_selector(self) -> &'static str {
            match self {
                Self::Space => SPACE_MENU_SELECTOR,
                Self::Share => SHARE_MENU_SELECTOR,
            }
        }

        fn diagnostic_name(self) -> &'static str {
            match self {
                Self::Space => "space",
                Self::Share => "share",
            }
        }
    }

    fn retryable_dom_error(error: &thirtyfour::error::WebDriverErrorInner) -> bool {
        matches!(
            error,
            thirtyfour::error::WebDriverErrorInner::NoSuchElement(_)
                | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
        )
    }

    async fn find_if_ready(
        driver: &WebDriver,
        selector: &'static str,
    ) -> Result<Option<WebElement>> {
        match driver.find(By::Css(selector)).await {
            Ok(element) => Ok(Some(element)),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn displayed_if_ready(element: &WebElement) -> Result<Option<bool>> {
        match element.is_displayed().await {
            Ok(displayed) => Ok(Some(displayed)),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn enter_frame_if_ready(frame: WebElement) -> Result<bool> {
        match frame.enter_frame().await {
            Ok(()) => Ok(true),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    async fn text_if_ready(element: &WebElement) -> Result<Option<String>> {
        match element.text().await {
            Ok(text) => Ok(Some(text)),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Read authored light-DOM text even while its custom-element row sits in
    /// a closed flyout. Rendered text is correctly empty for that row, while
    /// S3 needs to know whether the switcher subscription populated it.
    async fn light_text_if_ready(element: &WebElement) -> Result<Option<String>> {
        match element.prop("textContent").await {
            Ok(text) => Ok(text.map(|text| text.trim().to_owned())),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// The primary label is the direct authored text node. Descendant flyout
    /// text can become rendered when focus opens it, so host `.text()` is not
    /// stable across otherwise identical first and repeated menu opens.
    async fn direct_label_if_ready(
        driver: &WebDriver,
        element: &WebElement,
    ) -> Result<Option<String>> {
        let result = driver
            .execute(
                r#"
                return Array.from(arguments[0].childNodes)
                    .filter(node => node.nodeType === 3)
                    .map(node => node.textContent || "")
                    .join(" ")
                    .trim();
                "#,
                vec![element.to_json()?],
            )
            .await;
        match result {
            Ok(value) => Ok(value.json().as_str().map(str::to_owned)),
            Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Evidence that the menu's asynchronous product content, rather than
    /// merely its container, was ready after the trusted input.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct S3MenuContents {
        pub(crate) menu: S3Menu,
        pub(crate) dynamic_rows: usize,
        pub(crate) primary_label: String,
    }

    async fn enter_space_shell(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        let frame = driver
            .find(By::Css(FRAME_SELECTOR))
            .await
            .context("space shell frame did not mount")?;
        ensure!(frame.is_displayed().await?, "space shell frame is hidden");
        frame.enter_frame().await?;
        Ok(())
    }

    async fn get_json(driver: &WebDriver, path: &str) -> Result<Value> {
        driver.enter_default_frame().await?;
        let result = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                fetch(arguments[0]).then(async response => {
                    const text = await response.text();
                    let body;
                    try { body = text ? JSON.parse(text) : null; }
                    catch (_) { body = { raw: text }; }
                    done({ status: response.status, body });
                }).catch(error => done({ error: String(error) }));
                "##,
                vec![json!(path)],
            )
            .await?;
        let value = result.json().clone();
        ensure!(
            value.get("error").is_none(),
            "fixture request failed: {value}"
        );
        ensure!(value["status"] == 200, "fixture request failed: {value}");
        Ok(value["body"].clone())
    }

    /// Materialize the repository's authored Welcome seed in a disposable
    /// anonymous profile and resolve its runtime-only direct link.
    pub(crate) async fn provision_s3_fixture(
        driver: &WebDriver,
        env: &TestEnvironment,
    ) -> Result<S3Fixture> {
        goto(driver, env.tonk_web.as_str()).await?;
        let deadline = tokio::time::Instant::now() + MENU_WAIT;
        let (route_key, space_subject) = loop {
            let profile = get_json(driver, "/api/profile").await?;
            let spaces = profile["space"].as_array();
            if let Some([only]) = spaces.map(Vec::as_slice)
                && let (Some(key), Some(subject)) = (only["key"].as_str(), only["subject"].as_str())
            {
                break (key.to_owned(), subject.to_owned());
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "fresh anonymous profile did not settle to exactly one seeded Welcome space: {profile}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let direct_url = env
            .tonk_web
            .join(&format!("space/{route_key}"))
            .context("could not construct the S3 direct space link")?;
        verify_seeded_welcome(driver).await?;
        Ok(S3Fixture {
            route_key,
            space_subject,
            direct_url,
        })
    }

    async fn observe_welcome_headline(driver: &WebDriver) -> Result<Option<String>> {
        driver.enter_default_frame().await?;
        let Some(shell) = find_if_ready(driver, FRAME_SELECTOR).await? else {
            return Ok(None);
        };
        if displayed_if_ready(&shell).await? != Some(true) || !enter_frame_if_ready(shell).await? {
            return Ok(None);
        }
        let Some(content) = find_if_ready(driver, FRAME_SELECTOR).await? else {
            return Ok(None);
        };
        if displayed_if_ready(&content).await? != Some(true)
            || !enter_frame_if_ready(content).await?
        {
            return Ok(None);
        }
        let Some(headline) = find_if_ready(driver, ".wp-h1").await? else {
            return Ok(None);
        };
        if displayed_if_ready(&headline).await? != Some(true) {
            return Ok(None);
        }
        text_if_ready(&headline).await
    }

    async fn verify_seeded_welcome(driver: &WebDriver) -> Result<()> {
        let deadline = tokio::time::Instant::now() + MENU_WAIT;
        loop {
            if let Some(headline) = observe_welcome_headline(driver).await? {
                ensure!(
                    headline.contains("makes your small software"),
                    "the profile's sole space is not the authored Welcome seed: {headline:?}"
                );
                driver.enter_default_frame().await?;
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                driver.enter_default_frame().await?;
                return Err(anyhow!(
                    "the profile's sole space never rendered the authored Welcome headline"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Navigate through the fixture's real direct link and wait only for the
    /// route controls required by S3. The performance runner separately owns
    /// clocks and the navigation-to-readiness metric.
    pub(crate) async fn navigate_s3_direct(driver: &WebDriver, fixture: &S3Fixture) -> Result<()> {
        // This is the scenario action, so a failed first navigation must stay
        // failed. The shared integration helper's one-shot retry is suitable
        // for fixture setup only.
        driver.goto(fixture.direct_url.as_str()).await?;
        let deadline = tokio::time::Instant::now() + MENU_WAIT;
        loop {
            driver.enter_default_frame().await?;
            let Some(frame) = find_if_ready(driver, FRAME_SELECTOR).await? else {
                continue_after_route_poll(deadline, driver, "space shell frame is absent").await?;
                continue;
            };
            if displayed_if_ready(&frame).await? != Some(true) {
                continue_after_route_poll(deadline, driver, "space shell frame is not displayed")
                    .await?;
                continue;
            }
            match frame.enter_frame().await {
                Ok(()) => {}
                Err(error) if retryable_dom_error(error.as_inner()) => {
                    continue_after_route_poll(deadline, driver, "space shell frame went stale")
                        .await?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            let Some(bar) = find_if_ready(driver, S3_ROUTE_READY_SELECTOR).await? else {
                continue_after_route_poll(deadline, driver, "bar is absent").await?;
                continue;
            };
            if displayed_if_ready(&bar).await? != Some(true) {
                continue_after_route_poll(deadline, driver, "bar is hidden").await?;
                continue;
            }
            let subject = match bar.attr("space").await {
                Ok(subject) => subject.unwrap_or_default(),
                Err(error) if retryable_dom_error(error.as_inner()) => {
                    continue_after_route_poll(deadline, driver, "bar went stale").await?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if subject != fixture.space_subject {
                continue_after_route_poll(deadline, driver, "bar addresses a different space")
                    .await?;
                continue;
            }
            let Some(nested) = find_if_ready(driver, FRAME_SELECTOR).await? else {
                continue_after_route_poll(deadline, driver, "space content frame is absent")
                    .await?;
                continue;
            };
            if displayed_if_ready(&nested).await? == Some(true) {
                driver.enter_default_frame().await?;
                return Ok(());
            }
            continue_after_route_poll(deadline, driver, "space content frame is hidden").await?;
        }
    }

    async fn continue_after_route_poll(
        deadline: tokio::time::Instant,
        driver: &WebDriver,
        last: &str,
    ) -> Result<()> {
        if tokio::time::Instant::now() >= deadline {
            driver.enter_default_frame().await?;
            return Err(anyhow!(
                "direct S3 route never exposed usable controls: {last}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    /// Open a FABB menu through WebDriver's element-click endpoint and wait
    /// for its live, scenario-specific contents. WebDriver clicks are trusted
    /// browser input; this deliberately does not call DOM `click()` or
    /// `dispatchEvent()`.
    pub(crate) async fn open_s3_menu(
        driver: &WebDriver,
        fixture: &S3Fixture,
        menu: S3Menu,
    ) -> Result<S3MenuContents> {
        enter_space_shell(driver).await?;
        let bar = driver.find(By::Css(S3_ROUTE_READY_SELECTOR)).await?;
        let shadow = bar
            .get_shadow_root()
            .await
            .context("tonk-fab has no open shadow root")?;
        let opener = shadow
            .find(By::Css(menu.opener_selector()))
            .await
            .context("FABB menu opener is absent")?;
        if let Err(error) = opener.click().await {
            let (diagnostic, screenshot) = menu_timeout_diagnostic(driver, menu).await;
            return Err(error).context(format!(
                "S3 {menu:?} trusted menu click failed [failure_category=menu_click]; \
                 diagnostic={diagnostic}; root_screenshot={screenshot}"
            ));
        }

        let deadline = tokio::time::Instant::now() + MENU_WAIT;
        loop {
            match inspect_open_menu(driver, fixture, menu).await {
                Ok(Some(contents)) => return Ok(contents),
                Ok(None) => {}
                Err(error) => return Err(error),
            }
            if tokio::time::Instant::now() >= deadline {
                let (diagnostic, screenshot) = menu_timeout_diagnostic(driver, menu).await;
                return Err(anyhow!(
                    "S3 {menu:?} live menu contents never became ready; diagnostic={diagnostic}; \
                     root_screenshot={screenshot}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn inspect_open_menu(
        driver: &WebDriver,
        fixture: &S3Fixture,
        menu: S3Menu,
    ) -> Result<Option<S3MenuContents>> {
        let Some(container) = find_if_ready(driver, menu.menu_selector()).await? else {
            return Ok(None);
        };
        if displayed_if_ready(&container).await? != Some(true) {
            return Ok(None);
        }
        match container.attr("hidden").await {
            Ok(None) => {}
            Ok(Some(_)) => return Ok(None),
            Err(error) if retryable_dom_error(error.as_inner()) => return Ok(None),
            Err(error) => return Err(error.into()),
        }

        match menu {
            S3Menu::Space => {
                let Some(open) = find_if_ready(driver, S3_SPACE_CONTENTS_SELECTOR).await? else {
                    return Ok(None);
                };
                if displayed_if_ready(&open).await? != Some(true) {
                    return Ok(None);
                }
                let Some(open_label) = direct_label_if_ready(driver, &open).await? else {
                    return Ok(None);
                };
                ensure!(
                    open_label == "open",
                    "visible space-menu action changed: {open_label:?}"
                );
                let Some(current) = find_if_ready(driver, S3_SPACE_SUBSCRIPTION_SELECTOR).await?
                else {
                    return Ok(None);
                };
                let subject = match current.attr("data-space").await {
                    Ok(Some(subject)) => subject,
                    Ok(None) => return Ok(None),
                    Err(error) if retryable_dom_error(error.as_inner()) => return Ok(None),
                    Err(error) => return Err(error.into()),
                };
                ensure!(
                    subject == fixture.space_subject,
                    "space switcher current row addresses the wrong fixture"
                );
                let rows = match container.find_all(By::Css("tonk-mi[data-space]")).await {
                    Ok(rows) => rows.len(),
                    Err(error) if retryable_dom_error(error.as_inner()) => return Ok(None),
                    Err(error) => return Err(error.into()),
                };
                if rows == 0 {
                    return Ok(None);
                }
                ensure!(
                    rows == 1,
                    "fixture must render exactly one space row, got {rows}"
                );
                let Some(label) = light_text_if_ready(&current).await? else {
                    return Ok(None);
                };
                if label.is_empty() {
                    return Ok(None);
                }
                ensure!(
                    label == "Welcome to Tonk",
                    "seeded space label changed: {label:?}"
                );
                Ok(Some(S3MenuContents {
                    menu,
                    dynamic_rows: rows,
                    primary_label: open_label,
                }))
            }
            S3Menu::Share => {
                let Some(account) = find_if_ready(driver, S3_SHARE_CONTENTS_SELECTOR).await? else {
                    return Ok(None);
                };
                if displayed_if_ready(&account).await? != Some(true) {
                    return Ok(None);
                }
                let Some(account_label) = direct_label_if_ready(driver, &account).await? else {
                    return Ok(None);
                };
                if account_label.is_empty() {
                    return Ok(None);
                }
                ensure!(
                    account_label == "log in to share",
                    "anonymous share action changed: {account_label:?}"
                );
                let Some(roster) = find_if_ready(driver, "tonk-mi[data-share-members]").await?
                else {
                    return Ok(None);
                };
                let Some(roster_label) = text_if_ready(&roster).await? else {
                    return Ok(None);
                };
                if roster_label.is_empty() {
                    return Ok(None);
                }
                ensure!(
                    roster_label == "1 member",
                    "fixture roster did not settle to one member: {roster_label:?}"
                );
                Ok(Some(S3MenuContents {
                    menu,
                    dynamic_rows: 1,
                    primary_label: account_label,
                }))
            }
        }
    }

    /// Bounded and content-free: enough state to separate a missed trusted
    /// click from an open panel whose live selector never populated.
    async fn menu_timeout_diagnostic(driver: &WebDriver, menu: S3Menu) -> (Value, String) {
        let diagnostic = driver
            .execute(
                r##"
                const kind = arguments[0];
                const bar = document.querySelector("tonk-fab");
                const cell = bar?.shadowRoot?.querySelector(`[data-cell="${kind}"]`);
                const selector = kind === "space" ? "#fabb-space-menu" : "#fabb-share-menu";
                const panel = bar?.querySelector(selector);
                const rect = panel?.getBoundingClientRect();
                return {
                    bar_present: !!bar,
                    opener_present: !!cell,
                    opener_expanded: cell?.getAttribute("aria-expanded") === "true",
                    menu_present: !!panel,
                    menu_hidden: panel?.hasAttribute("hidden") ?? null,
                    menu_has_box: !!rect && rect.width > 0 && rect.height > 0,
                    current_space_rows: panel?.querySelectorAll("tonk-mi[data-space][current]").length ?? 0,
                    space_rows: panel?.querySelectorAll("tonk-mi[data-space]").length ?? 0,
                    account_rows: panel?.querySelectorAll("tonk-mi[data-share-account]:not([hidden])").length ?? 0,
                    roster_rows: panel?.querySelectorAll("tonk-mi[data-share-members]").length ?? 0
                };
                "##,
                vec![json!(menu.diagnostic_name())],
            )
            .await
            .map(|value| value.json().clone())
            .unwrap_or_else(|_| json!({ "available": false }));

        let _ = driver.enter_default_frame().await;
        let screenshot = std::env::var_os("TONK_PERF_FAILURE_ROOT")
            .map(std::path::PathBuf::from)
            .and_then(|root| {
                std::fs::create_dir_all(&root).ok()?;
                Some(root.join(format!("s3-{}-timeout.png", menu.diagnostic_name())))
            });
        let screenshot = match screenshot {
            Some(path) if driver.screenshot(&path).await.is_ok() => path.display().to_string(),
            Some(_) => "capture-failed".to_owned(),
            None => "unavailable: TONK_PERF_FAILURE_ROOT not set".to_owned(),
        };
        (diagnostic, screenshot)
    }

    /// Close the current menu with a real keyboard action and prove the same
    /// panel closed before a repeated-open sample begins.
    pub(crate) async fn close_s3_menu(driver: &WebDriver, menu: S3Menu) -> Result<()> {
        driver
            .action_chain()
            .send_keys(Key::Escape)
            .perform()
            .await?;
        let deadline = tokio::time::Instant::now() + MENU_WAIT;
        loop {
            let closed = match find_if_ready(driver, menu.menu_selector()).await? {
                Some(container) => match container.attr("hidden").await {
                    Ok(hidden) => hidden.is_some(),
                    Err(error) if retryable_dom_error(error.as_inner()) => false,
                    Err(error) => return Err(error.into()),
                },
                None => false,
            };
            if closed {
                driver.enter_default_frame().await?;
                return Ok(());
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "S3 {menu:?} menu did not close after trusted Escape input"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    #[ignore = "requires the isolated native browser test environment"]
    async fn s3_fixture_reaches_first_and_repeated_real_menu_contents() -> Result<()> {
        let (servers, env) = TestServers::start().await?;
        let driver = match env.blank_driver().await {
            Ok(driver) => driver,
            Err(error) => {
                servers.stop().await?;
                return Err(error);
            }
        };
        let tested: Result<()> = async {
            let cdp = ChromeDevTools::new(driver.handle.clone());
            cdp.execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({
                    "source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"
                }),
            )
            .await?;
            cdp.execute_cdp_with_params(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": 1280,
                    "height": 800,
                    "deviceScaleFactor": 1,
                    "mobile": false
                }),
            )
            .await?;
            let viewport = driver
                .execute("return [innerWidth, innerHeight]", Vec::new())
                .await?;
            ensure!(
                viewport.json() == &json!([1280, 800]),
                "S3 desktop viewport was not applied: {}",
                viewport.json()
            );
            let fixture = provision_s3_fixture(&driver, &env).await?;
            navigate_s3_direct(&driver, &fixture).await?;

            for menu in [S3Menu::Space, S3Menu::Share] {
                let first = open_s3_menu(&driver, &fixture, menu).await?;
                ensure!(first.dynamic_rows == 1, "fixture content changed");
                close_s3_menu(&driver, menu).await?;

                let repeated = open_s3_menu(&driver, &fixture, menu).await?;
                ensure!(repeated.menu == first.menu, "repeated menu kind drifted");
                ensure!(
                    repeated.dynamic_rows == first.dynamic_rows,
                    "repeated {menu:?} menu row-count drifted: first={} repeated={}",
                    first.dynamic_rows,
                    repeated.dynamic_rows
                );
                ensure!(
                    repeated.primary_label == first.primary_label,
                    "repeated {menu:?} menu primary-label match failed"
                );
                if menu == S3Menu::Space {
                    close_s3_menu(&driver, menu).await?;
                }
            }
            Ok(())
        }
        .await;
        let quit = driver.quit().await;
        let cleanup = servers.stop().await;
        tested?;
        quit?;
        cleanup?;
        Ok(())
    }
}

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
#[allow(unused_imports)] // Public fixture surface; the scenario runner is wired separately.
pub(crate) use browser::{
    S3Fixture, S3Menu, S3MenuContents, close_s3_menu, navigate_s3_direct, open_s3_menu,
    provision_s3_fixture,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_contract_is_frozen() {
        assert_eq!(
            s3_request_fixture()["selector"],
            json!(S3_ROUTE_READY_SELECTOR)
        );
        assert_eq!(
            serde_json::from_str::<Value>(S3_DATASET_CANONICAL_JSON).unwrap(),
            json!({
                "account_state": "local-anonymous-unregistered",
                "cache_state": S3_CACHE_STATE,
                "dataset": {
                    "space_count": 1,
                    "space_name": "Welcome to Tonk",
                    "source": "built-in-profile-seed",
                },
                "input_sequence": [
                    "navigate-direct-space-link",
                    "webdriver-click-space",
                    "webdriver-escape",
                    "webdriver-click-space",
                    "webdriver-escape",
                    "webdriver-click-share",
                    "webdriver-escape",
                    "webdriver-click-share",
                ],
                "menu_labels": {
                    "share_primary": "log in to share",
                    "share_roster": "1 member",
                    "space_primary": "open",
                },
                "recipe": S3_RECIPE,
                "route_pattern": "/space/{runtime-space-subject}",
                "schema_version": 1,
                "selectors": {
                    "frame": "tonk-site > iframe",
                    "route_ready": S3_ROUTE_READY_SELECTOR,
                    "share_contents": S3_SHARE_CONTENTS_SELECTOR,
                    "share_opener": "[data-cell=share]",
                    "space_contents": S3_SPACE_CONTENTS_SELECTOR,
                    "space_opener": "[data-cell=space]",
                    "space_subscription": S3_SPACE_SUBSCRIPTION_SELECTOR,
                },
            })
        );
        assert_eq!(
            tonk_analytics::distinct_id(S3_DATASET_CANONICAL_JSON)
                .strip_prefix("tonk:")
                .unwrap(),
            S3_DATASET_SHA256
        );
    }

    #[test]
    fn s3_validation_rejects_fixture_drift() {
        let mut request = json!({
            "schema_version": 1,
            "scenario": S3_SCENARIO,
            "fixture": s3_request_fixture(),
        });
        validate_s3_request(&request).unwrap();

        request["fixture"]["input_sequence"] = json!("synthetic-click");
        assert!(validate_s3_request(&request).is_err());
    }
}
