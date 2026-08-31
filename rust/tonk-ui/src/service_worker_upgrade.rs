//! Real-browser service-worker load-time upgrade tests for Storybook `UI-03`.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow, ensure};
    use serde_json::Value;
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::prelude::*;

    use crate::helpers::TestEnvironment;

    async fn worker_health(driver: &WebDriver) -> Result<Value> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                fetch("/api/health")
                    .then(async response => {
                        const body = await response.text();
                        try {
                            done({ status: response.status, body: JSON.parse(body) });
                        } catch (error) {
                            done({ status: response.status, error: String(error), body });
                        }
                    })
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        Ok(result.json().clone())
    }

    async fn wait_for_worker_started_at(driver: &WebDriver, previous: Option<u64>) -> Result<u64> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let last = worker_health(driver)
                .await
                .unwrap_or_else(|error| serde_json::json!({ "webdriverError": error.to_string() }));
            if let Some(started_at) = last["body"]["startedAt"].as_u64()
                && previous.is_none_or(|previous| previous != started_at)
            {
                return Ok(started_at);
            }
            if tokio::time::Instant::now() >= deadline {
                let document_state = driver
                    .execute(
                        r##"
                        return {
                            count: Number(sessionStorage.getItem("tonk:test:sw-documents")) || 0,
                            roots: JSON.parse(sessionStorage.getItem("tonk:test:sw-roots") || "{}"),
                            mounted: !!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate"),
                            guard: sessionStorage.getItem("tonk:sw-upgrade-reload"),
                        };
                        "##,
                        vec![],
                    )
                    .await
                    .map(|value| value.json().clone())
                    .unwrap_or(Value::Null);
                return Err(anyhow!(
                    "timed out waiting for a different worker; health={last}, document={document_state}"
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn create_state_sentinels(driver: &WebDriver) -> Result<()> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                (async () => {
                    const database = await new Promise((resolve, reject) => {
                        const request = indexedDB.open("tonk-sw-upgrade-sentinel", 1);
                        request.onupgradeneeded = () => request.result.createObjectStore("state");
                        request.onsuccess = () => resolve(request.result);
                        request.onerror = () => reject(request.error);
                    });
                    await new Promise((resolve, reject) => {
                        const transaction = database.transaction("state", "readwrite");
                        transaction.objectStore("state").put("preserved", "value");
                        transaction.oncomplete = resolve;
                        transaction.onerror = () => reject(transaction.error);
                    });
                    database.close();

                    const cache = await caches.open("tonk-sw-upgrade-sentinel");
                    await cache.put("/__tonk/sw-upgrade-sentinel", new Response("preserved"));
                    done({ ok: true });
                })().catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        ensure!(
            result.json()["ok"] == true,
            "failed to create state sentinels: {}",
            result.json()
        );
        Ok(())
    }

    async fn state_sentinels(driver: &WebDriver) -> Result<Value> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                (async () => {
                    const database = await new Promise((resolve, reject) => {
                        const request = indexedDB.open("tonk-sw-upgrade-sentinel", 1);
                        request.onsuccess = () => resolve(request.result);
                        request.onerror = () => reject(request.error);
                    });
                    const indexedDb = await new Promise((resolve, reject) => {
                        const request = database.transaction("state").objectStore("state").get("value");
                        request.onsuccess = () => resolve(request.result);
                        request.onerror = () => reject(request.error);
                    });
                    database.close();
                    const cache = await caches.open("tonk-sw-upgrade-sentinel");
                    const response = await cache.match("/__tonk/sw-upgrade-sentinel");
                    done({ indexedDb, cache: response ? await response.text() : null });
                })().catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        Ok(result.json().clone())
    }

    async fn wait_for_mounted_worker(driver: &WebDriver, started_at: u64) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut last = Value::Null;
        loop {
            let state = driver
                .execute_async(
                    r##"
                    const done = arguments[arguments.length - 1];
                    Promise.all([
                        navigator.serviceWorker.getRegistration(),
                        fetch("/api/health").then(response => response.json()),
                    ]).then(([registration, health]) => done({
                        health,
                        controlled: !!navigator.serviceWorker.controller,
                        active: registration?.active?.state || null,
                        installing: registration?.installing?.state || null,
                        waiting: registration?.waiting?.state || null,
                        mounted: !!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate"),
                        guard: sessionStorage.getItem("tonk:sw-upgrade-reload"),
                        documents: Number(sessionStorage.getItem("tonk:test:sw-documents")) || 0,
                        roots: JSON.parse(sessionStorage.getItem("tonk:test:sw-roots") || "{}"),
                    })).catch(error => done({ error: String(error) }));
                    "##,
                    vec![],
                )
                .await;
            if let Ok(state) = state {
                last = state.json().clone();
                if last["health"]["startedAt"].as_u64() == Some(started_at)
                    && last["mounted"] == true
                {
                    return Ok(last);
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the cached page to mount under worker {started_at}: {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn install_delayed_worker(script_path: &std::path::Path) -> Result<()> {
        let script = std::fs::read_to_string(script_path)
            .with_context(|| format!("read {}", script_path.display()))?;
        let build_ids = script
            .lines()
            .filter_map(|line| {
                line.strip_prefix("const BUILD_ID = \"")
                    .and_then(|value| value.strip_suffix("\";"))
            })
            .collect::<Vec<_>>();
        ensure!(build_ids.len() == 1, "expected exactly one worker build id");
        let old_hash = build_ids[0];
        ensure!(
            old_hash.len() == 16
                && old_hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "worker build id is malformed: {old_hash:?}"
        );
        let new_hash = if old_hash == "0000000000000000" {
            "1111111111111111"
        } else {
            "0000000000000000"
        };
        let old_declaration = format!("const BUILD_ID = \"{old_hash}\";");
        ensure!(script.matches(&old_declaration).count() == 1);
        let script = script.replacen(
            &old_declaration,
            &format!("const BUILD_ID = \"{new_hash}\";"),
            1,
        );

        let install_start = "event.waitUntil((async () => {";
        ensure!(script.matches(install_start).count() == 1);
        let script = script.replacen(
            install_start,
            &format!(
                "{install_start}\n        await new Promise(resolve => setTimeout(resolve, 1500));"
            ),
            1,
        );
        std::fs::write(script_path, script)
            .with_context(|| format!("write {}", script_path.display()))?;
        Ok(())
    }

    async fn install_document_probe(driver: &WebDriver) -> Result<()> {
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools
            .execute_cdp_with_params(
                "Page.addScriptToEvaluateOnNewDocument",
                serde_json::json!({
                    "source": r##"
                        (() => {
                            const countKey = "tonk:test:sw-documents";
                            const rootsKey = "tonk:test:sw-roots";
                            const documentNumber = (Number(sessionStorage.getItem(countKey)) || 0) + 1;
                            sessionStorage.setItem(countKey, String(documentNumber));
                            const roots = JSON.parse(sessionStorage.getItem(rootsKey) || "{}");
                            roots[documentNumber] = false;
                            sessionStorage.setItem(rootsKey, JSON.stringify(roots));
                            const recordRoot = () => {
                                if (!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate")) return;
                                const roots = JSON.parse(sessionStorage.getItem(rootsKey) || "{}");
                                roots[documentNumber] = true;
                                sessionStorage.setItem(rootsKey, JSON.stringify(roots));
                            };
                            new MutationObserver(recordRoot).observe(document, { childList: true, subtree: true });
                            recordRoot();
                        })();
                    "##
                }),
            )
            .await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_replaces_a_recent_worker_on_load_without_manual_cleanup(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let worker_a = wait_for_worker_started_at(&driver, None).await?;
        create_state_sentinels(&driver).await?;
        install_document_probe(&driver).await?;
        install_delayed_worker(&env.service_worker_script)?;

        driver.refresh().await?;
        let worker_b = wait_for_worker_started_at(&driver, Some(worker_a)).await?;
        ensure!(worker_b != worker_a);

        let state = wait_for_mounted_worker(&driver, worker_b).await?;
        assert_eq!(state["controlled"], true, "{state}");
        assert_eq!(state["active"], "activated", "{state}");
        assert!(state["installing"].is_null(), "{state}");
        assert!(state["waiting"].is_null(), "{state}");
        assert_eq!(state["documents"], 2, "{state}");
        assert_eq!(state["roots"]["1"], false, "{state}");
        assert_eq!(state["roots"]["2"], true, "{state}");
        assert_eq!(state["mounted"], true, "{state}");
        assert!(state["guard"].is_null(), "{state}");

        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_the_active_worker_when_the_load_time_update_check_is_offline(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let worker_a = wait_for_worker_started_at(&driver, None).await?;
        create_state_sentinels(&driver).await?;

        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools.execute_cdp("Network.enable").await?;
        devtools
            .execute_cdp_with_params(
                "Network.emulateNetworkConditions",
                serde_json::json!({
                    "offline": true,
                    "latency": 0,
                    "downloadThroughput": 0,
                    "uploadThroughput": 0,
                }),
            )
            .await?;

        let test_result: Result<()> = async {
            driver.refresh().await?;
            let state = wait_for_mounted_worker(&driver, worker_a).await?;
            ensure!(state["controlled"] == true, "{state}");
            ensure!(state["active"] == "activated", "{state}");
            ensure!(state["installing"].is_null(), "{state}");
            ensure!(state["waiting"].is_null(), "{state}");
            ensure!(state["guard"].is_null(), "{state}");

            let sentinels = state_sentinels(&driver).await?;
            ensure!(sentinels["indexedDb"] == "preserved", "{sentinels}");
            ensure!(sentinels["cache"] == "preserved", "{sentinels}");
            Ok(())
        }
        .await;

        let restore_result = devtools
            .execute_cdp_with_params(
                "Network.emulateNetworkConditions",
                serde_json::json!({
                    "offline": false,
                    "latency": 0,
                    "downloadThroughput": -1,
                    "uploadThroughput": -1,
                }),
            )
            .await;
        let quit_result = driver.quit().await;

        test_result?;
        restore_result?;
        quit_result?;
        Ok(())
    }
}
