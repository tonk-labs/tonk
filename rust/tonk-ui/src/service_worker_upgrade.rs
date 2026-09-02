//! Real-browser service-worker load-time upgrade tests for Storybook `UI-03`.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use std::io::Write as _;
    use std::path::Path;
    use std::process::Command;
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

    fn worker_build_id(script_path: &Path) -> Result<String> {
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
        let build = build_ids[0];
        ensure!(
            build.len() == 16
                && build
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "worker build id is malformed: {build:?}"
        );
        Ok(build.to_owned())
    }

    fn copy_artifact_tree(source: &Path, destination: &Path) -> Result<()> {
        std::fs::create_dir(destination)
            .with_context(|| format!("create generation {}", destination.display()))?;
        for entry in std::fs::read_dir(source)
            .with_context(|| format!("enumerate generation {}", source.display()))?
        {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                copy_artifact_tree(&source_path, &destination_path)?;
            } else if file_type.is_file() {
                std::fs::copy(&source_path, &destination_path).with_context(|| {
                    format!(
                        "copy artifact {} to {}",
                        source_path.display(),
                        destination_path.display()
                    )
                })?;
            } else {
                return Err(anyhow!(
                    "unsupported artifact member {}",
                    source_path.display()
                ));
            }
        }
        Ok(())
    }

    fn prepare_second_generation(env: &TestEnvironment) -> Result<(String, String)> {
        let generation_a = env.deployment_root.join("generation-a");
        let generation_b = env.deployment_root.join("generation-b");
        ensure!(
            !generation_b.exists(),
            "second generation already exists at {}",
            generation_b.display()
        );
        let build_a = worker_build_id(&generation_a.join("service_worker.js"))?;
        copy_artifact_tree(&generation_a, &generation_b)?;

        // Change a member of the published graph, then run the real publisher.
        // Merely rewriting the worker marker would create an invalid generation
        // and would not exercise the atomic complete-artifact contract.
        let index_path = generation_b.join("index.html");
        writeln!(
            std::fs::OpenOptions::new().append(true).open(&index_path)?,
            "<!-- integration generation B -->"
        )?;
        let stamp = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("stamp-service-worker.sh");
        let output = Command::new(&stamp)
            .arg(&generation_b)
            .output()
            .with_context(|| format!("run {}", stamp.display()))?;
        ensure!(
            output.status.success(),
            "second-generation stamp failed: status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let build_b = worker_build_id(&generation_b.join("service_worker.js"))?;
        ensure!(build_a != build_b, "A and B must have distinct build ids");
        Ok((build_a, build_b))
    }

    #[cfg(unix)]
    fn promote_second_generation(env: &TestEnvironment) -> Result<()> {
        use std::os::unix::fs::symlink;

        let next = env.deployment_root.join("current-next");
        let current = env.deployment_root.join("current");
        symlink("generation-b", &next)
            .with_context(|| format!("create deployment link {}", next.display()))?;
        std::fs::rename(&next, &current).with_context(|| {
            format!(
                "atomically promote {} over {}",
                next.display(),
                current.display()
            )
        })?;
        Ok(())
    }

    async fn request_registration_update(driver: &WebDriver) -> Result<()> {
        let result = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                navigator.serviceWorker.getRegistration()
                    .then(async registration => {
                        if (!registration) throw new Error("missing registration");
                        await registration.update();
                        done({ ok: true });
                    })
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        ensure!(
            result.json()["ok"] == true,
            "service-worker update failed: {}",
            result.json()
        );
        Ok(())
    }

    async fn set_update_hold(driver: &WebDriver, held: bool) -> Result<()> {
        let result = driver
            .execute_async(
                r#"
                const held = arguments[0];
                const done = arguments[arguments.length - 1];
                (async () => {
                    if (!navigator.locks?.request) throw new Error("Web Locks unavailable");
                    await navigator.locks.request("tonk-update-safety-v1", { mode: "exclusive" }, async () => {
                        const database = await new Promise((resolve, reject) => {
                            const request = indexedDB.open("tonk-update-safety-v1", 1);
                            request.onupgradeneeded = () => {
                                if (!request.result.objectStoreNames.contains("holds")) {
                                    request.result.createObjectStore("holds");
                                }
                            };
                            request.onsuccess = () => resolve(request.result);
                            request.onerror = () => reject(request.error);
                        });
                        await new Promise((resolve, reject) => {
                            const transaction = database.transaction("holds", "readwrite");
                            const store = transaction.objectStore("holds");
                            if (held) {
                                store.put({
                                    version: 1,
                                    kind: "account-setup",
                                    operationId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                                    leasedRevision: "1",
                                }, "account-setup");
                            } else {
                                store.delete("account-setup");
                            }
                            transaction.oncomplete = resolve;
                            transaction.onerror = () => reject(transaction.error);
                            transaction.onabort = () => reject(transaction.error);
                        });
                        database.close();
                    });
                    if (!held) {
                        const channel = new BroadcastChannel("tonk-update-safety-v1");
                        channel.postMessage({ type: "account-setup-hold-changed", version: 1 });
                        channel.close();
                    }
                    done({ ok: true });
                })().catch(error => done({ error: String(error) }));
                "#,
                vec![held.into()],
            )
            .await?;
        ensure!(
            result.json()["ok"] == true,
            "failed to change update hold: {}",
            result.json()
        );
        Ok(())
    }

    async fn tab_build_state(driver: &WebDriver) -> Result<Value> {
        let result = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                Promise.all([
                    navigator.serviceWorker.getRegistration(),
                    fetch("/api/health").then(response => response.json()),
                ]).then(([registration, health]) => done({
                    health,
                    documentBuild: document.querySelector('meta[name="tonk-worker-build"]')?.content || null,
                    active: registration?.active?.state || null,
                    installing: registration?.installing?.state || null,
                    waiting: registration?.waiting?.state || null,
                    mounted: !!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate"),
                })).catch(error => done({ error: String(error) }));
                "##,
                vec![],
            )
            .await?;
        Ok(result.json().clone())
    }

    async fn wait_for_mounted_build(driver: &WebDriver, build: &str) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut last = Value::Null;
        loop {
            if let Ok(state) = driver
                .execute_async(
                    r##"
                    const done = arguments[arguments.length - 1];
                    Promise.all([
                        navigator.serviceWorker.getRegistration(),
                        fetch("/api/health").then(response => response.json()),
                        caches.keys(),
                        fetch("/asset-manifest.json", { cache: "no-store" }).then(response => response.json()),
                    ]).then(([registration, health, cacheNames, manifest]) => done({
                        health,
                        manifest,
                        cacheNames,
                        documentBuild: document.querySelector('meta[name="tonk-worker-build"]')?.content || null,
                        controlled: !!navigator.serviceWorker.controller,
                        active: registration?.active?.state || null,
                        installing: registration?.installing?.state || null,
                        waiting: registration?.waiting?.state || null,
                        mounted: !!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate"),
                        guard: sessionStorage.getItem("tonk:sw-upgrade-reload"),
                    })).catch(error => done({ error: String(error) }));
                    "##,
                    vec![],
                )
                .await
            {
                last = state.json().clone();
                if last["health"]["build"] == build
                    && last["documentBuild"] == build
                    && last["manifest"]["build"] == build
                    && last["mounted"] == true
                {
                    return Ok(last);
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for coherent build {build}: {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
    async fn it_adopts_a_complete_second_generation_without_mixing_assets(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let health_a = worker_health(&driver).await?;
        let (build_a, build_b) = prepare_second_generation(&env)?;
        assert_eq!(health_a["body"]["build"], build_a, "{health_a}");
        create_state_sentinels(&driver).await?;
        install_document_probe(&driver).await?;
        promote_second_generation(&env)?;
        request_registration_update(&driver).await?;

        let state = wait_for_mounted_build(&driver, &build_b).await?;
        assert_eq!(state["controlled"], true, "{state}");
        assert_eq!(state["active"], "activated", "{state}");
        assert!(state["installing"].is_null(), "{state}");
        assert!(state["waiting"].is_null(), "{state}");
        assert_eq!(state["mounted"], true, "{state}");
        assert!(state["guard"].is_null(), "{state}");
        assert!(
            state["cacheNames"].as_array().is_some_and(|names| names
                .iter()
                .any(|name| name == &format!("TONK_SHELL_{build_a}"))),
            "the incumbent generation cache must be retained: {state}"
        );
        assert!(
            state["cacheNames"].as_array().is_some_and(|names| names
                .iter()
                .any(|name| name == &format!("TONK_SHELL_{build_b}"))),
            "the successor generation cache must be complete: {state}"
        );

        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_sibling_tabs_on_their_controller_until_claim_is_safe(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let primary = driver.window().await?;
        let (build_a, build_b) = prepare_second_generation(&env)?;
        let initial = wait_for_mounted_build(&driver, &build_a).await?;
        assert_eq!(initial["health"]["build"], build_a, "{initial}");
        create_state_sentinels(&driver).await?;
        set_update_hold(&driver, true).await?;

        let sibling = driver.new_tab().await?;
        driver.switch_to_window(sibling.clone()).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let sibling_a = wait_for_mounted_build(&driver, &build_a).await?;
        assert_eq!(sibling_a["documentBuild"], build_a, "{sibling_a}");

        driver.switch_to_window(primary.clone()).await?;
        promote_second_generation(&env)?;
        request_registration_update(&driver).await?;
        tokio::time::sleep(Duration::from_secs(2)).await;

        let primary_held = tab_build_state(&driver).await?;
        assert_eq!(primary_held["health"]["build"], build_a, "{primary_held}");
        assert_eq!(primary_held["documentBuild"], build_a, "{primary_held}");
        driver.switch_to_window(sibling.clone()).await?;
        let sibling_held = tab_build_state(&driver).await?;
        assert_eq!(sibling_held["health"]["build"], build_a, "{sibling_held}");
        assert_eq!(sibling_held["documentBuild"], build_a, "{sibling_held}");

        driver.switch_to_window(primary.clone()).await?;
        set_update_hold(&driver, false).await?;
        let primary_b = wait_for_mounted_build(&driver, &build_b).await?;
        assert_eq!(primary_b["health"]["build"], build_b, "{primary_b}");
        driver.switch_to_window(sibling).await?;
        let sibling_b = wait_for_mounted_build(&driver, &build_b).await?;
        assert_eq!(sibling_b["health"]["build"], build_b, "{sibling_b}");

        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_withdraws_a_generation_without_deleting_local_state(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let build = worker_build_id(&env.service_worker_script)?;
        let initial = wait_for_mounted_build(&driver, &build).await?;
        assert_eq!(initial["health"]["build"], build, "{initial}");
        create_state_sentinels(&driver).await?;
        let caches_before = initial["cacheNames"].clone();

        std::fs::write(
            env.deployment_root
                .join("generation-a")
                .join("kill-switch.json"),
            format!("{{\"revoked\":[\"{build}\"]}}\n"),
        )?;
        driver.refresh().await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let withdrawn = loop {
            let health = worker_health(&driver).await?;
            if health["body"]["worker"] == "failed"
                && health["body"]["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("withdrawn"))
            {
                break health;
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for withdrawal: {health}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert_eq!(withdrawn["body"]["build"], build, "{withdrawn}");

        let after = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                Promise.all([
                    navigator.serviceWorker.getRegistration(),
                    caches.keys(),
                    fetch("/api/profile", { method: "POST" }).then(async response => ({
                        status: response.status,
                        body: await response.json(),
                    })),
                ]).then(([registration, cacheNames, refused]) => done({
                    registration: !!registration,
                    cacheNames,
                    refused,
                })).catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;
        assert_eq!(after.json()["registration"], true, "{after:?}");
        assert_eq!(after.json()["cacheNames"], caches_before, "{after:?}");
        assert_eq!(after.json()["refused"]["status"], 503, "{after:?}");
        assert_eq!(
            after.json()["refused"]["body"]["error"]["kind"],
            "worker-failed",
            "{after:?}"
        );
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
