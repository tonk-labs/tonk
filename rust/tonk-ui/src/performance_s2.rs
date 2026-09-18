//! Returning linked-account diagnostic sampler for the frozen S2 fixture.
//!
//! Fixture provisioning and every account/content correctness read happen
//! outside the interval from root navigation to the displayed returning Hub.

use std::path::Path;
use std::time::Instant;

use anyhow::{Result, ensure};
use serde_json::{Value, json};
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;

use crate::helpers::TestEnvironment;
use crate::performance_s2_fixture::{
    S2_CACHE_STATE, navigate_s2_returning_visible, provision_s2_fixture, restart_s2_driver,
    validate_s2_request, verify_s2_returning,
};
use crate::performance_support::verify_artifact;

const WIDTH: u64 = 1280;
const HEIGHT: u64 = 800;

#[derive(Debug)]
struct CompleteMeasurement {
    local_content_ready_observed_ms: f64,
}

#[derive(Debug)]
enum Attempt {
    Complete(CompleteMeasurement),
    Failed {
        status: &'static str,
        category: &'static str,
        phase: &'static str,
    },
}

pub(crate) fn validate_request(request: &Value) -> Result<(u64, u64)> {
    validate_s2_request(request)?;
    let settings = &request["profile_settings"];
    ensure!(
        request["profile"] == "desktop",
        "S2 currently supports only the frozen desktop profile"
    );
    ensure!(
        settings
            == &json!({
                "cpu_slowdown": 1,
                "latency_ms": 0,
                "download_bytes_per_second": -1,
                "upload_bytes_per_second": -1,
                "viewport_width": WIDTH,
                "viewport_height": HEIGHT,
            }),
        "S2 desktop profile settings differ from the frozen 1280x800 unthrottled profile"
    );
    Ok((WIDTH, HEIGHT))
}

fn webdriver_timeout(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<thirtyfour::error::WebDriverError>()
        .is_some_and(|error| {
            matches!(
                error.as_inner(),
                thirtyfour::error::WebDriverErrorInner::WebDriverTimeout(_)
                    | thirtyfour::error::WebDriverErrorInner::Timeout(_)
            )
        })
}

fn webdriver_failure(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<thirtyfour::error::WebDriverError>()
        .is_some()
}

fn webdriver_interaction_failure(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<thirtyfour::error::WebDriverError>()
        .is_some_and(|error| {
            matches!(
                error.as_inner(),
                thirtyfour::error::WebDriverErrorInner::ElementClickIntercepted(_)
                    | thirtyfour::error::WebDriverErrorInner::ElementNotInteractable(_)
                    | thirtyfour::error::WebDriverErrorInner::InvalidElementState(_)
                    | thirtyfour::error::WebDriverErrorInner::MoveTargetOutOfBounds(_)
                    | thirtyfour::error::WebDriverErrorInner::NoSuchElement(_)
                    | thirtyfour::error::WebDriverErrorInner::StaleElementReference(_)
            )
        })
}

fn classify_s2_error(
    error: anyhow::Error,
    phase: &'static str,
    functional_category: &'static str,
    timeout_category: &'static str,
) -> Result<Attempt> {
    if webdriver_timeout(&error)
        || error.to_string().contains(" never ")
        || error.to_string().contains("did not settle")
        || error.to_string().contains("timed out")
    {
        return Ok(Attempt::Failed {
            status: "timeout",
            category: timeout_category,
            phase,
        });
    }
    if webdriver_failure(&error) && !webdriver_interaction_failure(&error) {
        return Err(error.context(format!("S2 infrastructure failure during {phase}")));
    }
    Ok(Attempt::Failed {
        status: "functional_failure",
        category: functional_category,
        phase,
    })
}

async fn configure_desktop(driver: &WebDriver) -> Result<Value> {
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
    let viewport = driver
        .execute("return [innerWidth, innerHeight]", vec![])
        .await?;
    ensure!(
        viewport.json() == &json!([WIDTH, HEIGHT]),
        "S2 desktop viewport override was not applied"
    );
    cdp.execute_cdp("Browser.getVersion")
        .await
        .map_err(Into::into)
}

