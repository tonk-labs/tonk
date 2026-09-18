//! Correctness prerequisite for returning-user performance fixtures.

use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use thirtyfour::extensions::cdp::ChromeDevTools;
use thirtyfour::prelude::*;

use crate::helpers::{TestEnvironment, TestServers};

async fn snapshot(driver: &WebDriver) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let state = driver
            .execute_async(
                r#"
            const done = arguments[arguments.length - 1];
            Promise.all(['/api/health', '/api/profile'].map(path =>
              fetch(path, {signal: AbortSignal.timeout(2000)}).then(response => {
                if (!response.ok) throw new Error('not ready');
                return response.json();
              })
            )).then(([health, profile]) => done({
              startedAt: health.startedAt,
              spaces: (profile.space || []).map(space => space.subject),
              marker: localStorage.getItem('tonk:perf:restart'),
              controlled: !!navigator.serviceWorker.controller
            })).catch(() => done(null));
        "#,
                vec![],
            )
            .await?
            .json()
            .clone();
        if state["startedAt"].as_u64().is_some()
            && state["spaces"].as_array().is_some_and(|spaces| {
                !spaces.is_empty()
                    && spaces
                        .iter()
                        .all(|space| space.as_str().is_some_and(|subject| !subject.is_empty()))
            })
            && state["controlled"] == true
        {
            return Ok(state);
        }
        ensure!(
            Instant::now() < deadline,
            "returning-user fixture did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn visit(driver: &WebDriver, env: &TestEnvironment) -> Result<()> {
    ChromeDevTools::new(driver.handle.clone())
        .execute_cdp_with_params(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({
                "source": "try { localStorage.setItem('tonk:telemetry', 'off'); } catch {}"
            }),
        )
        .await?;
    driver.goto(env.tonk_web.as_str()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires isolated Chrome and the local browser test servers"]
async fn retained_profile_preserves_local_state_with_a_cold_worker() -> Result<()> {
    let (servers, env) = TestServers::start().await?;
    let tested: Result<()> = async {
        let profile = tempfile::Builder::new()
            .prefix("perf-returning-")
            .tempdir_in(&env.browser_profile_root)?
            .keep();
        let first = env.blank_driver_with_profile(&profile).await?;
        let prepared: Result<Value> = async {
            visit(&first, &env).await?;
            let state = snapshot(&first).await?;
            first
                .execute(
                    "localStorage.setItem('tonk:perf:restart', 'retained-fixture-v1')",
                    vec![],
                )
                .await?;
            Ok(state)
        }
        .await;
        let quit = first.quit().await;
        let before = prepared?;
        quit?;

        let second = env.blank_driver_with_profile(&profile).await?;
        let checked: Result<()> = async {
            visit(&second, &env).await?;
            let after = snapshot(&second).await?;
            ensure!(
                after["marker"] == "retained-fixture-v1",
                "local fixture marker was lost"
            );
            ensure!(
                after["spaces"] == before["spaces"],
                "seeded local spaces changed across browser restart"
            );
            ensure!(
                after["startedAt"] != before["startedAt"],
                "worker was not restarted"
            );
            Ok(())
        }
        .await;
        let quit = second.quit().await;
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

#[test]
fn retained_profile_cannot_escape_disposable_workspace() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let profile = tempfile::tempdir_in(workspace.path())?;
    let origin = url::Url::parse("https://localhost:12345")?;
    let env = TestEnvironment {
        tonk_web: origin.clone(),
        chromedriver: origin.clone(),
        access_service: origin,
        ca_certificate: None,
        deployment_root: workspace.path().into(),
        service_worker_script: workspace.path().join("service_worker.js"),
        browser_profile_root: workspace.path().into(),
    };
    assert!(env.checked_test_profile(profile.path()).is_ok());
    assert!(env.checked_test_profile(outside.path()).is_err());
    assert!(env.checked_test_profile(workspace.path()).is_err());
    let nested = tempfile::tempdir_in(profile.path())?;
    assert!(env.checked_test_profile(nested.path()).is_err());
    #[cfg(unix)]
    {
        let link = workspace.path().join("outside-link");
        std::os::unix::fs::symlink(outside.path(), &link)?;
        assert!(env.checked_test_profile(&link).is_err());
    }
    Ok(())
}
