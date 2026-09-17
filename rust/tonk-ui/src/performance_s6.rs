//! Foreground-edit sampler for the frozen twenty-repository S6 fixture.
//!
//! Provisioning, route setup, probe configuration and screenshots sit outside
//! the measured trusted-click interval. The local access-service probe shapes
//! and counts repository permit requests for either supplied release artifact.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;

use crate::account_flow::{S6AccountFixture, provision_s6_account_fixture};
use crate::helpers::{TestEnvironment, add_prf_virtual_authenticator, goto};
use crate::performance_fixtures::{S3Fixture, navigate_s3_direct};
use crate::performance_support::verify_artifact;

const WIDTH: u64 = 1280;
const HEIGHT: u64 = 800;
const PROBE_DELAY_MS: u64 = 75;
const S6_SCENARIO: &str = "S6";
const S6_RECIPE: &str = "linked-twenty-cached-repositories-foreground-edit-v1";
const S6_CACHE_STATE: &str = "same-session-linked-twenty-cached-repositories-active-view-warm";
const S6_INPUT_SEQUENCE: &str = "provision-linked-account-and-twenty-spaces; navigate-active-space; configure-inactive-delay; await-inactive-request; webdriver-click-one-entity-edit";
const S6_SELECTOR: &str = "#tonk-perf-s6-edit[data-state='complete']";
const S6_DATASET_CANONICAL_JSON: &str = r##"{"account_state":"active-linked-disposable","cache_state":"same-session-linked-twenty-cached-repositories-active-view-warm","dataset":{"active_remote_repositories":1,"delayed_inactive_remote_repositories":18,"total_cached_repositories":20,"unshaped_welcome_repositories":1},"inactive_permit_delay_ms":75,"input_sequence":["provision-linked-account-and-twenty-spaces","navigate-active-space","configure-inactive-delay","await-inactive-request","webdriver-click-one-entity-edit"],"recipe":"linked-twenty-cached-repositories-foreground-edit-v1","route_pattern":"/space/{runtime-active-key}","schema_version":1,"selector":"#tonk-perf-s6-edit[data-state='complete']"}"##;
const S6_DATASET_SHA256: &str = "0c5b249e45e4f411e1b80ef93d32bbed823ad0ca89f855f409f1602b59dd218b";

pub(crate) fn s6_request_fixture() -> Value {
    json!({
        "cache_state": S6_CACHE_STATE,
        "dataset_sha256": S6_DATASET_SHA256,
        "input_sequence": S6_INPUT_SEQUENCE,
        "recipe": S6_RECIPE,
        "selector": S6_SELECTOR,
    })
}

pub(crate) fn validate_request(request: &Value) -> Result<(u64, u64)> {
    ensure!(request["schema_version"] == 1, "unsupported request schema");
    ensure!(request["scenario"] == S6_SCENARIO, "request is not S6");
    ensure!(
        request["fixture"] == s6_request_fixture(),
        "unsupported S6 fixture contract"
    );
    ensure!(request["profile"] == "desktop", "S6 supports only desktop");
    ensure!(
        request["profile_settings"]
            == json!({
                "cpu_slowdown": 1,
                "latency_ms": 0,
                "download_bytes_per_second": -1,
                "upload_bytes_per_second": -1,
                "viewport_width": WIDTH,
                "viewport_height": HEIGHT,
            }),
        "S6 desktop profile settings differ from the frozen profile"
    );
    Ok((WIDTH, HEIGHT))
}

async fn configure_desktop(driver: &WebDriver) -> Result<Value> {
    let cdp = ChromeDevTools::new(driver.handle.clone());
    cdp.execute_cdp_with_params(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"}),
    )
    .await?;
    cdp.execute_cdp_with_params(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": WIDTH,
            "height": HEIGHT,
            "deviceScaleFactor": 1,
            "mobile": false,
        }),
    )
    .await?;
    cdp.execute_cdp_with_params("Emulation.setCPUThrottlingRate", json!({"rate": 1}))
        .await?;
    cdp.execute_cdp("Network.enable").await?;
    cdp.execute_cdp_with_params(
        "Network.emulateNetworkConditions",
        json!({
            "offline": false,
            "latency": 0,
            "downloadThroughput": -1,
            "uploadThroughput": -1,
        }),
    )
    .await?;
    cdp.execute_cdp("Browser.getVersion")
        .await
        .map_err(Into::into)
}

async fn configure_probe(env: &TestEnvironment, fixture: &S6AccountFixture) -> Result<()> {
    let response = reqwest::Client::new()
        .post(env.access_service.join("_test/sync-probe")?)
        .json(&json!({
            "delay_ms": PROBE_DELAY_MS,
            "delayed_subjects": fixture.delayed_subjects,
        }))
        .send()
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::NO_CONTENT,
        "S6 sync probe configuration failed with {}",
        response.status()
    );
    Ok(())
}