fn unavailable() -> Value {
    json!({
        "local_content_ready_ms": "the visible returning-Hub endpoint has no validated presentation timestamp; the native monotonic observation is diagnostic only",
        "account_sweeps": "account-sweep counters are not implemented",
        "writer_waits": "writer-wait counters are not implemented",
        "offline_authorization": "linked-profile offline authorization is not implemented",
        "requests": "all-target Network collection is not implemented",
        "bytes": "all-target Network collection is not implemented",
    })
}

fn ok_record(
    request: &Value,
    browser: Value,
    version: &Value,
    screenshot: &Path,
    measurement: CompleteMeasurement,
) -> Value {
    json!({
        "schema_version": 1,
        "status": "ok",
        "identity_verified": true,
        "profile_verified": true,
        "environment_verified": false,
        "fixture_verified": true,
        "environment": {
            "browser": browser,
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "hardware": null,
            "power_mode": null,
            "compression": null,
            "server": "repository Caddy HTTPS test server",
        },
        "build_id": version["build"],
        "browser": browser,
        "diagnostics": {
            "available": true,
            "kind": "native monotonic semantic observation",
            "trace_collection_available": false,
            "presentation_endpoint_verified": false,
            "resource_guards_verified": false,
            "account_sweep_guard_verified": false,
            "writer_wait_guard_verified": false,
            "offline_authorization_verified": false,
        },
        "diagnostics_enabled": std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0"),
        "scenario": "S2",
        "profile": "desktop",
        "profile_settings": request["profile_settings"],
        "cache_state": S2_CACHE_STATE,
        "metrics": {
            "local_content_ready_ms": null,
            "local_content_ready_observed_ms": measurement.local_content_ready_observed_ms,
            "account_sweeps": null,
            "writer_waits": null,
            "offline_authorization": null,
            "requests": null,
            "bytes": null,
        },
        "unavailable": unavailable(),
        "endpoint": {
            "local_content": "displayed returning Hub, linked account trigger, spaces roster, and exact fixture-space direct label",
            "post_endpoint_verification": "active registered account, empty custody work, exact two-space roster and repository identities, local marker, origin/main, controlled new worker",
            "returning_authenticator_installed": false,
            "worker_restart_verified": true,
            "presentation_endpoint_verified": false,
            "sample_unit": "one independently provisioned linked profile followed by one cold browser/worker restart",
        },
        "clock": "std::time::Instant around root navigation through semantic Hub completion is diagnostic",
        "screenshot": screenshot,
    })
}

fn failure_record(
    request: &Value,
    browser: Value,
    version: &Value,
    screenshot: &Path,
    status: &str,
    category: &str,
    phase: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "status": status,
        "identity_verified": true,
        "profile_verified": true,
        "environment_verified": false,
        "fixture_verified": false,
        "environment": {
            "browser": browser,
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "hardware": null,
            "power_mode": null,
            "compression": null,
            "server": "repository Caddy HTTPS test server",
        },
        "build_id": version["build"],
        "browser": browser,
        "diagnostics_enabled": std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0"),
        "scenario": "S2",
        "profile": "desktop",
        "profile_settings": request["profile_settings"],
        "cache_state": S2_CACHE_STATE,
        "metrics": {},
        "failure_category": category,
        "failure_phase": phase,
        "screenshot": screenshot,
    })
}

async fn finish_record(
    env: &TestEnvironment,
    request: &Value,
    output: &Path,
    driver: &WebDriver,
    browser: Value,
    attempt: Attempt,
) -> Result<Value> {
    driver.enter_default_frame().await?;
    let screenshot = output.with_extension("png");
    driver.screenshot(&screenshot).await?;
    let version = verify_artifact(env, driver, WIDTH, HEIGHT).await?;
    Ok(match attempt {
        Attempt::Complete(measurement) => {
            ok_record(request, browser, &version, &screenshot, measurement)
        }
        Attempt::Failed {
            status,
            category,
            phase,
        } => failure_record(
            request,
            browser,
            &version,
            &screenshot,
            status,
            category,
            phase,
        ),
    })
}

