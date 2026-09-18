//! Direct-space and menu performance measurement for the frozen S3 fixture.
//!
//! One call owns one browser session. Fixture provisioning, observer setup,
//! menu closing, screenshots, and artifact identity checks are deliberately
//! outside the four menu-open intervals and the direct-route interval.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;

use crate::helpers::TestEnvironment;
use crate::performance_fixtures::{
    S3_CACHE_STATE, S3Fixture, S3Menu, S3MenuContents, close_s3_menu, navigate_s3_direct,
    open_s3_menu, provision_s3_fixture, validate_s3_request,
};
use crate::performance_support::verify_artifact;

const WIDTH: u64 = 1280;
const HEIGHT: u64 = 800;
const EVENT_TIMING_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct Interaction {
    observed_ms: f64,
    event_timing_ms: Option<f64>,
    event_timing_supported: bool,
    observed_epoch_ms: f64,
    contents: S3MenuContents,
}

#[derive(Debug)]
struct CompleteMeasurement {
    route_observed_ms: f64,
    space_first: Interaction,
    space_repeated: Interaction,
    share_first: Interaction,
    share_repeated: Interaction,
}

#[derive(Debug)]
enum Attempt {
    Complete(Box<CompleteMeasurement>),
    Failed {
        status: &'static str,
        category: &'static str,
        phase: &'static str,
    },
}

pub(crate) fn validate_request(request: &Value) -> Result<(u64, u64)> {
    validate_s3_request(request)?;
    let settings = &request["profile_settings"];
    ensure!(
        request["profile"] == "desktop",
        "S3 currently supports only the frozen desktop profile"
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
        "S3 desktop profile settings differ from the frozen 1280x800 unthrottled profile"
    );
    Ok((WIDTH, HEIGHT))
}

fn menu_name(menu: S3Menu) -> &'static str {
    match menu {
        S3Menu::Space => "space",
        S3Menu::Share => "share",
    }
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