async fn read_probe(env: &TestEnvironment) -> Result<Value> {
    Ok(reqwest::get(env.access_service.join("_test/sync-probe")?)
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn await_inactive_request(env: &TestEnvironment) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = read_probe(env).await?;
        if snapshot["delayed_event_count"].as_u64().unwrap_or_default() > 0 {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "S6 visible loop never attempted an inactive upstream request"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn install_edit_control(driver: &WebDriver, active_key: &str) -> Result<()> {
    driver.enter_default_frame().await?;
    let installed = driver
        .execute(
            r#"
            const key = arguments[0];
            document.querySelector('#tonk-perf-s6-edit')?.remove();
            const button = document.createElement('button');
            button.id = 'tonk-perf-s6-edit';
            button.type = 'button';
            button.textContent = 'apply performance edit';
            button.dataset.state = 'ready';
            Object.assign(button.style, {
              position: 'fixed', left: '24px', bottom: '24px', zIndex: '2147483647',
              padding: '12px 16px', background: '#fff', color: '#111', border: '2px solid #111'
            });
            button.addEventListener('click', async event => {
              button.dataset.trusted = String(event.isTrusted);
              button.dataset.state = 'pending';
              const started = performance.now();
              try {
                const response = await fetch(`/api/repository/${key}/branch/main/evaluate`, {
                  method: 'POST',
                  headers: {'content-type': 'text/yaml'},
                  body: 'attribute!: &s6-foreground-edit\n  the: xyz.tonk.perf/s6-foreground-edit\n  as: text\n  cardinality: one\n  description: S6 foreground edit marker\n'
                });
                const body = await response.json();
                button.dataset.status = String(response.status);
                button.dataset.claims = String(body?.commits?.claims ?? -1);
                button.dataset.elapsedMs = String(performance.now() - started);
                button.dataset.state = response.ok ? 'complete' : 'failed';
                button.textContent = response.ok ? 'edit complete' : 'edit failed';
              } catch (error) {
                button.dataset.error = String(error);
                button.dataset.elapsedMs = String(performance.now() - started);
                button.dataset.state = 'failed';
                button.textContent = 'edit failed';
              }
            });
            document.body.append(button);
            return button.dataset.state;
            "#,
            vec![json!(active_key)],
        )
        .await?;
    ensure!(
        installed.json() == "ready",
        "S6 edit control was not installed"
    );
    Ok(())
}

async fn perform_edit(driver: &WebDriver) -> Result<(f64, f64, u64)> {
    driver.enter_default_frame().await?;
    let button = driver.find(By::Css("#tonk-perf-s6-edit")).await?;
    let observed = Instant::now();
    button.click().await?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = driver
            .execute(
                r#"
                const button = document.querySelector('#tonk-perf-s6-edit');
                return button ? {...button.dataset, text: button.textContent} : null;
                "#,
                vec![],
            )
            .await?
            .json()
            .clone();
        if state["state"] == "complete" {
            ensure!(state["trusted"] == "true", "S6 edit click was not trusted");
            ensure!(
                state["status"] == "200",
                "S6 edit returned a non-success status"
            );
            let browser_ms = state["elapsedMs"]
                .as_str()
                .context("S6 edit omitted browser elapsed time")?
                .parse::<f64>()?;
            let claims = state["claims"]
                .as_str()
                .context("S6 edit omitted commit claims")?
                .parse::<u64>()?;
            return Ok((
                browser_ms,
                observed.elapsed().as_secs_f64() * 1000.0,
                claims,
            ));
        }
        ensure!(state["state"] != "failed", "S6 edit failed: {state}");
        ensure!(Instant::now() < deadline, "S6 edit did not complete");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn summarize_probe(snapshot: &Value, fixture: &S6AccountFixture) -> Value {
    let inactive: BTreeSet<&str> = fixture
        .inactive_subjects
        .iter()
        .map(String::as_str)
        .collect();
    let mut commands = BTreeMap::<String, u64>::new();
    let mut active_requests = 0_u64;
    let mut inactive_requests = 0_u64;
    let mut unique_inactive = BTreeSet::new();
    for event in snapshot["events"].as_array().into_iter().flatten() {
        if let Some(command) = event["command"].as_str() {
            *commands.entry(command.to_owned()).or_default() += 1;
        }
        match event["subject"].as_str() {
            Some(subject) if subject == fixture.active_subject => active_requests += 1,
            Some(subject) if inactive.contains(subject) => {
                inactive_requests += 1;
                unique_inactive.insert(subject);
            }
            _ => {}
        }
    }
    json!({
        "attempted_upstream_requests": snapshot["event_count"],
        "delayed_inactive_requests": snapshot["delayed_event_count"],
        "active_requests": active_requests,
        "inactive_requests": inactive_requests,
        "unique_inactive_repositories": unique_inactive.len(),
        "commands": commands,
        "runtime_subjects_persisted": false,
    })
}

pub(crate) async fn sample(env: &TestEnvironment, request: &Value, output: &Path) -> Result<Value> {
    ensure!(
        std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
        "Chrome CDP is required; Safari S6 metrics are unavailable"
    );
    validate_request(request)?;
    let driver = env.blank_driver().await?;
    let measured: Result<Value> = async {
        let browser = configure_desktop(&driver).await?;
        add_prf_virtual_authenticator(&driver).await?;
        goto(&driver, env.tonk_web.as_str()).await?;
        let fixture = provision_s6_account_fixture(&driver, env).await?;
        let route = S3Fixture {
            route_key: fixture.active_key.clone(),
            space_subject: fixture.active_subject.clone(),
            direct_url: env.tonk_web.join(&format!("space/{}", fixture.active_key))?,
        };
        navigate_s3_direct(&driver, &route).await?;
        install_edit_control(&driver, &fixture.active_key).await?;
        configure_probe(env, &fixture).await?;
        await_inactive_request(env).await?;
        let (edit_completed_ms, edit_completed_observed_ms, committed_claims) =
            perform_edit(&driver).await?;

        // Stay below the two-second scheduler cadence: this captures the drain
        // that overlapped the edit without admitting the next ordinary drain.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let probe = summarize_probe(&read_probe(env).await?, &fixture);
        driver.enter_default_frame().await?;
        let screenshot = output.with_extension("png");
        driver.screenshot(&screenshot).await?;
        let version = verify_artifact(env, &driver, WIDTH, HEIGHT).await?;
        Ok(json!({
            "schema_version": 1,
            "status": "ok",
            "identity_verified": true,
            "profile_verified": true,
            "environment_verified": false,
            "fixture_verified": true,
            "build_id": version["build"],
            "browser": browser,
            "diagnostics_enabled": std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0"),
            "scenario": S6_SCENARIO,
            "profile": "desktop",
            "profile_settings": request["profile_settings"],
            "cache_state": S6_CACHE_STATE,
            "metrics": {
                "active_query_ms": null,
                "edit_completed_ms": edit_completed_ms,
                "edit_completed_observed_ms": edit_completed_observed_ms,
                "visible_freshness_ms": null,
                "idle_freshness_ms": null,
                "dirty_push_ms": null,
            },
            "mechanism": probe,
            "guards": {
                "repository_count": 20,
                "active_repository_count": 1,
                "inactive_repository_count": 19,
                "created_remote_backed_repository_count": 19,
                "delayed_inactive_repository_count": 18,
                "unshaped_welcome_repository_count": 1,
                "trusted_input": true,
                "commit_response_status": 200,
                "committed_claims": committed_claims,
                "dirty_push_verified": false,
                "visible_freshness_verified": false,
                "idle_freshness_verified": false,
            },
            "unavailable": {
                "active_query_ms": "the first S6 slice measures only the registered foreground edit primary",
                "visible_freshness_ms": "remote-writer freshness injection is not implemented",
                "idle_freshness_ms": "the 60-second guard is covered by deterministic queue tests, not yet by this browser slice",
                "dirty_push_ms": "the probe records permit attempts but does not yet identify push completion",
            },
            "endpoint": {
                "edit": "trusted WebDriver click through successful one-entity evaluate response and visible completion label",
                "probe_start": "first delayed inactive repository permit request",
                "probe_delay_ms": PROBE_DELAY_MS,
                "sample_unit": "one independently provisioned linked profile with twenty cached repositories",
            },
            "clock": "same-document performance.now around fetch-backed edit completion; native monotonic observation is diagnostic",
            "screenshot": screenshot,
        }))
    }
    .await;
    let quit = driver.quit().await;
    let result = measured?;
    quit?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "schema_version": 1,
            "scenario": "S6",
            "profile": "desktop",
            "fixture": s6_request_fixture(),
            "profile_settings": {
                "cpu_slowdown": 1,
                "latency_ms": 0,
                "download_bytes_per_second": -1,
                "upload_bytes_per_second": -1,
                "viewport_width": WIDTH,
                "viewport_height": HEIGHT,
            },
        })
    }

    #[test]
    fn request_requires_the_exact_fixture_and_desktop_profile() {
        let mut value = request();
        assert_eq!(validate_request(&value).unwrap(), (WIDTH, HEIGHT));
        value["fixture"]["cache_state"] = json!("different");
        assert!(validate_request(&value).is_err());
        value = request();
        value["profile_settings"]["viewport_width"] = json!(1279);
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn canonical_dataset_digest_is_current() {
        use sha2_0_10::{Digest, Sha256};

        assert_eq!(
            hex::encode(Sha256::digest(S6_DATASET_CANONICAL_JSON.as_bytes())),
            S6_DATASET_SHA256
        );
    }
}
