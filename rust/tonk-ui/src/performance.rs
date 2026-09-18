//! Isolated release-artifact smoke measurement. Observation latency is deliberately
//! distinct from validated presentation/readiness timing; it cannot retain a patch.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use anyhow::{Context, Result, ensure};
    use serde_json::{Value, json};
    use std::path::Path;
    use std::time::{Duration, Instant};
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::prelude::*;

    use crate::helpers::{TestEnvironment, TestServers};
    use crate::performance_support::{verify_artifact, verify_copy};

    const S1_BASELINE_MEASUREMENT: &str = "navigation-to-usable-observed-v3";
    const S1_EXPECTED_HEADLINE: &str =
        "makes your small software live right away and ready to share";
    const S1_POLL_INTERVAL: Duration = Duration::from_millis(50);
    const S1_TIMEOUT: Duration = Duration::from_secs(120);
    const CLOCK_SYNC_ID: &str = "tonk-s1-input";
    const MAX_OBSERVED_CLOCK_UNCERTAINTY_MS: f64 = 25.0;
    const FRAME_CLOCK_SCRIPT: &str = r#"
        (() => {
          const token = 'tonk-s1-frame-clock-v1';
          if (!globalThis.__tonkS1FrameClockExchange) {
            globalThis.__tonkS1FrameClockExchange = () => new Promise(resolve => {
              const words = crypto.getRandomValues(new Uint32Array(4));
              const requestId = Array.from(words,
                word => word.toString(16).padStart(8, '0')).join('');
              const innerBefore = performance.timeOrigin + performance.now();
              let settled = false;
              let timer;
              const finish = result => {
                if (settled) return;
                settled = true;
                if (timer !== undefined) clearTimeout(timer);
                removeEventListener('message', handler);
                resolve(result);
              };
              const handler = event => {
                const data = event.data;
                if (event.source !== top || data?.token !== token
                    || data?.kind !== 'pong' || data?.request_id !== requestId
                    || !Number.isFinite(data.top_epoch_ms)
                    || data.top_epoch_ms < 0) return;
                finish({available: true,
                  inner_before_epoch_ms: innerBefore,
                  top_epoch_ms: data.top_epoch_ms,
                  inner_after_epoch_ms: performance.timeOrigin + performance.now()});
              };
              addEventListener('message', handler);
              timer = setTimeout(() => finish({available: false,
                reason: 'top-frame clock relay timed out'}), 1000);
              top.postMessage({token, kind: 'ping', request_id: requestId}, '*');
            });
          }
          if (window !== top || globalThis.__tonkS1FrameClockRelay) return;
          const handler = event => {
            const data = event.data;
            if (!event.source || event.source === window
                || data?.token !== token || data?.kind !== 'ping'
                || !/^[0-9a-f]{32}$/.test(data.request_id)) return;
            const topEpoch = performance.timeOrigin + performance.now();
            event.source.postMessage({token, kind: 'pong',
              request_id: data.request_id, top_epoch_ms: topEpoch}, '*');
          };
          addEventListener('message', handler);
          globalThis.__tonkS1FrameClockRelay = {token, handler};
        })();
    "#;

    #[derive(Clone, Debug)]
    struct ClockBracket {
        before_epoch_ms: f64,
        after_epoch_ms: f64,
    }

    #[derive(Clone, Debug)]
    struct CrossRealmSample {
        endpoint_epoch_ms: f64,
        uncertainty_ms: f64,
    }

    #[derive(Clone, Debug)]
    struct HeadlineObservation {
        observed_epoch_ms: f64,
        clock_conversion: Option<CrossRealmSample>,
        lcp: Value,
    }

    struct UsableObservation {
        observed_epoch_ms: f64,
        clock_conversion: Option<CrossRealmSample>,
    }

    fn clock_bracket(before_epoch_ms: f64, after_epoch_ms: f64) -> Result<ClockBracket> {
        ensure!(
            before_epoch_ms.is_finite() && after_epoch_ms.is_finite(),
            "frame clock samples must be finite"
        );
        ensure!(
            after_epoch_ms >= before_epoch_ms,
            "frame clock moved backwards across trace marker"
        );
        ensure!(
            after_epoch_ms - before_epoch_ms <= 250.0,
            "trace clock marker round trip exceeded 250ms"
        );
        Ok(ClockBracket {
            before_epoch_ms,
            after_epoch_ms,
        })
    }

    fn cross_realm_sample(
        top_before_epoch_ms: f64,
        endpoint_epoch_ms: f64,
        top_after_epoch_ms: f64,
    ) -> Result<CrossRealmSample> {
        ensure!(
            top_before_epoch_ms.is_finite()
                && endpoint_epoch_ms.is_finite()
                && top_after_epoch_ms.is_finite(),
            "cross-realm clock samples must be finite"
        );
        ensure!(
            top_before_epoch_ms <= endpoint_epoch_ms && endpoint_epoch_ms <= top_after_epoch_ms,
            "cross-realm timeOrigin + now sample escaped its bracketing realm interval"
        );
        let width = top_after_epoch_ms - top_before_epoch_ms;
        ensure!(width <= 1000.0, "cross-realm clock bracket exceeded 1000ms");
        Ok(CrossRealmSample {
            endpoint_epoch_ms,
            uncertainty_ms: width / 2.0,
        })
    }

    fn elapsed_from_navigation(navigation_epoch_ms: f64, endpoint_epoch_ms: f64) -> Result<f64> {
        ensure!(
            navigation_epoch_ms.is_finite() && endpoint_epoch_ms.is_finite(),
            "navigation clock samples must be finite"
        );
        ensure!(
            endpoint_epoch_ms >= navigation_epoch_ms,
            "endpoint clock preceded top-level navigation"
        );
        Ok(endpoint_epoch_ms - navigation_epoch_ms)
    }

    fn observed_from_sample(
        navigation_epoch_ms: f64,
        sample: &CrossRealmSample,
    ) -> Result<Option<f64>> {
        let elapsed = elapsed_from_navigation(navigation_epoch_ms, sample.endpoint_epoch_ms)?;
        Ok((sample.uncertainty_ms <= MAX_OBSERVED_CLOCK_UNCERTAINTY_MS).then_some(elapsed))
    }

    fn observed_from_clock_conversion(
        navigation_epoch_ms: f64,
        observed_epoch_ms: f64,
        conversion: Option<&CrossRealmSample>,
    ) -> Result<Option<f64>> {
        let Some(conversion) = conversion else {
            return Ok(None);
        };
        if conversion.uncertainty_ms > MAX_OBSERVED_CLOCK_UNCERTAINTY_MS {
            return Ok(None);
        }
        Ok(Some(elapsed_from_navigation(
            navigation_epoch_ms,
            observed_epoch_ms,
        )?))
    }

    fn headline_lcp_epoch(observation: &Value, observed_epoch_ms: f64) -> Option<f64> {
        if observation["supported"] != true {
            return None;
        }
        let candidate = &observation["headline_candidate"];
        if !matches!(
            candidate["node_match"].as_str(),
            Some("exact" | "descendant")
        ) {
            return None;
        }
        let start = candidate["start_time_ms"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)?;
        let render = candidate["render_time_ms"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)?;
        let presentation = candidate["presentation_time_ms"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)?;
        let expected = match candidate["presentation_basis"].as_str()? {
            "renderTime" if render > 0.0 => render,
            "startTime" if render == 0.0 => start,
            _ => return None,
        };
        if (presentation - expected).abs() > f64::EPSILON {
            return None;
        }
        candidate["epoch_ms"].as_f64().filter(|epoch| {
            epoch.is_finite() && *epoch >= presentation && *epoch <= observed_epoch_ms + 1.0
        })
    }

    fn frame_clock_conversion(observation: &Value) -> Result<Option<CrossRealmSample>> {
        let exchange = &observation["clock_exchange"];
        if exchange["available"] != true {
            return Ok(None);
        }
        let inner_before_epoch_ms = exchange["inner_before_epoch_ms"]
            .as_f64()
            .context("headline clock exchange omitted inner-before timestamp")?;
        let top_epoch_ms = exchange["top_epoch_ms"]
            .as_f64()
            .context("headline clock exchange omitted top timestamp")?;
        let inner_after_epoch_ms = exchange["inner_after_epoch_ms"]
            .as_f64()
            .context("headline clock exchange omitted inner-after timestamp")?;
        Ok(Some(cross_realm_sample(
            inner_before_epoch_ms,
            top_epoch_ms,
            inner_after_epoch_ms,
        )?))
    }

    async fn frame_epoch_ms(driver: &WebDriver) -> Result<f64> {
        driver
            .execute("return performance.timeOrigin + performance.now()", vec![])
            .await?
            .json()
            .as_f64()
            .filter(|sample| sample.is_finite() && *sample >= 0.0)
            .context("frame returned no finite timeOrigin + now sample")
    }

    async fn first(driver: &WebDriver, selector: &str) -> Result<Option<WebElement>> {
        Ok(driver
            .find_all(By::Css(selector.to_owned()))
            .await?
            .into_iter()
            .next())
    }

    /// Observe the revised plan's single S1 endpoint. The caller owns the native
    /// monotonic clock; this helper deliberately performs no page-clock relay,
    /// paint observation, trace collection, or resource accounting.
    async fn observe_s1_baseline_endpoint(
        driver: &WebDriver,
        calibration_delay_ms: u64,
    ) -> Result<Option<WebElement>> {
        driver.enter_default_frame().await?;
        for _ in 0..2 {
            let Some(frame) = first(driver, "tonk-site > iframe").await? else {
                driver.enter_default_frame().await?;
                return Ok(None);
            };
            if !frame.is_displayed().await? {
                driver.enter_default_frame().await?;
                return Ok(None);
            }
            frame.enter_frame().await?;
        }
        let Some(headline) = first(driver, ".wp-h1").await? else {
            driver.enter_default_frame().await?;
            return Ok(None);
        };
        if !headline.is_displayed().await? || headline.text().await?.trim() != S1_EXPECTED_HEADLINE
        {
            driver.enter_default_frame().await?;
            return Ok(None);
        }

        if calibration_delay_ms > 0 {
            // WebDriver has entered the opaque inner realm. Document-start CDP
            // injection is not guaranteed to reach this out-of-process frame.
            let hidden_now = driver
                .execute(
                    r#"
                    if (globalThis.__tonkCalibrationDelay) return false;
                    const el = document.querySelector('.wp-h1');
                    const previous = el.style.getPropertyValue('visibility');
                    const priority = el.style.getPropertyPriority('visibility');
                    const delay = arguments[0];
                    el.style.setProperty('visibility', 'hidden', 'important');
                    const control = {requested_ms: delay, hidden_at_ms: performance.now()};
                    globalThis.__tonkCalibrationDelay = control;
                    setTimeout(() => {
                      if (previous) el.style.setProperty('visibility', previous, priority);
                      else el.style.removeProperty('visibility');
                      control.released_at_ms = performance.now();
                    }, delay);
                    return true;
                    "#,
                    vec![json!(calibration_delay_ms)],
                )
                .await?
                .json()
                .as_bool()
                .context("calibration visibility control returned no result")?;
            if hidden_now {
                driver.enter_default_frame().await?;
                return Ok(None);
            }
        }

        // The outer frame was already checked on the way in. Return directly
        // to it, and resolve the open shadow-root selector in one command.
        // Keep WebDriver's displayed/enabled semantics unchanged.
        driver.enter_parent_frame().await?;
        let share_result = driver
            .execute(
                "return document.querySelector('tonk-fab')?.shadowRoot?.querySelector('[data-cell=\"share\"]') || null",
                vec![],
            )
            .await?;
        if share_result.json().is_null() {
            driver.enter_default_frame().await?;
            return Ok(None);
        }
        let share = share_result.element()?;
        if !share.is_displayed().await? || !share.is_enabled().await? {
            driver.enter_default_frame().await?;
            return Ok(None);
        }
        Ok(Some(share))
    }

    async fn await_s1_baseline_share_menu(driver: &WebDriver) -> Result<bool> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ready = driver
                .execute(
                    r#"
                    const bar = document.querySelector('tonk-fab');
                    const cell = bar?.shadowRoot?.querySelector('[data-cell="share"]');
                    const menu = bar?.querySelector('#fabb-share-menu');
                    const account = bar?.querySelector('[data-share-account]');
                    const roster = bar?.querySelector('[data-share-members]');
                    const visible = element => !!element
                      && element.getClientRects().length > 0
                      && getComputedStyle(element).display !== 'none'
                      && getComputedStyle(element).visibility !== 'hidden';
                    return globalThis.__tonkBaselineTrustedClick === true
                      && cell?.getAttribute('aria-expanded') === 'true'
                      && !!menu && !menu.hasAttribute('hidden')
                      && visible(account) && account.textContent.trim().startsWith('log in to share')
                      && visible(roster) && roster.textContent.trim() === '1 member';
                    "#,
                    vec![],
                )
                .await?
                .json()
                .as_bool()
                == Some(true);
            if ready {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn sample_s1_observed_baseline(
        env: &TestEnvironment,
        request: &Value,
        output: &Path,
    ) -> Result<Value> {
        ensure!(
            std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
            "the frozen S1 baseline uses desktop Chrome"
        );
        ensure!(
            std::env::var("TONK_PERF_TRACE").as_deref() == Ok("0"),
            "the frozen S1 baseline requires tracing to be disabled"
        );
        let (width, height) = validate_request(request)?;
        ensure!(
            request["measurement"] == S1_BASELINE_MEASUREMENT,
            "unsupported S1 baseline measurement"
        );
        let driver = env
            .performance_driver()
            .await
            .context("S1 empty browser setup")?;
        let measured: Result<Value> = async {
            let cdp = ChromeDevTools::new(driver.handle.clone());
            cdp.execute_cdp_with_params(
                "Emulation.setDeviceMetricsOverride",
                json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
            )
            .await?;
            cdp.execute_cdp_with_params("Emulation.setCPUThrottlingRate", json!({"rate": 1}))
                .await?;
            cdp.execute_cdp("Network.enable").await?;
            cdp.execute_cdp_with_params(
                "Network.emulateNetworkConditions",
                json!({"offline": false, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}),
            )
            .await?;
            cdp.execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"}),
            )
            .await?;
            let browser = cdp.execute_cdp("Browser.getVersion").await?;

            // Calibration-only positive control: delay the actual headline's
            // visibility in its own realm. It changes no served artifact and
            // remains explicit in the request/result, never a product sample.
            let calibration_delay_ms = match request.get("calibration_headline_delay_ms") {
                None => 0,
                Some(value) => value.as_u64().context("invalid calibration delay")?,
            };
            ensure!(calibration_delay_ms <= 2000, "calibration delay exceeds 2s");

            let started = Instant::now();
            if let Err(error) = driver.goto(env.tonk_web.as_str()).await {
                if matches!(
                    error.as_inner(),
                    thirtyfour::error::WebDriverErrorInner::WebDriverTimeout(_)
                        | thirtyfour::error::WebDriverErrorInner::Timeout(_)
                ) {
                    return Ok(json!({"schema_version": 1, "status": "timeout",
                        "identity_verified": false, "metrics": {},
                        "failure_category": "navigation_timeout"}));
                }
                return Err(error).context("S1 navigation before readiness");
            }
            let navigation_returned_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut poll_durations_ms = Vec::new();
            let share = loop {
                let poll_started = Instant::now();
                let observed = observe_s1_baseline_endpoint(&driver, calibration_delay_ms)
                    .await
                    .context("S1 endpoint observation")?;
                poll_durations_ms.push(poll_started.elapsed().as_secs_f64() * 1000.0);
                if let Some(share) = observed {
                    break share;
                }
                if started.elapsed() >= S1_TIMEOUT {
                    return Ok(json!({"schema_version": 1, "status": "timeout",
                        "identity_verified": false, "metrics": {},
                        "failure_category": "observed_readiness_timeout"}));
                }
                tokio::time::sleep(S1_POLL_INTERVAL).await;
            };
            let navigation_to_usable_observed_ms = started.elapsed().as_secs_f64() * 1000.0;

            driver
                .execute(
                    r#"
                    globalThis.__tonkBaselineTrustedClick = false;
                    document.querySelector('tonk-fab')?.shadowRoot
                      ?.querySelector('[data-cell="share"]')
                      ?.addEventListener('click', event => {
                        globalThis.__tonkBaselineTrustedClick = event.isTrusted;
                      }, {once: true});
                    "#,
                    vec![],
                )
                .await?;
            share.click().await?;
            if !await_s1_baseline_share_menu(&driver).await? {
                driver.enter_default_frame().await?;
                let screenshot = output.with_extension("functional-failure.png");
                let screenshot_saved = driver.screenshot(&screenshot).await.is_ok();
                return Ok(json!({"schema_version": 1, "status": "functional_failure",
                    "identity_verified": false,
                    "metrics": {"navigation_to_usable_observed_ms": navigation_to_usable_observed_ms},
                    "failure_category": "trusted_share_menu_check_failed",
                    "functional_failure_screenshot_saved": screenshot_saved,
                    "functional_failure_screenshot": screenshot}));
            }

            driver.enter_default_frame().await?;
            let mut calibration_control = Value::Null;
            if calibration_delay_ms > 0 {
                for _ in 0..2 {
                    first(&driver, "tonk-site > iframe")
                        .await?
                        .context("calibration frame missing")?
                        .enter_frame()
                        .await?;
                }
                calibration_control = driver
                    .execute("return globalThis.__tonkCalibrationDelay || null", vec![])
                    .await?
                    .json()
                    .clone();
                let hidden = calibration_control["hidden_at_ms"]
                    .as_f64()
                    .context("positive control did not hide headline")?;
                let released = calibration_control["released_at_ms"]
                    .as_f64()
                    .context("positive control did not release headline")?;
                ensure!(
                    released - hidden >= calibration_delay_ms as f64 - 1.0,
                    "positive control released too early"
                );
                driver.enter_default_frame().await?;
            }
            let screenshot = output.with_extension("png");
            driver.screenshot(&screenshot).await?;
            let version = verify_artifact(env, &driver, width, height).await?;
            Ok(json!({
                "schema_version": 1,
                "status": "ok",
                "identity_verified": true,
                "fixture_verified": true,
                "profile_verified": true,
                "environment_verified": false,
                "measurement": S1_BASELINE_MEASUREMENT,
                "page_load_strategy": "none",
                "scenario": "S1",
                "profile": "desktop",
                "profile_settings": request["profile_settings"],
                "cache_state": "fresh-profile-empty-http-and-service-worker",
                "browser": browser,
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "build_id": version["build"],
                "artifact_version": version,
                "metrics": {
                    "navigation_to_usable_observed_ms": navigation_to_usable_observed_ms
                },
                "observer_diagnostics": {
                    "version": 1,
                    "navigation_returned_ms": navigation_returned_ms,
                    "poll_durations_ms": poll_durations_ms,
                },
                "calibration_headline_delay_ms": calibration_delay_ms,
                "calibration_control": calibration_control,
                "endpoint": {
                    "headline_frames": 2,
                    "headline_selector": ".wp-h1",
                    "headline_text": S1_EXPECTED_HEADLINE,
                    "share_selector": "tonk-fab::shadow [data-cell=share]",
                    "share_displayed_and_enabled": true,
                    "poll_interval_ms": S1_POLL_INTERVAL.as_millis(),
                    "timeout_ms": S1_TIMEOUT.as_millis()
                },
                "functional_check": {
                    "trusted_webdriver_click": true,
                    "share_menu_open": true,
                    "account_action": "log in to share",
                    "member_count": "1 member"
                },
                "screenshot": screenshot
            }))
        }
        .await;
        let quit = driver.quit().await;
        let result = measured?;
        quit?;
        Ok(result)
    }

    async fn observe_headline(
        driver: &WebDriver,
        collect_lcp: bool,
    ) -> Result<Option<HeadlineObservation>> {
        driver.enter_default_frame().await?;
        for _ in 0..2 {
            let Some(frame) = first(driver, "tonk-site > iframe").await? else {
                driver.enter_default_frame().await?;
                return Ok(None);
            };
            if !frame.is_displayed().await? {
                driver.enter_default_frame().await?;
                return Ok(None);
            }
            frame.enter_frame().await?;
        }
        // Opaque child documents should receive the pre-navigation CDP script;
        // execute the same idempotent helper here as a target-attachment fallback.
        driver.execute(FRAME_CLOCK_SCRIPT, vec![]).await?;
        let Some(headline) = first(driver, ".wp-h1").await? else {
            driver.enter_default_frame().await?;
            return Ok(None);
        };
        if !headline.is_displayed().await?
            || !headline.text().await?.contains("makes your small software")
        {
            driver.enter_default_frame().await?;
            return Ok(None);
        }
        // This runs after load so `buffered: true` is required. Retain no text,
        // URL, selector or arbitrary element data: the only persisted DOM fact
        // is how an LCP candidate relates to the already verified headline.
        let lcp = driver
            .execute_async(
                r#"
                const collectLcp = arguments[0];
                const done = arguments[arguments.length - 1];
                const headline = document.querySelector('.wp-h1');
                const exchangeClock = payload => {
                  const observedEpoch = performance.timeOrigin + performance.now();
                  const clock = globalThis.__tonkS1FrameClockExchange;
                  if (typeof clock !== 'function') {
                    done({...payload, observed_epoch_ms: observedEpoch,
                      clock_exchange: {available: false,
                        reason: 'frame clock helper unavailable'}});
                    return;
                  }
                  clock().then(clockExchange => done({...payload,
                    observed_epoch_ms: observedEpoch,
                    clock_exchange: clockExchange})).catch(() => done({...payload,
                      observed_epoch_ms: observedEpoch,
                      clock_exchange: {available: false,
                        reason: 'frame clock exchange failed'}}));
                };
                if (!collectLcp) {
                  exchangeClock({supported: false, reason: 'trace diagnostics disabled'});
                  return;
                }
                const supported = !!headline && globalThis.PerformanceObserver
                  ?.supportedEntryTypes?.includes('largest-contentful-paint');
                if (!supported) {
                  exchangeClock({supported: false, reason: headline
                    ? 'largest-contentful-paint unsupported'
                    : 'headline missing'});
                  return;
                }
                const relation = element => {
                  if (!element) return 'missing';
                  if (element === headline) return 'exact';
                  if (headline.contains(element)) return 'descendant';
                  if (element.contains(headline)) return 'ancestor';
                  return 'other';
                };
                const tagCategory = element => {
                  const tag = element?.tagName;
                  if (tag === 'IMG') return 'img';
                  if (tag === 'SVG') return 'svg';
                  if (tag === 'VIDEO') return 'video';
                  if (tag === 'CANVAS') return 'canvas';
                  if (['H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'P', 'DIV', 'SPAN',
                       'MAIN', 'SECTION', 'ARTICLE'].includes(tag)) return 'text-container';
                  return element ? 'other' : 'missing';
                };
                const geometry = element => {
                  if (!element || !headline) {
                    return {rectangle_relation: 'no_box', area_ratio_bucket: 'unavailable'};
                  }
                  const candidate = element.getBoundingClientRect();
                  const target = headline.getBoundingClientRect();
                  if (candidate.width <= 0 || candidate.height <= 0
                      || target.width <= 0 || target.height <= 0) {
                    return {rectangle_relation: 'no_box', area_ratio_bucket: 'unavailable'};
                  }
                  const epsilon = 1;
                  const same = Math.abs(candidate.left - target.left) <= epsilon
                    && Math.abs(candidate.top - target.top) <= epsilon
                    && Math.abs(candidate.right - target.right) <= epsilon
                    && Math.abs(candidate.bottom - target.bottom) <= epsilon;
                  const contains = candidate.left <= target.left + epsilon
                    && candidate.top <= target.top + epsilon
                    && candidate.right >= target.right - epsilon
                    && candidate.bottom >= target.bottom - epsilon;
                  const inside = target.left <= candidate.left + epsilon
                    && target.top <= candidate.top + epsilon
                    && target.right >= candidate.right - epsilon
                    && target.bottom >= candidate.bottom - epsilon;
                  const overlaps = Math.min(candidate.right, target.right)
                      > Math.max(candidate.left, target.left)
                    && Math.min(candidate.bottom, target.bottom)
                      > Math.max(candidate.top, target.top);
                  const ratio = candidate.width * candidate.height
                    / (target.width * target.height);
                  return {
                    rectangle_relation: same ? 'same_rect'
                      : contains ? 'contains'
                      : inside ? 'inside'
                      : overlaps ? 'overlaps' : 'disjoint',
                    area_ratio_bucket: ratio < 0.25 ? 'lt_quarter'
                      : ratio < 1 ? 'quarter_to_one'
                      : ratio < 4 ? 'one_to_four' : 'gte_four'
                  };
                };
                const candidates = [];
                let candidateCount = 0;
                let matchingCandidateCount = 0;
                let finalNodeMatch = 'none';
                let headlineCandidate = null;
                let observer;
                const finish = () => {
                  observer?.disconnect();
                  exchangeClock({supported: true, candidate_count: candidateCount,
                    candidates_truncated: candidateCount > candidates.length,
                    candidates, matching_candidate_count: matchingCandidateCount,
                    final_node_match: finalNodeMatch,
                    headline_candidate: headlineCandidate});
                };
                try {
                  observer = new PerformanceObserver(list => {
                    for (const entry of list.getEntries()) {
                      const presentation = entry.renderTime > 0
                        ? entry.renderTime : entry.startTime;
                      const nodeMatch = relation(entry.element);
                      const candidate = {node_match: nodeMatch,
                        tag_category: tagCategory(entry.element),
                        ...geometry(entry.element),
                        start_time_ms: entry.startTime,
                        render_time_ms: entry.renderTime,
                        load_time_ms: entry.loadTime,
                        size: entry.size,
                        presentation_basis: entry.renderTime > 0 ? 'renderTime' : 'startTime',
                        presentation_time_ms: presentation,
                        epoch_ms: performance.timeOrigin + presentation};
                      candidateCount += 1;
                      finalNodeMatch = nodeMatch;
                      if (nodeMatch === 'exact' || nodeMatch === 'descendant') {
                        matchingCandidateCount += 1;
                        headlineCandidate = candidate;
                      }
                      candidates.push(candidate);
                      if (candidates.length > 8) candidates.shift();
                    }
                  });
                  observer.observe({type: 'largest-contentful-paint', buffered: true});
                  setTimeout(finish, 0);
                } catch (_) {
                  observer?.disconnect();
                  exchangeClock({supported: false, reason: 'observer construction failed'});
                }
                "#,
                vec![json!(collect_lcp)],
            )
            .await?
            .json()
            .clone();
        let observed_epoch_ms = lcp["observed_epoch_ms"]
            .as_f64()
            .filter(|sample| sample.is_finite() && *sample >= 0.0)
            .context("headline observation returned no finite same-frame epoch")?;
        let clock_conversion = frame_clock_conversion(&lcp)?;
        driver.enter_default_frame().await?;
        Ok(Some(HeadlineObservation {
            observed_epoch_ms,
            clock_conversion,
            lcp,
        }))
    }

    async fn enter_guest(driver: &WebDriver) -> Result<bool> {
        driver.enter_default_frame().await?;
        let Some(frame) = first(driver, "tonk-site > iframe").await? else {
            return Ok(false);
        };
        if !frame.is_displayed().await? {
            return Ok(false);
        }
        frame.enter_frame().await?;
        Ok(true)
    }

    async fn prepare_s1_input(
        driver: &WebDriver,
    ) -> Result<Option<(WebElement, UsableObservation)>> {
        if !enter_guest(driver).await? {
            return Ok(None);
        }
        driver.execute(FRAME_CLOCK_SCRIPT, vec![]).await?;
        let prepared = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                const bar = document.querySelector('tonk-fab');
                const cell = bar?.shadowRoot?.querySelector('[data-cell="share"]');
                const row = bar?.querySelector('[data-share-account]');
                const inner = row?.shadowRoot?.querySelector('.row');
                if (!cell || !row || row.hasAttribute('hidden') || !inner) {
                  done(null);
                  return;
                }
                const state = {entries: [], trusted: false, trustedClickStart: null};
                cell.addEventListener('click', event => {
                  if (event.isTrusted) {
                    state.trusted = true;
                    state.trustedClickStart = event.timeStamp;
                  }
                }, {once: true});
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
                globalThis.__tonkS1Input = state;
                const observedEpoch = performance.timeOrigin + performance.now();
                const clock = globalThis.__tonkS1FrameClockExchange;
                if (typeof clock !== 'function') {
                  done({observed_epoch_ms: observedEpoch,
                    clock_exchange: {available: false,
                      reason: 'frame clock helper unavailable'}});
                  return;
                }
                clock().then(clockExchange => done({observed_epoch_ms: observedEpoch,
                  clock_exchange: clockExchange})).catch(() => done({
                    observed_epoch_ms: observedEpoch,
                    clock_exchange: {available: false,
                      reason: 'frame clock exchange failed'}}));
                "#,
                vec![],
            )
            .await?;
        if prepared.json().is_null() {
            return Ok(None);
        }
        let observed_epoch_ms = prepared.json()["observed_epoch_ms"]
            .as_f64()
            .filter(|sample| sample.is_finite() && *sample >= 0.0)
            .context("usable control returned no finite same-frame epoch")?;
        let clock_conversion = frame_clock_conversion(prepared.json())?;

        // Resolve the element in the same guest context where the input
        // observer and clock exchange were installed.
        let Some(bar) = first(driver, "tonk-fab").await? else {
            return Ok(None);
        };
        let shadow = bar.get_shadow_root().await?;
        let Some(share) = shadow
            .find_all(By::Css("[data-cell=\"share\"]"))
            .await?
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        if !share.is_displayed().await? || !share.is_enabled().await? {
            return Ok(None);
        }
        Ok(Some((
            share,
            UsableObservation {
                observed_epoch_ms,
                clock_conversion,
            },
        )))
    }

    fn validate_input_endpoint(observation: &Value) -> Result<()> {
        ensure!(
            observation["trusted"] == true,
            "share control did not receive a trusted click"
        );
        ensure!(
            observation["trusted_click_start_ms"]
                .as_f64()
                .is_some_and(|timestamp| timestamp.is_finite() && timestamp >= 0.0),
            "trusted click has no finite frame timestamp"
        );
        ensure!(
            observation["expanded"] == true && observation["menu_contents_ready"] == true,
            "trusted click did not open ready share-menu contents"
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

    async fn await_s1_input(driver: &WebDriver) -> Result<Option<Value>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut endpoint_seen = None;
        loop {
            let observation = driver
                .execute(
                    r#"
                    const state = globalThis.__tonkS1Input;
                    const bar = document.querySelector('tonk-fab');
                    const cell = bar?.shadowRoot?.querySelector('[data-cell="share"]');
                    const row = bar?.querySelector('[data-share-account]');
                    const inner = row?.shadowRoot?.querySelector('.row');
                    const slotHasText = !!row && row.textContent.trim().length > 0;
                    const innerVisible = !!inner && inner.getClientRects().length > 0
                      && getComputedStyle(inner).visibility !== 'hidden'
                      && getComputedStyle(inner).display !== 'none';
                    const visible = innerVisible && slotHasText;
                    return {trusted: state?.trusted === true,
                      trusted_click_start_ms: state?.trustedClickStart,
                      cell_exists: !!cell,
                      expanded: cell?.getAttribute('aria-expanded') === 'true',
                      row_exists: !!row,
                      row_hidden: row?.hasAttribute('hidden') ?? null,
                      inner_exists: !!inner,
                      inner_has_box: !!inner && inner.getClientRects().length > 0,
                      inner_visible: innerVisible,
                      slot_has_text: slotHasText,
                      menu_contents_ready: visible,
                      observed_epoch_ms: performance.timeOrigin + performance.now(),
                      entries: state?.entries || []};
                    "#,
                    vec![],
                )
                .await?
                .json()
                .clone();
            if validate_input_endpoint(&observation).is_ok() {
                let seen = endpoint_seen.get_or_insert_with(Instant::now);
                if input_event_timing(&observation).is_some()
                    || seen.elapsed() >= Duration::from_secs(1)
                {
                    return Ok(Some(observation));
                }
            }
            if Instant::now() >= deadline {
                return Ok(Some(observation));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn validate_request(request: &Value) -> Result<(u64, u64)> {
        ensure!(request["schema_version"] == 1, "unsupported request schema");
        ensure!(request["scenario"] == "S1", "only S1 is implemented");
        let fixture = &request["fixture"];
        ensure!(
            fixture["recipe"] == "fresh-anonymous-nested-welcome-v1"
                && fixture["selector"] == ".wp-h1"
                && fixture["input_sequence"] == "open-share-menu-v1"
                && fixture["cache_state"] == "fresh-profile-empty-http-and-service-worker"
                && fixture["dataset_sha256"]
                    == "ef4c970fce9dc639ad8770e8ee025de886e753fdd942f451400ec85bf6e5e55a",
            "unsupported fixture contract; update and revalidate the runner for fixture changes"
        );
        let settings = &request["profile_settings"];
        // Refuse a silently substituted profile. CDP acknowledges each setting;
        // process/worker-wide constrained-profile validation is still outstanding.
        ensure!(
            request["profile"] == "desktop",
            "constrained profile is unavailable until worker/child throttling is validated"
        );
        ensure!(
            settings["cpu_slowdown"] == 1 && settings["latency_ms"] == 0,
            "desktop profile must be unthrottled"
        );
        ensure!(
            settings["download_bytes_per_second"] == -1
                && settings["upload_bytes_per_second"] == -1,
            "desktop throughput must be unlimited (-1)"
        );
        let width = settings["viewport_width"]
            .as_u64()
            .context("viewport_width required")?;
        let height = settings["viewport_height"]
            .as_u64()
            .context("viewport_height required")?;
        ensure!(
            (320..=7680).contains(&width) && (240..=4320).contains(&height),
            "viewport outside supported bounds"
        );
        Ok((width, height))
    }

    fn contains_string(value: &Value, needle: &str) -> bool {
        match value {
            Value::String(value) => value == needle,
            Value::Array(values) => values.iter().any(|value| contains_string(value, needle)),
            Value::Object(values) => values.values().any(|value| contains_string(value, needle)),
            _ => false,
        }
    }

    fn clock_alignment(trace_timestamp_ms: Option<f64>, bracket: &ClockBracket) -> Value {
        let midpoint = (bracket.before_epoch_ms + bracket.after_epoch_ms) / 2.0;
        let uncertainty = (bracket.after_epoch_ms - bracket.before_epoch_ms) / 2.0;
        match trace_timestamp_ms.filter(|timestamp| timestamp.is_finite() && *timestamp >= 0.0) {
            Some(timestamp) => json!({
                "available": true,
                "method": "Tracing.recordClockSyncMarker bracketed by iframe timeOrigin + now",
                "trace_timestamp_ms": timestamp,
                "frame_epoch_midpoint_ms": midpoint,
                "trace_minus_epoch_ms": timestamp - midpoint,
                "uncertainty_ms": uncertainty,
            }),
            None => json!({
                "available": false,
                "reason": "matching trace clock-sync marker missing",
                "frame_epoch_midpoint_ms": midpoint,
                "uncertainty_ms": uncertainty,
            }),
        }
    }

    fn headline_trace_alignment(
        lcp_epoch_ms: Option<f64>,
        sync_timestamp_ms: Option<f64>,
        bracket: &ClockBracket,
        paints: &[Value],
        trace_lcp_candidates: &[Value],
        truncated: bool,
    ) -> Value {
        const MAX_NEARBY_TRACE_DELTA_MS: f64 = 20.0;
        let Some(lcp_epoch_ms) = lcp_epoch_ms else {
            return json!({"available": false,
                "reason": "no LCP candidate matched the nested .wp-h1 or its descendant"});
        };
        let Some(sync_timestamp_ms) =
            sync_timestamp_ms.filter(|timestamp| timestamp.is_finite() && *timestamp >= 0.0)
        else {
            return json!({"available": false,
                "reason": "matching trace clock-sync marker missing",
                "lcp_epoch_ms": lcp_epoch_ms});
        };
        let midpoint = (bracket.before_epoch_ms + bracket.after_epoch_ms) / 2.0;
        let uncertainty_ms = (bracket.after_epoch_ms - bracket.before_epoch_ms) / 2.0;
        let lcp_trace_timestamp_ms = lcp_epoch_ms + sync_timestamp_ms - midpoint;
        let nearest = |events: &[Value], required_name: Option<&str>| {
            events
                .iter()
                .filter(|event| required_name.is_none_or(|name| event["name"] == name))
                .filter_map(|event| {
                    let timestamp = event["trace_timestamp_ms"]
                        .as_f64()
                        .filter(|timestamp| timestamp.is_finite() && *timestamp >= 0.0)?;
                    Some((timestamp, (timestamp - lcp_trace_timestamp_ms).abs()))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1))
        };
        let nearest_paint = nearest(paints, Some("Paint"));
        let nearest_lcp = nearest(trace_lcp_candidates, None);
        let nearby = |candidate: Option<(f64, f64)>| {
            !truncated
                && uncertainty_ms <= MAX_OBSERVED_CLOCK_UNCERTAINTY_MS
                && candidate.is_some_and(|(_, delta)| delta <= MAX_NEARBY_TRACE_DELTA_MS)
        };
        json!({
            "available": true,
            "method": "matched headline LCP epoch mapped through clock-sync marker to nearby trace events",
            "lcp_epoch_ms": lcp_epoch_ms,
            "lcp_trace_timestamp_ms": lcp_trace_timestamp_ms,
            "nearest_trace_paint_ms": nearest_paint.map(|candidate| candidate.0),
            "nearest_trace_paint_delta_ms": nearest_paint.map(|candidate| candidate.1),
            "nearby_trace_paint": nearby(nearest_paint),
            "nearest_trace_lcp_candidate_ms": nearest_lcp.map(|candidate| candidate.0),
            "nearest_trace_lcp_candidate_delta_ms": nearest_lcp.map(|candidate| candidate.1),
            "nearby_trace_lcp_candidate": nearby(nearest_lcp),
            "maximum_nearby_trace_delta_ms": MAX_NEARBY_TRACE_DELTA_MS,
            "clock_uncertainty_ms": uncertainty_ms,
            "trace_truncated": truncated,
            "attribution": "temporal proximity only; trace events are not attributed to the headline node",
        })
    }

    // Discard URLs, request identifiers, headers, frame names and arbitrary trace
    // args before persisting. These logs can contain disposable identity routes.
    fn summarize_logs(
        entries: &[Value],
        bracket: &ClockBracket,
        headline_lcp_epoch_ms: Option<f64>,
    ) -> Value {
        const LIMIT: usize = 50_000;
        let mut paints = Vec::new();
        let mut trace_lcp_candidates = Vec::new();
        let mut requests = 0_u64;
        let mut bytes = 0.0_f64;
        let mut sync_timestamp_ms = None;
        let mut overflow = entries.len() > LIMIT;
        for entry in entries.iter().take(LIMIT) {
            let Some(raw) = entry["message"].as_str() else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(raw) else {
                continue;
            };
            let event = &message["message"];
            match event["method"].as_str() {
                Some("Network.loadingFinished") => {
                    requests += 1;
                    if let Some(length) = event["params"]["encodedDataLength"].as_f64()
                        && length.is_finite()
                        && length >= 0.0
                    {
                        bytes += length;
                    }
                }
                Some("Tracing.bufferUsage") => {
                    overflow |= event["params"]["percentFull"]
                        .as_f64()
                        .is_some_and(|n| n >= 1.0);
                }
                Some("Tracing.dataCollected") => {
                    let params = &event["params"];
                    // ChromeDriver reports each trace event separately; accept
                    // the CDP batch shape too for deterministic replay tests.
                    let events = params["value"]
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| vec![params.clone()]);
                    for trace in events {
                        let name = trace["name"].as_str().unwrap_or("");
                        let normalized = name.to_ascii_lowercase().replace(['_', '-'], "");
                        if normalized.contains("clocksync")
                            && contains_string(&trace["args"], CLOCK_SYNC_ID)
                            && let Some(ts) = trace["ts"].as_f64()
                            && ts.is_finite()
                            && ts >= 0.0
                        {
                            sync_timestamp_ms = Some(ts / 1000.0);
                        }
                        if matches!(
                            name,
                            "Paint" | "DrawFrame" | "FramePresented" | "CompositeLayers"
                        ) && let Some(ts) = trace["ts"].as_f64()
                            && ts.is_finite()
                            && ts >= 0.0
                        {
                            if paints.len() == LIMIT {
                                overflow = true;
                                break;
                            }
                            paints.push(json!({"name": name, "trace_timestamp_ms": ts / 1000.0}));
                        }
                        if normalized.contains("largestcontentfulpaint")
                            && let Some(ts) = trace["ts"].as_f64()
                            && ts.is_finite()
                            && ts >= 0.0
                        {
                            if trace_lcp_candidates.len() == LIMIT {
                                overflow = true;
                                break;
                            }
                            trace_lcp_candidates
                                .push(json!({"name": name, "trace_timestamp_ms": ts / 1000.0}));
                        }
                    }
                }
                _ => {}
            }
        }
        let clock_sync = clock_alignment(sync_timestamp_ms, bracket);
        let headline_lcp_trace = headline_trace_alignment(
            headline_lcp_epoch_ms,
            sync_timestamp_ms,
            bracket,
            &paints,
            &trace_lcp_candidates,
            overflow,
        );
        json!({"scope": "ChromeDriver performance-log targets; worker/OOPIF coverage unverified",
            "clock": "Chrome trace monotonic microseconds converted to milliseconds",
            "clock_sync": clock_sync,
            "headline_lcp_trace": headline_lcp_trace,
            "truncated": overflow,
            "network_events_available": requests > 0, "paint_events_available": !paints.is_empty(),
            "completed_requests": if requests > 0 { Some(requests) } else { None },
            "encoded_bytes": if requests > 0 { Some(bytes) } else { None },
            "paint_events": paints,
            "trace_lcp_candidates": trace_lcp_candidates,
            "presentation_endpoint_verified": false})
    }

    async fn drain_diagnostics(
        env: &TestEnvironment,
        driver: &WebDriver,
        bracket: &ClockBracket,
        headline_lcp_epoch_ms: Option<f64>,
    ) -> Result<Value> {
        let url = env
            .chromedriver
            .join(&format!("session/{}/se/log", driver.handle.session_id()))?;
        let client = reqwest::Client::new();
        let mut response = client
            .post(url)
            .json(&json!({"type": "performance"}))
            .send()
            .await?;
        // Legacy ChromeDriver used the JSON Wire endpoint. Only an absent
        // endpoint permits this fallback; never mask a log/transport failure.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            let legacy = env
                .chromedriver
                .join(&format!("session/{}/log", driver.handle.session_id()))?;
            response = client
                .post(legacy)
                .json(&json!({"type": "performance"}))
                .send()
                .await?;
        }
        let response = response.error_for_status()?;
        let logs: Value = response.json().await?;
        let entries = logs["value"]
            .as_array()
            .context("ChromeDriver returned no performance log")?;
        Ok(summarize_logs(entries, bracket, headline_lcp_epoch_ms))
    }

    async fn sample(env: &TestEnvironment, request: &Value, output: &Path) -> Result<Value> {
        ensure!(
            std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
            "Chrome CDP is required; Safari metrics are unavailable"
        );
        let (width, height) = validate_request(request)?;
        let settings = &request["profile_settings"];
        let driver = env.blank_driver().await?;
        let measured: Result<Value> = async {
            let cdp = ChromeDevTools::new(driver.handle.clone());
            cdp.execute_cdp_with_params("Emulation.setDeviceMetricsOverride", json!({
                "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false
            })).await?;
            cdp.execute_cdp_with_params("Emulation.setCPUThrottlingRate", json!({"rate": 1})).await?;
            cdp.execute_cdp("Network.enable").await?;
            cdp.execute_cdp_with_params("Network.emulateNetworkConditions", json!({
                "offline": false, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1
            })).await?;
            // Stop telemetry before app code runs, including on the first-use route.
            cdp.execute_cdp_with_params("Page.addScriptToEvaluateOnNewDocument", json!({
                "source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"
            })).await?;
            // Install the same content-free clock relay in trace-on and trace-off
            // samples before navigation. Only LCP collection is trace-gated.
            cdp.execute_cdp_with_params("Page.addScriptToEvaluateOnNewDocument", json!({
                "source": FRAME_CLOCK_SCRIPT
            })).await?;
            let browser = cdp.execute_cdp("Browser.getVersion").await?;
            // LCP observation and trace correlation remain opt-in. The small
            // clock relay above is fixed in both modes for future calibration.
            let diagnostics_enabled = std::env::var("TONK_PERF_TRACE").as_deref() != Ok("0");
            let started = Instant::now();
            // No retry: a failed first navigation remains a failed sample.
            if let Err(error) = driver.goto(env.tonk_web.as_str()).await {
                if matches!(error.as_inner(),
                    thirtyfour::error::WebDriverErrorInner::WebDriverTimeout(_)
                    | thirtyfour::error::WebDriverErrorInner::Timeout(_)) {
                    return Ok(json!({"status": "timeout", "identity_verified": false,
                        "metrics": {}, "failure_category": "navigation_timeout"}));
                }
                return Err(error.into());
            }
            driver.enter_default_frame().await?;
            let navigation_epoch_ms = driver
                .execute("return performance.timeOrigin", vec![])
                .await?
                .json()
                .as_f64()
                .filter(|sample| sample.is_finite() && *sample >= 0.0)
                .context("top document returned no finite timeOrigin")?;
            let headline = loop {
                if let Some(observation) =
                    observe_headline(&driver, diagnostics_enabled).await?
                {
                    break observation;
                }
                if started.elapsed() > Duration::from_secs(120) {
                    return Ok(json!({"status": "timeout", "identity_verified": false,
                        "metrics": {}, "failure_category": "nested_headline_timeout"}));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            let headline_lcp_epoch_ms =
                headline_lcp_epoch(&headline.lcp, headline.observed_epoch_ms);
            let headline_clock_uncertainty_ms = headline
                .clock_conversion
                .as_ref()
                .map(|conversion| conversion.uncertainty_ms);
            let (share, usable_clock) = loop {
                if let Some(endpoint) = prepare_s1_input(&driver).await? {
                    break endpoint;
                }
                if started.elapsed() > Duration::from_secs(120) {
                    return Ok(json!({"status": "timeout", "identity_verified": false,
                        "metrics": {}, "failure_category": "usable_control_timeout"}));
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            };
            let usable_clock_uncertainty_ms = usable_clock
                .clock_conversion
                .as_ref()
                .map(|conversion| conversion.uncertainty_ms);
            // Run the identical external endpoint without trace collection to
            // calibrate overhead. This flag never changes production code.
            let bracket = if diagnostics_enabled {
                let before = frame_epoch_ms(&driver).await?;
                cdp.execute_cdp_with_params(
                    "Tracing.recordClockSyncMarker",
                    json!({"syncId": CLOCK_SYNC_ID}),
                )
                .await?;
                let after = frame_epoch_ms(&driver).await?;
                Some(clock_bracket(before, after)?)
            } else {
                None
            };
            // WebDriver's element-click endpoint is the repository's proven
            // path for this shadow control. The in-frame listener below still
            // requires the resulting DOM click to report `isTrusted`.
            share.click().await?;
            let Some(input) = await_s1_input(&driver).await? else {
                return Ok(json!({"status": "timeout", "identity_verified": false,
                    "metrics": {}, "failure_category": "trusted_input_menu_timeout"}));
            };
            if validate_input_endpoint(&input).is_err() {
                let diagnostics = if diagnostics_enabled {
                    drain_diagnostics(
                        env,
                        &driver,
                        bracket.as_ref().expect("trace bracket"),
                        headline_lcp_epoch_ms,
                    )
                    .await?
                } else {
                    json!({"available": false, "reason": "disabled for overhead calibration",
                        "clock_sync": {"available": false, "reason": "trace diagnostics disabled"}})
                };
                driver.enter_default_frame().await?;
                let timeout_screenshot = output.with_extension("timeout.png");
                let screenshot_saved = driver.screenshot(&timeout_screenshot).await.is_ok();
                return Ok(json!({"status": "timeout", "identity_verified": false,
                    "metrics": {}, "failure_category": "trusted_input_menu_timeout",
                    "input_state": {
                        "trusted": input["trusted"], "cell_exists": input["cell_exists"],
                        "expanded": input["expanded"], "row_exists": input["row_exists"],
                        "row_hidden": input["row_hidden"], "inner_exists": input["inner_exists"],
                        "inner_has_box": input["inner_has_box"],
                        "inner_visible": input["inner_visible"],
                        "slot_has_text": input["slot_has_text"],
                        "menu_contents_ready": input["menu_contents_ready"],
                        "event_timing_entries": input["entries"].as_array().map_or(0, Vec::len)
                    },
                    "diagnostics": diagnostics,
                    "timeout_screenshot_saved": screenshot_saved,
                    "timeout_screenshot": timeout_screenshot}));
            }
            let first_input_ms = input_event_timing(&input);
            let input_observed_epoch_ms = input["observed_epoch_ms"]
                .as_f64()
                .filter(|sample| sample.is_finite() && *sample >= usable_clock.observed_epoch_ms)
                .context("input endpoint clock preceded usable control")?;
            let diagnostics = if diagnostics_enabled {
                drain_diagnostics(
                    env,
                    &driver,
                    bracket.as_ref().expect("trace bracket"),
                    headline_lcp_epoch_ms,
                )
                .await?
            } else {
                json!({"available": false, "reason": "disabled for overhead calibration",
                    "clock_sync": {"available": false, "reason": "trace diagnostics disabled"}})
            };
            let headline_lcp_observed_ms = match headline_lcp_epoch_ms {
                Some(epoch) => observed_from_clock_conversion(
                    navigation_epoch_ms,
                    epoch,
                    headline.clock_conversion.as_ref(),
                )?,
                None => None,
            };
            // All screenshots, reads and identity fetches happen after the endpoint.
            driver.enter_default_frame().await?;
            driver.screenshot(&output.with_extension("png")).await?;
            let version = verify_artifact(env, &driver, width, height).await?;
            let mut unavailable = json!({
                "navigation_to_usable_ms": "usable DOM control is validated but its presentation timestamp is not",
                "requests": "all-target Network collection not implemented",
                "bytes": "all-target Network collection not implemented"
            });
            if first_input_ms.is_none() {
                unavailable["first_input_ms"] = json!(
                    "trusted input completed but this browser emitted no qualifying Event Timing interaction"
                );
            }
            let headline_observed_ms = observed_from_clock_conversion(
                navigation_epoch_ms,
                headline.observed_epoch_ms,
                headline.clock_conversion.as_ref(),
            )?;
            let usable_observed_ms = observed_from_clock_conversion(
                navigation_epoch_ms,
                usable_clock.observed_epoch_ms,
                usable_clock.clock_conversion.as_ref(),
            )?;
            if headline_observed_ms.is_none() {
                unavailable["navigation_to_headline_observed_ms"] =
                    if headline.clock_conversion.is_some() {
                        json!("same-frame clock exchange exceeded the 25ms uncertainty ceiling")
                    } else {
                        json!("same-frame cross-realm clock exchange unavailable")
                    };
            }
            if usable_observed_ms.is_none() {
                unavailable["navigation_to_usable_observed_ms"] =
                    if usable_clock.clock_conversion.is_some() {
                        json!("same-frame clock exchange exceeded the 25ms uncertainty ceiling")
                    } else {
                        json!("same-frame cross-realm clock exchange unavailable")
                    };
            }
            if headline_lcp_observed_ms.is_none() {
                unavailable["navigation_to_headline_lcp_observed_ms"] = if diagnostics_enabled {
                    json!(if headline_lcp_epoch_ms.is_none() {
                        "no LCP candidate matched the nested .wp-h1 or its descendant"
                    } else if headline.clock_conversion.is_none() {
                        "same-frame cross-realm clock exchange unavailable"
                    } else {
                        "same-frame clock exchange exceeded the 25ms uncertainty ceiling"
                    })
                } else {
                    json!("trace diagnostics disabled")
                };
            }
            Ok(json!({
                "schema_version": 1, "status": "ok", "identity_verified": true,
                "profile_verified": true, "environment_verified": false,
                "fixture_verified": false,
                "environment": {"browser": browser, "os": std::env::consts::OS,
                    "architecture": std::env::consts::ARCH, "hardware": null,
                    "power_mode": null, "compression": null, "server": "repository Caddy HTTPS test server"},
                "build_id": version["build"], "browser": browser, "diagnostics": diagnostics,
                "diagnostics_enabled": diagnostics_enabled,
                "scenario": "S1", "profile": "desktop", "profile_settings": settings,
                "cache_state": "fresh-profile-empty-http-and-service-worker",
                "metrics": {"navigation_to_headline_observed_ms": headline_observed_ms,
                    "navigation_to_headline_lcp_observed_ms": headline_lcp_observed_ms,
                    "navigation_to_usable_observed_ms": usable_observed_ms,
                    "navigation_to_usable_ms": null, "first_input_ms": first_input_ms,
                    "first_input_event_timing_ms": first_input_ms,
                    "requests": null, "bytes": null},
                "unavailable": unavailable,
                "endpoint": {"headline": "two opaque guest frames; displayed .wp-h1 with frozen Welcome text",
                    "headline_lcp": headline.lcp,
                    "headline_lcp_candidate_matched": headline_lcp_epoch_ms.is_some(),
                    "nearby_trace_paint": diagnostics["headline_lcp_trace"]["nearby_trace_paint"],
                    "nearby_trace_lcp_candidate": diagnostics["headline_lcp_trace"]["nearby_trace_lcp_candidate"],
                    "usable_control": "first guest tonk-fab share cell displayed and enabled",
                    "trusted_input": "WebDriver element click opened visible anonymous share-menu contents",
                    "input_verified": true, "menu_contents_ready": true,
                    "input_observed_epoch_ms": input_observed_epoch_ms,
                    "presentation_endpoint_verified": false},
                "clock": "cross-frame timeOrigin + now validated by a same-frame postMessage exchange; trace mapping uses a bracketed Chrome clock-sync marker",
                "clock_validation": {
                    "headline_cross_realm_uncertainty_ms": headline_clock_uncertainty_ms,
                    "headline_same_frame_exchange": headline.clock_conversion.is_some(),
                    "usable_cross_realm_uncertainty_ms": usable_clock_uncertainty_ms,
                    "headline_ordering_validated": headline.clock_conversion.is_some(),
                    "usable_same_frame_exchange": usable_clock.clock_conversion.is_some(),
                    "usable_ordering_validated": usable_clock.clock_conversion.is_some()
                },
                "screenshot": output.with_extension("png")
            }))
        }.await;
        let quit = driver.quit().await;
        let result = measured?;
        quit?;
        Ok(result)
    }

    #[test]
    fn request_rejects_silent_profile_substitution() {
        let mut request = json!({"schema_version": 1, "scenario": "S1", "profile": "desktop",
            "fixture": {"recipe": "fresh-anonymous-nested-welcome-v1", "selector": ".wp-h1",
                "input_sequence": "open-share-menu-v1", "cache_state": "fresh-profile-empty-http-and-service-worker",
                "dataset_sha256": "ef4c970fce9dc639ad8770e8ee025de886e753fdd942f451400ec85bf6e5e55a"},
            "profile_settings": {"cpu_slowdown": 1, "latency_ms": 0,
                "download_bytes_per_second": -1, "upload_bytes_per_second": -1,
                "viewport_width": 1200, "viewport_height": 900}});
        assert_eq!(validate_request(&request).unwrap(), (1200, 900));
        request["fixture"]["selector"] = json!("body");
        assert!(validate_request(&request).is_err());
        request["fixture"]["selector"] = json!(".wp-h1");
        request["profile_settings"]["cpu_slowdown"] = json!(4);
        assert!(validate_request(&request).is_err());
        request["profile_settings"]["cpu_slowdown"] = json!(1);
        request["profile"] = json!("constrained");
        assert!(validate_request(&request).is_err());
        request["profile"] = json!("desktop");
        request["scenario"] = json!("S2");
        assert!(validate_request(&request).is_err());
    }

    #[test]
    fn artifact_copy_rejects_changed_and_missing_nested_assets() -> Result<()> {
        let source = tempfile::tempdir()?;
        let copy = tempfile::tempdir()?;
        for root in [source.path(), copy.path()] {
            std::fs::create_dir(root.join("guest"))?;
            std::fs::write(root.join("guest/runtime.wasm"), [0, 97, 115, 109])?;
        }
        verify_copy(source.path(), copy.path())?;
        std::fs::write(copy.path().join("guest/runtime.wasm"), [0, 97, 115, 110])?;
        assert!(verify_copy(source.path(), copy.path()).is_err());
        std::fs::remove_file(copy.path().join("guest/runtime.wasm"))?;
        assert!(verify_copy(source.path(), copy.path()).is_err());
        std::fs::write(copy.path().join("guest/runtime.wasm"), [0, 97, 115, 109])?;
        std::fs::write(copy.path().join("unexpected.js"), b"extra")?;
        assert!(verify_copy(source.path(), copy.path()).is_err());
        Ok(())
    }

    #[test]
    fn logs_strip_content_and_preserve_trace_clock_units() {
        let wrap = |event: Value| json!({"message": json!({"message": event}).to_string()});
        let logs = vec![
            wrap(
                json!({"method": "Network.loadingFinished", "params": {"requestId": "secret", "encodedDataLength": 42}}),
            ),
            wrap(
                json!({"method": "Network.requestWillBeSent", "params": {"url": "https://secret.invalid/private"}}),
            ),
            wrap(
                json!({"method": "Tracing.dataCollected", "params": {"name": "Paint", "ts": 1234500, "args": {"text": "private"}}}),
            ),
            wrap(
                json!({"method": "Tracing.dataCollected", "params": {"value": [{"name": "DrawFrame", "ts": 1236500}]}}),
            ),
            wrap(
                json!({"method": "Tracing.dataCollected", "params": {"name": "largestContentfulPaint::Candidate", "ts": 1234500, "args": {"nodeId": "private"}}}),
            ),
            wrap(json!({"method": "Tracing.dataCollected", "params": {
                "name": "ClockSync", "ts": 1235500,
                "args": {"sync_id": CLOCK_SYNC_ID, "private": "do not retain"}
            }})),
        ];
        let bracket = clock_bracket(1_700_000_000_000.0, 1_700_000_000_004.0).unwrap();
        let summary = summarize_logs(&logs, &bracket, Some(1_700_000_000_001.0));
        assert_eq!(summary["completed_requests"], 1);
        assert_eq!(summary["encoded_bytes"], 42.0);
        assert_eq!(summary["paint_events"][0]["trace_timestamp_ms"], 1234.5);
        assert_eq!(summary["paint_events"][1]["trace_timestamp_ms"], 1236.5);
        assert!(!summary.to_string().contains("secret"));
        assert!(!summary.to_string().contains("private"));
        assert_eq!(summary["presentation_endpoint_verified"], false);
        assert_eq!(summary["clock_sync"]["available"], true);
        assert_eq!(summary["clock_sync"]["trace_timestamp_ms"], 1235.5);
        assert_eq!(summary["clock_sync"]["uncertainty_ms"], 2.0);
        assert_eq!(summary["headline_lcp_trace"]["nearby_trace_paint"], true);
        assert_eq!(
            summary["headline_lcp_trace"]["nearby_trace_lcp_candidate"],
            true
        );
        assert!(
            (summary["headline_lcp_trace"]["nearest_trace_paint_delta_ms"]
                .as_f64()
                .unwrap()
                - 0.0)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn clock_sync_rejects_invalid_intervals_and_ignores_wrong_markers() {
        assert!(clock_bracket(f64::NAN, 2.0).is_err());
        assert!(clock_bracket(3.0, 2.0).is_err());
        assert!(clock_bracket(1.0, 252.0).is_err());
        let bracket = clock_bracket(1000.0, 1004.0).unwrap();
        let wrap = |event: Value| json!({"message": json!({"message": event}).to_string()});
        let wrong = vec![wrap(json!({"method": "Tracing.dataCollected", "params": {
            "name": "clock_sync", "ts": 2000, "args": {"sync_id": "another-run"}
        }}))];
        let summary = summarize_logs(&wrong, &bracket, None);
        assert_eq!(summary["clock_sync"]["available"], false);
        assert_eq!(
            summary["clock_sync"]["reason"],
            "matching trace clock-sync marker missing"
        );
        assert!(cross_realm_sample(f64::NAN, 2.0, 3.0).is_err());
        assert!(cross_realm_sample(2.0, 1.0, 3.0).is_err());
        assert!(cross_realm_sample(1.0, 3.0, 2.0).is_err());
        assert!(cross_realm_sample(1.0, 2.0, 1002.0).is_err());
        let sample = cross_realm_sample(100.0, 104.0, 110.0).unwrap();
        assert_eq!(sample.endpoint_epoch_ms, 104.0);
        assert_eq!(sample.uncertainty_ms, 5.0);
        assert_eq!(observed_from_sample(90.0, &sample).unwrap(), Some(14.0));
        assert!(elapsed_from_navigation(105.0, sample.endpoint_epoch_ms).is_err());
        let imprecise = cross_realm_sample(100.0, 120.0, 160.0).unwrap();
        assert_eq!(observed_from_sample(90.0, &imprecise).unwrap(), None);
    }

    #[test]
    fn headline_lcp_requires_the_verified_heading_candidate() {
        let exact = json!({"supported": true, "headline_candidate": {
            "node_match": "exact", "start_time_ms": 120.0,
            "render_time_ms": 123.0, "presentation_time_ms": 123.0,
            "presentation_basis": "renderTime", "epoch_ms": 1000.0}});
        assert_eq!(headline_lcp_epoch(&exact, 1001.0), Some(1000.0));

        let mut descendant = exact.clone();
        descendant["headline_candidate"]["node_match"] = json!("descendant");
        descendant["headline_candidate"]["render_time_ms"] = json!(0.0);
        descendant["headline_candidate"]["presentation_time_ms"] = json!(120.0);
        descendant["headline_candidate"]["presentation_basis"] = json!("startTime");
        assert_eq!(headline_lcp_epoch(&descendant, 1001.0), Some(1000.0));

        for relation in ["ancestor", "other", "missing"] {
            let mut unrelated = exact.clone();
            unrelated["headline_candidate"]["node_match"] = json!(relation);
            assert_eq!(headline_lcp_epoch(&unrelated, 1001.0), None);
        }
        let mut future = exact;
        future["headline_candidate"]["epoch_ms"] = json!(1003.0);
        assert_eq!(headline_lcp_epoch(&future, 1001.0), None);
    }

    #[test]
    fn frame_clock_exchange_validates_timestamp_conversion() -> Result<()> {
        let valid = json!({"clock_exchange": {"available": true,
            "inner_before_epoch_ms": 1000.0, "top_epoch_ms": 1004.0,
            "inner_after_epoch_ms": 1010.0}});
        let conversion = frame_clock_conversion(&valid)?.context("conversion required")?;
        assert_eq!(conversion.endpoint_epoch_ms, 1004.0);
        assert_eq!(conversion.uncertainty_ms, 5.0);
        assert!(conversion.uncertainty_ms <= MAX_OBSERVED_CLOCK_UNCERTAINTY_MS);
        assert_eq!(
            observed_from_clock_conversion(900.0, 950.0, Some(&conversion))?,
            Some(50.0)
        );

        let imprecise = json!({"clock_exchange": {"available": true,
            "inner_before_epoch_ms": 1000.0, "top_epoch_ms": 1020.0,
            "inner_after_epoch_ms": 1060.0}});
        let imprecise = frame_clock_conversion(&imprecise)?.context("conversion required")?;
        assert_eq!(imprecise.uncertainty_ms, 30.0);
        assert!(imprecise.uncertainty_ms > MAX_OBSERVED_CLOCK_UNCERTAINTY_MS);
        assert_eq!(
            observed_from_clock_conversion(900.0, 950.0, Some(&imprecise))?,
            None
        );

        let unavailable = json!({"clock_exchange": {"available": false,
            "reason": "top-frame clock relay timed out"}});
        assert!(frame_clock_conversion(&unavailable)?.is_none());
        assert_eq!(observed_from_clock_conversion(900.0, 950.0, None)?, None);

        let backwards = json!({"clock_exchange": {"available": true,
            "inner_before_epoch_ms": 1005.0, "top_epoch_ms": 1004.0,
            "inner_after_epoch_ms": 1010.0}});
        assert!(frame_clock_conversion(&backwards).is_err());

        let too_wide = json!({"clock_exchange": {"available": true,
            "inner_before_epoch_ms": 1000.0, "top_epoch_ms": 1500.0,
            "inner_after_epoch_ms": 2001.0}});
        assert!(frame_clock_conversion(&too_wide).is_err());

        let missing = json!({"clock_exchange": {"available": true,
            "inner_before_epoch_ms": 1000.0, "inner_after_epoch_ms": 1001.0}});
        assert!(frame_clock_conversion(&missing).is_err());
        Ok(())
    }

    #[test]
    fn arbitrary_trace_paint_cannot_become_a_headline_endpoint() {
        let bracket = clock_bracket(1000.0, 1004.0).unwrap();
        let alignment = headline_trace_alignment(
            None,
            Some(50.0),
            &bracket,
            &[json!({"name": "Paint", "trace_timestamp_ms": 49.0})],
            &[],
            false,
        );
        assert_eq!(alignment["available"], false);
        assert_eq!(
            alignment["reason"],
            "no LCP candidate matched the nested .wp-h1 or its descendant"
        );
    }

    #[test]
    fn input_requires_trusted_click_visible_menu_and_event_timing() {
        let valid = json!({"trusted": true, "trusted_click_start_ms": 12.0,
        "expanded": true, "menu_contents_ready": true,
        "entries": [
            {"name": "pointerdown", "startTime": 10.0, "duration": 16.0, "interactionId": 7},
            {"name": "click", "startTime": 12.0, "duration": 24.0, "interactionId": 7},
            {"name": "click", "startTime": 100.0, "duration": 80.0, "interactionId": 9}
        ]});
        validate_input_endpoint(&valid).unwrap();
        assert_eq!(input_event_timing(&valid), Some(24.0));
        let mut invalid = valid.clone();
        invalid["trusted"] = json!(false);
        assert!(validate_input_endpoint(&invalid).is_err());
        invalid = valid.clone();
        invalid["menu_contents_ready"] = json!(false);
        assert!(validate_input_endpoint(&invalid).is_err());
        invalid = valid;
        invalid["entries"] =
            json!([{"name": "click", "startTime": 12.0, "duration": 24.0, "interactionId": 0}]);
        validate_input_endpoint(&invalid).unwrap();
        assert_eq!(input_event_timing(&invalid), None);
    }

    #[tokio::test]
    #[ignore = "isolated browser startup stress diagnostic; no Tonk navigation"]
    async fn it_checks_performance_browser_startup() -> Result<()> {
        let (servers, env) = TestServers::start().await?;
        let result: Result<()> = async {
            for index in 0..100 {
                let driver = env
                    .performance_driver()
                    .await
                    .with_context(|| format!("blank browser startup {index}"))?;
                let cdp = ChromeDevTools::new(driver.handle.clone());
                let probe: Result<()> = async {
                    cdp.execute_cdp_with_params(
                        "Emulation.setDeviceMetricsOverride",
                        json!({"width": 1280, "height": 800, "deviceScaleFactor": 1, "mobile": false}),
                    )
                    .await?;
                    cdp.execute_cdp_with_params("Emulation.setCPUThrottlingRate", json!({"rate": 1}))
                        .await?;
                    cdp.execute_cdp("Network.enable").await?;
                    cdp.execute_cdp_with_params(
                        "Network.emulateNetworkConditions",
                        json!({"offline": false, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}),
                    )
                    .await?;
                    let url = driver.current_url().await?;
                    ensure!(matches!(url.as_str(), "about:blank" | "data:,"),
                        "startup diagnostic left empty page");
                    Ok(())
                }
                .await;
                let quit = driver.quit().await;
                probe.with_context(|| format!("blank browser setup commands {index}"))?;
                quit?;
                eprintln!("blank-browser-startup {}/100 ok", index + 1);
            }
            Ok(())
        }
        .await;
        let cleanup = servers.stop().await;
        result?;
        cleanup?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires explicit release artifact, request, result paths and isolated Chrome"]
    async fn it_profiles_supplied_artifact() -> Result<()> {
        let request: Value = serde_json::from_slice(&std::fs::read(
            std::env::var_os("TONK_PERF_REQUEST").context("TONK_PERF_REQUEST required")?,
        )?)?;
        let output = std::env::var_os("TONK_PERF_RESULT").context("TONK_PERF_RESULT required")?;
        let output = Path::new(&output);
        let result: Result<Value> = async {
            let scenario = request["scenario"].as_str().context("scenario required")?;
            match scenario {
                "S1" => {
                    validate_request(&request)?;
                    if let Some(measurement) = request["measurement"].as_str()
                        && measurement.starts_with("navigation-to-usable-observed-")
                    {
                        ensure!(
                            measurement == S1_BASELINE_MEASUREMENT,
                            "unsupported observed-readiness protocol version"
                        );
                    }
                }
                "S2" => {
                    crate::performance_s2::validate_request(&request)?;
                }
                "S3" => {
                    crate::performance_s3::validate_request(&request)?;
                }
                "S6" => {
                    crate::performance_s6::validate_request(&request)?;
                }
                _ => anyhow::bail!("unsupported performance scenario"),
            }
            let (servers, env) = TestServers::start().await?;
            let measured = match scenario {
                "S1" if request["measurement"] == S1_BASELINE_MEASUREMENT => {
                    sample_s1_observed_baseline(&env, &request, output).await
                }
                "S1" => sample(&env, &request, output).await,
                "S2" => crate::performance_s2::sample(&env, &request, output).await,
                "S3" => crate::performance_s3::sample(&env, &request, output).await,
                "S6" => crate::performance_s6::sample(&env, &request, output).await,
                _ => unreachable!("scenario validated before server startup"),
            };
            let cleanup = servers.stop().await;
            cleanup?;
            measured
        }
        .await;
        let mut record = match &result {
            Ok(record) => record.clone(),
            Err(_) => json!({"schema_version": 1, "status": "harness_error",
                "identity_verified": false, "metrics": {}, "failure_category": "runner_error"}),
        };
        for key in [
            "scenario",
            "profile",
            "profile_settings",
            "fixture_seed",
            "slot",
            "fixture_sha256",
        ] {
            record[key] = request[key].clone();
        }
        std::fs::write(output, serde_json::to_vec_pretty(&record)?)?;
        result?;
        Ok(())
    }
}
