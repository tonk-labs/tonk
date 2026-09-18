//! Frozen linked-account fixture for returning-profile S2 measurements.
//!
//! Provisioning and browser restart are fixture setup. A later sampler owns
//! timing and must begin only before [`navigate_s2_returning`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;

use crate::account_flow::{S2AccountFixture, provision_s2_account_fixture};
use crate::helpers::{TestEnvironment, TestServers, add_prf_virtual_authenticator, goto};

pub(crate) const S2_SCENARIO: &str = "S2";
pub(crate) const S2_RECIPE: &str = "linked-active-returning-local-browser-restart-v1";
pub(crate) const S2_CACHE_STATE: &str =
    "same-profile-browser-process-cold-worker-cold-http-disk-cache-warm";
pub(crate) const S2_ROUTE_READY_SELECTOR: &str = "[data-spaces-view] a.srow";
pub(crate) const S2_INPUT_SEQUENCE: &str =
    "provision-linked-profile; quit-browser; restart-same-profile; navigate-root";

pub(crate) const S2_DATASET_CANONICAL_JSON: &str = r##"{"account":{"display_name":"Tab Owner","state":"active-linked-disposable","virtual_authenticator":"ctap2_1-prf-setup-only"},"cache_state":"same-profile-browser-process-cold-worker-cold-http-disk-cache-warm","dataset":{"marker_attribute":"xyz.tonk.perf/s2-returning-local-v1","marker_bookmark":"s2-returning-local-v1","space_count":2,"space_name":"S2 Returning Local","welcome_name":"Welcome to Tonk"},"input_sequence":["provision-linked-profile","quit-browser","restart-same-profile","navigate-root"],"recipe":"linked-active-returning-local-browser-restart-v1","route":"/","schema_version":1,"selectors":{"account_label":"[data-account-label]","account_trigger":"[data-account-trigger]","frame":"tonk-site > iframe","hub":".hub-page","space_label":":scope > .n","space_row":"[data-spaces-view] a.srow[href='/space/{runtime-space-key}']","spaces":"[data-spaces-view]"}}"##;

/// SHA-256 of [`S2_DATASET_CANONICAL_JSON`].
pub(crate) const S2_DATASET_SHA256: &str =
    "e7a0a47f81c645773038db6a7405eb71e43551f09c462d9c395cd82e98a06bce";

pub(crate) fn s2_request_fixture() -> Value {
    json!({
        "cache_state": S2_CACHE_STATE,
        "dataset_sha256": S2_DATASET_SHA256,
        "input_sequence": S2_INPUT_SEQUENCE,
        "recipe": S2_RECIPE,
        "selector": S2_ROUTE_READY_SELECTOR,
    })
}