fn classify_fixture_error(
    error: anyhow::Error,
    phase: &'static str,
    functional_category: &'static str,
    timeout_category: &'static str,
) -> Result<Attempt> {
    if webdriver_timeout(&error)
        || error.to_string().contains(" never ")
        || error.to_string().contains("did not settle")
        || error.to_string().contains("did not close")
    {
        return Ok(Attempt::Failed {
            status: "timeout",
            category: timeout_category,
            phase,
        });
    }
    if webdriver_failure(&error) && !webdriver_interaction_failure(&error) {
        return Err(error.context(format!("S3 infrastructure failure during {phase}")));
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
    // Install this before fixture provisioning, which is the first app visit.
    cdp.execute_cdp_with_params(
        "Page.addScriptToEvaluateOnNewDocument",
        json!({
            "source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"
        }),
    )
    .await?;
    let viewport = driver
        .execute("return [innerWidth, innerHeight]", vec![])
        .await?;
    ensure!(
        viewport.json() == &json!([WIDTH, HEIGHT]),
        "S3 desktop viewport override was not applied: {}",
        viewport.json()
    );
    cdp.execute_cdp("Browser.getVersion")
        .await
        .map_err(Into::into)
}

async fn enter_space_shell(driver: &WebDriver) -> Result<()> {
    driver.enter_default_frame().await?;
    let frame = driver
        .find(By::Css("tonk-site > iframe"))
        .await
        .context("S3 space shell frame is absent before menu input")?;
    ensure!(
        frame.is_displayed().await?,
        "S3 space shell frame is hidden before menu input"
    );
    frame.enter_frame().await?;
    Ok(())
}

/// Install the observer without reading or populating menu contents. The
/// listener and Event Timing entries live in the same frame and therefore use
/// one `performance.timeOrigin` and timestamp domain.
async fn prepare_menu_input(driver: &WebDriver, menu: S3Menu) -> Result<()> {
    enter_space_shell(driver).await?;
    let prepared = driver
        .execute(
            r#"
            const kind = arguments[0];
            globalThis.__tonkS3Input?.observer?.disconnect();
            const bar = document.querySelector('tonk-fab');
            const cell = bar?.shadowRoot?.querySelector(`[data-cell="${kind}"]`);
            if (!cell) return {installed: false};
            const supported = Array.isArray(PerformanceObserver.supportedEntryTypes)
              && PerformanceObserver.supportedEntryTypes.includes('event');
            const state = {entries: [], trusted: false, trustedClickStart: null,
              eventTimingSupported: supported, observer: null};
            cell.addEventListener('click', event => {
              if (event.isTrusted) {
                state.trusted = true;
                state.trustedClickStart = event.timeStamp;
              }
            }, {once: true});
            if (supported) {
              const observer = new PerformanceObserver(list => {
                for (const entry of list.getEntries()) {
                  if (!['pointerdown', 'pointerup', 'click', 'keydown'].includes(entry.name)) continue;
                  state.entries.push({name: entry.name, startTime: entry.startTime,
                    duration: entry.duration, processingStart: entry.processingStart,
                    processingEnd: entry.processingEnd, interactionId: entry.interactionId || 0});
                }
              });
              observer.observe({type: 'event', durationThreshold: 16});
              state.observer = observer;
            }
            globalThis.__tonkS3Input = state;
            return {installed: true};
            "#,
            vec![json!(menu_name(menu))],
        )
        .await?;
    ensure!(
        prepared.json()["installed"] == true,
        "S3 {} menu opener disappeared before trusted input",
        menu_name(menu)
    );
    Ok(())
}

fn input_event_timing(observation: &Value) -> Option<f64> {
    let entries = observation["entries"].as_array()?;
    let trusted_start = observation["trusted_click_start_ms"].as_f64()?;
    let interaction = entries
        .iter()
        .filter(|entry| entry["name"] == "click")
        .filter_map(|entry| {
            let start = entry["startTime"].as_f64()?;
            let interaction = entry["interactionId"].as_u64()?;
            (interaction > 0 && (start - trusted_start).abs() <= 16.0)
                .then_some((interaction, (start - trusted_start).abs()))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))?
        .0;
    entries
        .iter()
        .filter(|entry| entry["interactionId"].as_u64() == Some(interaction))
        .filter_map(|entry| entry["duration"].as_f64())
        .filter(|duration| duration.is_finite() && *duration >= 0.0)
        .reduce(f64::max)
}

async fn await_menu_input(driver: &WebDriver, menu: S3Menu) -> Result<Value> {
    let deadline = Instant::now() + EVENT_TIMING_WAIT;
    loop {
        let observation = driver
            .execute(
                r#"
                const state = globalThis.__tonkS3Input;
                return {trusted: state?.trusted === true,
                  trusted_click_start_ms: state?.trustedClickStart,
                  event_timing_supported: state?.eventTimingSupported === true,
                  observed_epoch_ms: performance.timeOrigin + performance.now(),
                  entries: state?.entries || []};
                "#,
                vec![],
            )
            .await?
            .json()
            .clone();
        if observation["trusted"] == true
            && (input_event_timing(&observation).is_some() || Instant::now() >= deadline)
        {
            let _ = driver
                .execute(
                    "globalThis.__tonkS3Input?.observer?.disconnect(); return true;",
                    vec![],
                )
                .await?;
            return Ok(observation);
        }
        ensure!(
            Instant::now() < deadline,
            "S3 {} menu did not receive a trusted WebDriver click",
            menu_name(menu)
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn measure_menu(
    driver: &WebDriver,
    fixture: &S3Fixture,
    menu: S3Menu,
) -> Result<Interaction> {
    prepare_menu_input(driver, menu).await?;
    let started = Instant::now();
    let contents = open_s3_menu(driver, fixture, menu).await?;
    let observed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let input = await_menu_input(driver, menu).await?;
    let observed_epoch_ms = input["observed_epoch_ms"]
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .context("S3 input observer returned no finite frame timestamp")?;
    Ok(Interaction {
        observed_ms,
        event_timing_ms: input_event_timing(&input),
        event_timing_supported: input["event_timing_supported"] == true,
        observed_epoch_ms,
        contents,
    })
}

async fn measure_all(driver: &WebDriver, env: &TestEnvironment) -> Result<Attempt> {
    let fixture = match provision_s3_fixture(driver, env).await {
        Ok(fixture) => fixture,
        Err(error) => {
            return classify_fixture_error(
                error,
                "fixture_provision",
                "fixture_provision_failure",
                "fixture_provision_timeout",
            );
        }
    };

    let route_started = Instant::now();
    if let Err(error) = navigate_s3_direct(driver, &fixture).await {
        return classify_fixture_error(
            error,
            "direct_route",
            "route_functional_failure",
            "route_readiness_timeout",
        );
    }
    let route_observed_ms = route_started.elapsed().as_secs_f64() * 1000.0;

    let space_first = match measure_menu(driver, &fixture, S3Menu::Space).await {
        Ok(value) => value,
        Err(error) => {
            return classify_fixture_error(
                error,
                "space_menu_first",
                "space_menu_first_functional_failure",
                "space_menu_first_timeout",
            );
        }
    };
    if let Err(error) = close_s3_menu(driver, S3Menu::Space).await {
        return classify_fixture_error(
            error,
            "space_menu_first_close",
            "space_menu_close_functional_failure",
            "space_menu_close_timeout",
        );
    }
    let space_repeated = match measure_menu(driver, &fixture, S3Menu::Space).await {
        Ok(value) => value,
        Err(error) => {
            return classify_fixture_error(
                error,
                "space_menu_repeated",
                "space_menu_repeated_functional_failure",
                "space_menu_repeated_timeout",
            );
        }
    };
    if let Err(error) = close_s3_menu(driver, S3Menu::Space).await {
        return classify_fixture_error(
            error,
            "space_menu_repeated_close",
            "space_menu_close_functional_failure",
            "space_menu_close_timeout",
        );
    }

    let share_first = match measure_menu(driver, &fixture, S3Menu::Share).await {
        Ok(value) => value,
        Err(error) => {
            return classify_fixture_error(
                error,
                "share_menu_first",
                "share_menu_first_functional_failure",
                "share_menu_first_timeout",
            );
        }
    };
    if let Err(error) = close_s3_menu(driver, S3Menu::Share).await {
        return classify_fixture_error(
            error,
            "share_menu_first_close",
            "share_menu_close_functional_failure",
            "share_menu_close_timeout",
        );
    }
    let share_repeated = match measure_menu(driver, &fixture, S3Menu::Share).await {
        Ok(value) => value,
        Err(error) => {
            return classify_fixture_error(
                error,
                "share_menu_repeated",
                "share_menu_repeated_functional_failure",
                "share_menu_repeated_timeout",
            );
        }
    };
    if let Err(error) = close_s3_menu(driver, S3Menu::Share).await {
        return classify_fixture_error(
            error,
            "share_menu_repeated_close",
            "share_menu_close_functional_failure",
            "share_menu_close_timeout",
        );
    }

    Ok(Attempt::Complete(Box::new(CompleteMeasurement {
        route_observed_ms,
        space_first,
        space_repeated,
        share_first,
        share_repeated,
    })))
}

fn unavailable_for(measurement: &CompleteMeasurement) -> Value {
    let mut unavailable = json!({
        "route_ready_ms": "the semantic route endpoint has no validated presentation timestamp; the native monotonic observation is diagnostic only",
        "menu_ready_ms": "S3 has four distinct ordered menu endpoints and no predeclared scalar aggregation",
        "input_to_next_paint_ms": "no independently validated next-paint or presentation endpoint",
        "subscription_counts": "subscription transport counters are not implemented",
        "requests": "all-target Network collection is not implemented",
        "bytes": "all-target Network collection is not implemented",
    });
    for (name, interaction) in [
        (
            "space_menu_first_input_event_timing_ms",
            &measurement.space_first,
        ),
        (
            "space_menu_repeated_input_event_timing_ms",
            &measurement.space_repeated,
        ),
        (
            "share_menu_first_input_event_timing_ms",
            &measurement.share_first,
        ),
        (
            "share_menu_repeated_input_event_timing_ms",
            &measurement.share_repeated,
        ),
    ] {
        if interaction.event_timing_ms.is_none() {
            unavailable[name] = json!(if interaction.event_timing_supported {
                "trusted input completed but Chrome emitted no qualifying Event Timing interaction"
            } else {
                "Event Timing entries are unsupported in this browser"
            });
        }
    }
    unavailable
}

fn ok_record(
    request: &Value,
    browser: Value,
    version: &Value,
    screenshot: &Path,
    measurement: CompleteMeasurement,
) -> Value {
    let unavailable = unavailable_for(&measurement);
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
            "kind": "native monotonic semantic observations plus same-frame trusted Event Timing",
            "trace_collection_available": false,
            "presentation_endpoint_verified": false,
            "resource_guards_verified": false,
        },
        "diagnostics_enabled": std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0"),
        "scenario": "S3",
        "profile": "desktop",
        "profile_settings": request["profile_settings"],
        "cache_state": S3_CACHE_STATE,
        "metrics": {
            "route_ready_ms": null,
            "route_ready_observed_ms": measurement.route_observed_ms,
            "menu_ready_ms": null,
            "space_menu_first_ready_observed_ms": measurement.space_first.observed_ms,
            "space_menu_repeated_ready_observed_ms": measurement.space_repeated.observed_ms,
            "share_menu_first_ready_observed_ms": measurement.share_first.observed_ms,
            "share_menu_repeated_ready_observed_ms": measurement.share_repeated.observed_ms,
            "input_to_next_paint_ms": null,
            "space_menu_first_input_event_timing_ms": measurement.space_first.event_timing_ms,
            "space_menu_repeated_input_event_timing_ms": measurement.space_repeated.event_timing_ms,
            "share_menu_first_input_event_timing_ms": measurement.share_first.event_timing_ms,
            "share_menu_repeated_input_event_timing_ms": measurement.share_repeated.event_timing_ms,
            "subscription_counts": null,
            "requests": null,
            "bytes": null,
        },
        "unavailable": unavailable,
        "endpoint": {
            "route": "direct fixture URL; displayed subject-matched tonk-fab and displayed nested content frame",
            "space_menu": "displayed open action plus exactly one current Welcome to Tonk subscription row",
            "share_menu": "displayed log in to share action plus one-member live roster",
            "trusted_input": "four WebDriver element clicks, each verified by a same-frame isTrusted listener",
            "input_verified": true,
            "menu_contents_ready": true,
            "presentation_endpoint_verified": false,
            "sample_unit": "one independent browser session containing one route and four ordered menu opens",
            "ordered_interactions": [
                {"menu": "space", "ordinal": "first", "dynamic_rows": measurement.space_first.contents.dynamic_rows,
                    "primary_label": measurement.space_first.contents.primary_label,
                    "observed_epoch_ms": measurement.space_first.observed_epoch_ms},
                {"menu": "space", "ordinal": "repeated", "dynamic_rows": measurement.space_repeated.contents.dynamic_rows,
                    "primary_label": measurement.space_repeated.contents.primary_label,
                    "observed_epoch_ms": measurement.space_repeated.observed_epoch_ms},
                {"menu": "share", "ordinal": "first", "dynamic_rows": measurement.share_first.contents.dynamic_rows,
                    "primary_label": measurement.share_first.contents.primary_label,
                    "observed_epoch_ms": measurement.share_first.observed_epoch_ms},
                {"menu": "share", "ordinal": "repeated", "dynamic_rows": measurement.share_repeated.contents.dynamic_rows,
                    "primary_label": measurement.share_repeated.contents.primary_label,
                    "observed_epoch_ms": measurement.share_repeated.observed_epoch_ms},
            ],
        },
        "clock": "std::time::Instant around semantic route/menu completion is diagnostic; Event Timing is associated within each input frame",
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
        "scenario": "S3",
        "profile": "desktop",
        "profile_settings": request["profile_settings"],
        "cache_state": S3_CACHE_STATE,
        "metrics": {},
        "failure_category": category,
        "failure_phase": phase,
        "screenshot": screenshot,
    })
}

/// Measure the frozen S3 route and four ordered menu interactions in one
/// independent browser session.
pub(crate) async fn sample(env: &TestEnvironment, request: &Value, output: &Path) -> Result<Value> {
    ensure!(
        std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
        "Chrome CDP is required; Safari S3 metrics are unavailable"
    );
    let (width, height) = validate_request(request)?;
    ensure!(
        (width, height) == (WIDTH, HEIGHT),
        "validated S3 viewport does not match the implemented desktop sampler"
    );
    let driver = env.blank_driver().await?;
    let measured: Result<Value> = async {
        let browser = configure_desktop(&driver).await?;
        let attempt = measure_all(&driver, env).await?;

        // Post-endpoint evidence must not warm or lengthen any measured action.
        driver.enter_default_frame().await?;
        let screenshot = output.with_extension("png");
        driver.screenshot(&screenshot).await?;
        let version = verify_artifact(env, &driver, WIDTH, HEIGHT).await?;

        Ok(match attempt {
            Attempt::Complete(measurement) => {
                ok_record(request, browser, &version, &screenshot, *measurement)
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
    .await;
    let quit = driver.quit().await;
    let result = measured?;
    quit?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance_fixtures::s3_request_fixture;

    fn request() -> Value {
        json!({
            "schema_version": 1,
            "scenario": "S3",
            "profile": "desktop",
            "fixture": s3_request_fixture(),
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
    fn event_timing_matches_only_the_trusted_click_interaction() {
        let observation = json!({
            "trusted_click_start_ms": 40.0,
            "entries": [
                {"name": "pointerdown", "startTime": 35.0, "duration": 16.0, "interactionId": 7},
                {"name": "click", "startTime": 40.0, "duration": 24.0, "interactionId": 7},
                {"name": "click", "startTime": 41.0, "duration": 80.0, "interactionId": 0},
                {"name": "click", "startTime": 100.0, "duration": 96.0, "interactionId": 9},
            ],
        });
        assert_eq!(input_event_timing(&observation), Some(24.0));
        assert_eq!(
            input_event_timing(&json!({
                "trusted_click_start_ms": 40.0,
                "entries": [{"name": "click", "startTime": 40.0, "duration": 24.0, "interactionId": 0}],
            })),
            None
        );
    }

    #[test]
    fn contextual_webdriver_errors_keep_failure_provenance() {
        use thirtyfour::error::{WebDriverError, WebDriverErrorInfo};

        let invalid_session = anyhow::Error::from(WebDriverError::InvalidSessionId(
            WebDriverErrorInfo::new("invalid session".to_owned()),
        ))
        .context("menu click context");
        assert!(
            classify_fixture_error(invalid_session, "share_menu_first", "functional", "timeout")
                .is_err()
        );

        let not_interactable = anyhow::Error::from(WebDriverError::ElementNotInteractable(
            WebDriverErrorInfo::new("not interactable".to_owned()),
        ))
        .context("menu click context");
        let classified = classify_fixture_error(
            not_interactable,
            "share_menu_first",
            "functional",
            "timeout",
        )
        .unwrap();
        assert!(matches!(
            classified,
            Attempt::Failed {
                status: "functional_failure",
                category: "functional",
                phase: "share_menu_first",
            }
        ));
    }
}