fn provision_failure_record(request: &Value, status: &str, category: &str, phase: &str) -> Value {
    json!({
        "schema_version": 1,
        "status": status,
        "identity_verified": false,
        "profile_verified": false,
        "environment_verified": false,
        "fixture_verified": false,
        "build_id": null,
        "browser": null,
        "diagnostics_enabled": std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0"),
        "scenario": "S2",
        "profile": "desktop",
        "profile_settings": request["profile_settings"],
        "cache_state": S2_CACHE_STATE,
        "metrics": {},
        "failure_category": category,
        "failure_phase": phase,
        "unavailable": {
            "artifact_identity": "fixture provisioning did not yield the returning measurement browser",
            "profile_application": "fixture provisioning failed before the returning desktop profile could be verified",
            "screenshot": "no returning measurement browser existed",
        },
        "screenshot": null,
    })
}

/// Measure the frozen S2 returning-Hub endpoint in one independently
/// provisioned linked profile and one cold browser/worker restart.
pub(crate) async fn sample(env: &TestEnvironment, request: &Value, output: &Path) -> Result<Value> {
    ensure!(
        std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
        "Chrome CDP is required; Safari S2 metrics are unavailable"
    );
    let (width, height) = validate_request(request)?;
    ensure!(
        (width, height) == (WIDTH, HEIGHT),
        "validated S2 viewport does not match the implemented desktop sampler"
    );

    let fixture = match provision_s2_fixture(env).await {
        Ok(fixture) => fixture,
        Err(error) => {
            let attempt = classify_s2_error(
                error,
                "fixture_provision",
                "fixture_provision_failure",
                "fixture_provision_timeout",
            )?;
            let Attempt::Failed {
                status,
                category,
                phase,
            } = attempt
            else {
                unreachable!("error classification cannot produce a complete S2 attempt")
            };
            return Ok(provision_failure_record(request, status, category, phase));
        }
    };

    let driver = restart_s2_driver(env, &fixture).await?;
    let measured: Result<Value> = async {
        let browser = configure_desktop(&driver).await?;
        let started = Instant::now();
        let attempt = match navigate_s2_returning_visible(&driver, env, &fixture).await {
            Ok(()) => {
                let local_content_ready_observed_ms = started.elapsed().as_secs_f64() * 1000.0;
                match verify_s2_returning(&driver, &fixture).await {
                    Ok(_) => Attempt::Complete(CompleteMeasurement {
                        local_content_ready_observed_ms,
                    }),
                    Err(error) => classify_s2_error(
                        error,
                        "post_endpoint_verification",
                        "returning_state_functional_failure",
                        "returning_state_verification_timeout",
                    )?,
                }
            }
            Err(error) => classify_s2_error(
                error,
                "returning_route",
                "local_content_functional_failure",
                "local_content_readiness_timeout",
            )?,
        };
        finish_record(env, request, output, &driver, browser, attempt).await
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
    use crate::performance_s2_fixture::s2_request_fixture;

    fn request() -> Value {
        json!({
            "schema_version": 1,
            "scenario": "S2",
            "profile": "desktop",
            "fixture": s2_request_fixture(),
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
    fn request_requires_the_exact_desktop_profile() {
        let mut value = request();
        assert_eq!(validate_request(&value).unwrap(), (WIDTH, HEIGHT));
        value["profile_settings"]["viewport_width"] = json!(1279);
        assert!(validate_request(&value).is_err());
        value = request();
        value["profile_settings"]["extra"] = json!(true);
        assert!(validate_request(&value).is_err());
        value = request();
        value["profile"] = json!("constrained");
        assert!(validate_request(&value).is_err());
    }

    #[test]
    fn required_unimplemented_guards_remain_explicitly_unavailable() {
        let unavailable = unavailable();
        for metric in [
            "local_content_ready_ms",
            "account_sweeps",
            "writer_waits",
            "offline_authorization",
            "requests",
            "bytes",
        ] {
            assert!(
                unavailable[metric].is_string(),
                "missing reason for {metric}"
            );
        }

        let failed = provision_failure_record(
            &request(),
            "functional_failure",
            "fixture_provision_failure",
            "fixture_provision",
        );
        assert_eq!(failed["identity_verified"], false);
        assert_eq!(failed["profile_verified"], false);
        assert_eq!(failed["fixture_verified"], false);
        assert_eq!(failed["metrics"], json!({}));
        assert_eq!(failed["screenshot"], Value::Null);
        assert!(failed["unavailable"]["artifact_identity"].is_string());
    }
}