pub(crate) fn validate_s2_request(request: &Value) -> Result<()> {
    ensure!(request["schema_version"] == 1, "unsupported request schema");
    ensure!(request["scenario"] == S2_SCENARIO, "request is not S2");
    ensure!(
        request["fixture"] == s2_request_fixture(),
        "unsupported S2 fixture contract"
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct S2Fixture {
    pub(crate) profile_path: PathBuf,
    pub(crate) account: S2AccountFixture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct S2ReturningState {
    pub(crate) worker_started_at: u64,
}

fn retryable_dom_error(error: &thirtyfour::error::WebDriverErrorInner) -> bool {
    matches!(
        error,
        thirtyfour::error::WebDriverErrorInner::NoSuchElement(_)
            | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
            | thirtyfour::error::WebDriverErrorInner::NoSuchFrame(_)
    )
}

async fn first(driver: &WebDriver, selector: &str) -> Result<Option<WebElement>> {
    match driver.find_all(By::Css(selector.to_owned())).await {
        Ok(elements) => Ok(elements.into_iter().next()),
        Err(error) if retryable_dom_error(error.as_inner()) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn configure_fixture_browser(driver: &WebDriver) -> Result<()> {
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
            "mobile": false,
        }),
    )
    .await?;
    let viewport = driver
        .execute("return [innerWidth, innerHeight]", vec![])
        .await?;
    ensure!(
        viewport.json() == &json!([1280, 800]),
        "S2 fixture desktop viewport override was not applied"
    );
    Ok(())
}

/// Provision the linked account and authored local marker, then shut down the
/// setup browser so the returned profile can be restarted cold.
pub(crate) async fn provision_s2_fixture(env: &TestEnvironment) -> Result<S2Fixture> {
    let profile_path = tempfile::Builder::new()
        .prefix("perf-s2-")
        .tempdir_in(&env.browser_profile_root)?
        .keep();
    let driver = env.blank_driver_with_profile(&profile_path).await?;
    let provisioned: Result<S2AccountFixture> = async {
        configure_fixture_browser(&driver).await?;
        add_prf_virtual_authenticator(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        provision_s2_account_fixture(&driver, env).await
    }
    .await;
    let quit = driver.quit().await;
    let account = provisioned?;
    quit?;
    Ok(S2Fixture {
        profile_path,
        account,
    })
}

/// Start a new Chrome process on the provisioned profile without visiting the
/// app or installing another authenticator.
pub(crate) async fn restart_s2_driver(
    env: &TestEnvironment,
    fixture: &S2Fixture,
) -> Result<WebDriver> {
    let driver = env.blank_driver_with_profile(&fixture.profile_path).await?;
    if let Err(error) = configure_fixture_browser(&driver).await {
        let _ = driver.quit().await;
        return Err(error);
    }
    Ok(driver)
}

async fn observe_returning_hub(driver: &WebDriver, fixture: &S2Fixture) -> Result<Option<Value>> {
    driver.enter_default_frame().await?;
    let Some(frame) = first(driver, "tonk-site > iframe").await? else {
        return Ok(None);
    };
    match frame.is_displayed().await {
        Ok(true) => {}
        Ok(false) => return Ok(None),
        Err(error) if retryable_dom_error(error.as_inner()) => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    match frame.enter_frame().await {
        Ok(()) => {}
        Err(error) if retryable_dom_error(error.as_inner()) => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let state = driver
        .execute(
            r#"
            const expectedKey = arguments[0];
            const visible = element => !!element && element.getClientRects().length > 0
              && getComputedStyle(element).display !== 'none'
              && getComputedStyle(element).visibility !== 'hidden';
            const hub = document.querySelector('.hub-page');
            const trigger = document.querySelector('[data-account-trigger]');
            const label = trigger?.querySelector('[data-account-label]');
            const spaces = document.querySelector('[data-spaces-view]');
            const row = spaces?.querySelector(`a.srow[href="/space/${expectedKey}"]`);
            const rowLabel = row?.querySelector(':scope > .n');
            return {
              hub_present: !!hub,
              hub_displayed: visible(hub),
              account_trigger_present: !!trigger,
              account_trigger_displayed: visible(trigger),
              account_label_present: !!label,
              account_label_displayed: visible(label),
              account_label_matches: (label?.textContent || '').trim() === 'Tab Owner',
              spaces_present: !!spaces,
              spaces_displayed: visible(spaces),
              fixture_row_present: !!row,
              fixture_row_displayed: visible(row),
              fixture_label_present: !!rowLabel,
              fixture_label_matches: (rowLabel?.textContent || '').trim() === 'S2 Returning Local'
            };
            "#,
            vec![json!(fixture.account.space_key)],
        )
        .await?
        .json()
        .clone();
    Ok(Some(state))
}

fn returning_hub_ready(state: &Value) -> bool {
    [
        "hub_present",
        "hub_displayed",
        "account_trigger_present",
        "account_trigger_displayed",
        "account_label_present",
        "account_label_displayed",
        "account_label_matches",
        "spaces_present",
        "spaces_displayed",
        "fixture_row_present",
        "fixture_row_displayed",
        "fixture_label_present",
        "fixture_label_matches",
    ]
    .into_iter()
    .all(|field| state[field] == true)
}

/// Navigate without retry and stop at the displayed returning-Hub endpoint.
/// Account, worker, and repository readback deliberately happen separately.
pub(crate) async fn navigate_s2_returning_visible(
    driver: &WebDriver,
    env: &TestEnvironment,
    fixture: &S2Fixture,
) -> Result<()> {
    driver.goto(env.tonk_web.as_str()).await?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut last = json!({"available": false});
    loop {
        if let Some(state) = observe_returning_hub(driver, fixture).await? {
            if returning_hub_ready(&state) {
                break;
            }
            last = state;
        }
        if Instant::now() >= deadline {
            driver.enter_default_frame().await?;
            return Err(anyhow!(
                "S2 returning Hub never reached its exact local-content endpoint; categorical_state={last}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

/// Prove account, worker, and local-content correctness after the visible
/// endpoint. These reads must stay outside an S2 route measurement interval.
pub(crate) async fn verify_s2_returning(
    driver: &WebDriver,
    fixture: &S2Fixture,
) -> Result<S2ReturningState> {
    driver.enter_default_frame().await?;
    ensure!(
        first(driver, "#tonk-register").await?.is_none(),
        "S2 returning startup raised account registration"
    );
    ensure!(
        first(driver, "#tonk-custody-consent").await?.is_none(),
        "S2 returning startup requested a new passkey assertion"
    );
    let controlled = driver
        .execute("return !!navigator.serviceWorker?.controller", vec![])
        .await?;
    ensure!(
        controlled.json() == &json!(true),
        "S2 returning page is not service-worker controlled"
    );

    // All account/API reads happen after the visible readiness endpoint.
    let worker_started_at = fixture.account.verify_restored(driver).await?;
    ensure!(
        worker_started_at != fixture.account.worker_started_at,
        "S2 browser restart reused the setup worker"
    );
    Ok(S2ReturningState { worker_started_at })
}

/// Navigate without retry and prove the returning Hub plus linked local state.
pub(crate) async fn navigate_s2_returning(
    driver: &WebDriver,
    env: &TestEnvironment,
    fixture: &S2Fixture,
) -> Result<S2ReturningState> {
    navigate_s2_returning_visible(driver, env, fixture).await?;
    verify_s2_returning(driver, fixture).await
}

#[tokio::test]
#[ignore = "requires isolated Chrome and the local browser test servers"]
async fn linked_account_and_local_content_survive_cold_browser_restart() -> Result<()> {
    let (servers, env) = TestServers::start().await?;
    let tested: Result<()> = async {
        let fixture = provision_s2_fixture(&env).await?;
        let driver = restart_s2_driver(&env, &fixture).await?;
        let checked = navigate_s2_returning(&driver, &env, &fixture).await;
        let quit = driver.quit().await;
        checked?;
        quit?;
        Ok(())
    }
    .await;
    let cleanup = servers.stop().await;
    tested?;
    cleanup?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s2_contract_is_frozen() {
        assert_eq!(
            serde_json::from_str::<Value>(S2_DATASET_CANONICAL_JSON).unwrap(),
            json!({
                "account": {
                    "display_name": "Tab Owner",
                    "state": "active-linked-disposable",
                    "virtual_authenticator": "ctap2_1-prf-setup-only",
                },
                "cache_state": S2_CACHE_STATE,
                "dataset": {
                    "marker_attribute": "xyz.tonk.perf/s2-returning-local-v1",
                    "marker_bookmark": "s2-returning-local-v1",
                    "space_count": 2,
                    "space_name": "S2 Returning Local",
                    "welcome_name": "Welcome to Tonk",
                },
                "input_sequence": [
                    "provision-linked-profile",
                    "quit-browser",
                    "restart-same-profile",
                    "navigate-root",
                ],
                "recipe": S2_RECIPE,
                "route": "/",
                "schema_version": 1,
                "selectors": {
                    "account_label": "[data-account-label]",
                    "account_trigger": "[data-account-trigger]",
                    "frame": "tonk-site > iframe",
                    "hub": ".hub-page",
                    "space_label": ":scope > .n",
                    "space_row": "[data-spaces-view] a.srow[href='/space/{runtime-space-key}']",
                    "spaces": "[data-spaces-view]",
                },
            })
        );
        assert_eq!(
            tonk_analytics::distinct_id(S2_DATASET_CANONICAL_JSON)
                .strip_prefix("tonk:")
                .unwrap(),
            S2_DATASET_SHA256
        );
        let mut request = json!({
            "schema_version": 1,
            "scenario": S2_SCENARIO,
            "fixture": s2_request_fixture(),
        });
        validate_s2_request(&request).unwrap();
        request["fixture"]["cache_state"] = json!("memory-cache-warm");
        assert!(validate_s2_request(&request).is_err());
    }
}
