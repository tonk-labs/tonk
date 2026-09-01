//! Real-browser service-worker load-time upgrade tests for Storybook `UI-03`.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use std::collections::BTreeMap;
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

    #[derive(Debug)]
    struct GenerationContract {
        build: String,
        /// Digest prefix stamped into the worker glue and re-observed from the
        /// exact ArrayBuffer handed to wasm-bindgen initialization.
        worker_wasm: String,
        /// Stable-URL members whose bytes must stay coherent with the
        /// document/worker build. Values are their manifest SHA-256 digests.
        probes: BTreeMap<String, String>,
        /// Worker-owned members are deliberately absent from the shell
        /// manifest: the browser pins the imported glue while the worker
        /// verifies and caches its Wasm. Keep their exact fixture bytes so the
        /// two-generation test still proves that A and B differ at this layer.
        worker_members: BTreeMap<String, Vec<u8>>,
    }

    fn generation_contract(root: &Path) -> Result<GenerationContract> {
        let build = worker_build_id(&root.join("service_worker.js"))?;
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(root.join("asset-manifest.json"))?)?;
        let version: Value = serde_json::from_slice(&std::fs::read(root.join("version.json"))?)?;
        ensure!(manifest["build"] == build, "manifest/worker build mismatch");
        ensure!(version["build"] == build, "version/worker build mismatch");
        let worker_wasm = version["workerWasm"]
            .as_str()
            .ok_or_else(|| anyhow!("version has no worker Wasm digest"))?
            .to_owned();
        let assets = manifest["assets"]
            .as_object()
            .ok_or_else(|| anyhow!("asset manifest has no asset map"))?;

        let select_one =
            |label: &str, predicate: &dyn Fn(&str) -> bool| -> Result<(String, String)> {
                let found = assets
                    .iter()
                    .filter(|(path, _)| predicate(path))
                    .collect::<Vec<_>>();
                ensure!(
                    found.len() == 1,
                    "expected one {label} probe, found {found:?}"
                );
                let (path, digest) = found[0];
                let digest = digest
                    .as_str()
                    .ok_or_else(|| anyhow!("{label} digest is not a string"))?;
                Ok((path.clone(), digest.to_owned()))
            };

        let mut probes = BTreeMap::new();
        for (path, digest) in [
            select_one("document", &|path| path == "/")?,
            select_one("UI Wasm", &|path| {
                path.starts_with("/ui-") && path.ends_with("_bg.wasm")
            })?,
            select_one("guest glue", &|path| {
                path.starts_with("/guest/guest-") && path.ends_with(".js")
            })?,
            select_one("guest Wasm", &|path| {
                path.starts_with("/guest/guest_bg-") && path.ends_with(".wasm")
            })?,
        ] {
            probes.insert(path, digest);
        }
        let worker_members = ["service_worker.js", "worker.js", "worker_bg.wasm"]
            .into_iter()
            .map(|path| {
                Ok((
                    path.to_owned(),
                    std::fs::read(root.join(path))
                        .with_context(|| format!("read worker member {path}"))?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(GenerationContract {
            build,
            worker_wasm,
            probes,
            worker_members,
        })
    }

    fn encode_u32_leb(mut value: u32) -> Vec<u8> {
        let mut encoded = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            encoded.push(byte);
            if value == 0 {
                return encoded;
            }
        }
    }

    /// Append a valid custom section. Engines ignore custom sections, so B's
    /// Wasm remains executable while being byte-distinct from A.
    fn append_wasm_generation_marker(path: &Path) -> Result<()> {
        let name = b"tonk-integration-generation-b";
        let mut payload = encode_u32_leb(name.len() as u32);
        payload.extend_from_slice(name);
        let mut section = vec![0];
        section.extend_from_slice(&encode_u32_leb(payload.len() as u32));
        section.extend_from_slice(&payload);
        std::fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(&section)?;
        Ok(())
    }

    fn distinguish_generation_b(root: &Path) -> Result<()> {
        fn visit(path: &Path) -> Result<()> {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    visit(&path)?;
                    continue;
                }
                if path.extension().and_then(|extension| extension.to_str()) == Some("wasm") {
                    append_wasm_generation_marker(&path)?;
                }
            }
            Ok(())
        }
        visit(root)?;

        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(root.join("index.html"))?,
            "<!-- integration generation B: index.html -->"
        )?;
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(root.join("worker.js"))?,
            "// integration generation B worker glue"
        )?;
        let guest_glue = std::fs::read_dir(root.join("guest"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("guest-") && name.ends_with(".js"))
            })
            .ok_or_else(|| anyhow!("generation has no guest glue"))?;
        writeln!(
            std::fs::OpenOptions::new().append(true).open(guest_glue)?,
            "// integration generation B guest glue"
        )?;
        Ok(())
    }

    /// Put the navigation counter in the served document itself so every
    /// WebDriver observes the same lifecycle evidence. Session storage spans
    /// same-tab reloads but not test environments; the root map distinguishes
    /// the deliberately unmounted alignment document from mounted A/B pages.
    fn instrument_generation_documents(root: &Path) -> Result<()> {
        let index_path = root.join("index.html");
        let index = std::fs::read_to_string(&index_path)?;
        let marker = "data-tonk-test-sw-documents";
        ensure!(
            !index.contains(marker),
            "document probe is already installed"
        );
        let probe = r##"<script data-tonk-test-sw-documents>
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
        </script>
        "##;
        let instrumented = index.replacen("</head>", &format!("{probe}</head>"), 1);
        ensure!(
            instrumented != index,
            "generation document has no closing head"
        );
        std::fs::write(&index_path, instrumented)?;
        Ok(())
    }

    fn stamp_generation(root: &Path) -> Result<()> {
        let stamp = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("stamp-service-worker.sh");
        let output = Command::new(&stamp)
            .arg(root)
            .output()
            .with_context(|| format!("run {}", stamp.display()))?;
        ensure!(
            output.status.success(),
            "generation stamp failed: status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
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

    fn prepare_second_generation(
        env: &TestEnvironment,
    ) -> Result<(GenerationContract, GenerationContract)> {
        let generation_a = env.deployment_root.join("generation-a");
        let generation_b = env.deployment_root.join("generation-b");
        ensure!(
            !generation_b.exists(),
            "second generation already exists at {}",
            generation_b.display()
        );
        instrument_generation_documents(&generation_a)?;
        stamp_generation(&generation_a)?;
        let generation_a_contract = generation_contract(&generation_a)?;
        copy_artifact_tree(&generation_a, &generation_b)?;

        // Make every load-bearing layer byte-distinct while keeping each Wasm
        // module valid, then run the real publisher over the complete graph.
        distinguish_generation_b(&generation_b)?;
        stamp_generation(&generation_b)?;
        let generation_b_contract = generation_contract(&generation_b)?;
        ensure!(
            generation_a_contract.build != generation_b_contract.build,
            "A and B must have distinct build ids"
        );
        ensure!(
            generation_a_contract
                .probes
                .keys()
                .eq(generation_b_contract.probes.keys()),
            "A and B must expose the same stable probe URLs"
        );
        for (path, digest_a) in &generation_a_contract.probes {
            ensure!(
                generation_b_contract.probes.get(path) != Some(digest_a),
                "generation probe {path} is byte-identical across A and B"
            );
        }
        ensure!(
            generation_a_contract
                .worker_members
                .keys()
                .eq(generation_b_contract.worker_members.keys()),
            "A and B must expose the same worker-owned members"
        );
        for (path, bytes_a) in &generation_a_contract.worker_members {
            ensure!(
                generation_b_contract.worker_members.get(path) != Some(bytes_a),
                "worker-owned generation member {path} is byte-identical across A and B"
            );
        }
        Ok((generation_a_contract, generation_b_contract))
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
                    const expectedBuild = arguments[0];
                    const done = arguments[arguments.length - 1];
                    (async () => {
                        const registration = await navigator.serviceWorker.getRegistration();
                        const documentBuild = document.querySelector('meta[name="tonk-worker-build"]')?.content || null;
                        const lifecycle = {
                            documentBuild,
                            controlled: !!navigator.serviceWorker.controller,
                            active: registration?.active?.state || null,
                            installing: registration?.installing?.state || null,
                            waiting: registration?.waiting?.state || null,
                            mounted: !!document.querySelector("#tonk-root, tonk-site, tonk-account, tonk-activate"),
                            guard: sessionStorage.getItem("tonk:sw-upgrade-reload"),
                            testErrors: globalThis.__tonkTestErrors || [],
                            testInstallProgress: (globalThis.__tonkTestInstallProgress || []).slice(-5),
                            documents: Number(sessionStorage.getItem("tonk:test:sw-documents")) || 0,
                            roots: JSON.parse(sessionStorage.getItem("tonk:test:sw-roots") || "{}"),
                        };
                        // Polling the retiring worker's fetch boundary can
                        // itself keep that worker alive and prevent Chrome
                        // from advancing an installed successor. Wait on only
                        // registration/document state until B is actually the
                        // document, then verify its data plane and manifest.
                        if (documentBuild !== expectedBuild) {
                            done(lifecycle);
                            return;
                        }
                        const [healthResponse, cacheNames, manifestResponse] = await Promise.all([
                            fetch("/api/health"),
                            caches.keys(),
                            fetch("/asset-manifest.json", { cache: "no-store" }),
                        ]);
                        const healthBody = await healthResponse.text();
                        const manifestBody = await manifestResponse.text();
                        const parse = body => {
                            try { return JSON.parse(body); } catch { return null; }
                        };
                        const parsedManifest = parse(manifestBody);
                        done({
                            ...lifecycle,
                            health: parse(healthBody),
                            healthStatus: healthResponse.status,
                            healthBody: healthBody.slice(0, 200),
                            cacheNames,
                            manifest: parsedManifest && { build: parsedManifest.build },
                            manifestStatus: manifestResponse.status,
                            manifestBody: manifestBody.slice(0, 200),
                        });
                    })().catch(error => done({ error: String(error) }));
                    "##,
                    vec![build.into()],
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

    async fn wait_for_successor_while_controller_is_held(
        driver: &WebDriver,
        incumbent: &str,
        successor: &str,
    ) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut last = Value::Null;
        loop {
            if let Ok(state) = driver
                .execute_async(
                    r#"
                    const done = arguments[arguments.length - 1];
                    Promise.all([
                        navigator.serviceWorker.getRegistration(),
                        fetch("/api/health").then(response => response.json()),
                        fetch("/version.json", { cache: "no-store" }).then(response => response.json()),
                    ]).then(([registration, health, discovery]) => done({
                        health,
                        discovery,
                        controlled: !!navigator.serviceWorker.controller,
                        active: registration?.active?.state || null,
                        installing: registration?.installing?.state || null,
                        waiting: registration?.waiting?.state || null,
                        documentBuild: document.querySelector('meta[name="tonk-worker-build"]')?.content || null,
                    })).catch(error => done({ error: String(error) }));
                    "#,
                    vec![],
                )
                .await
            {
                last = state.json().clone();
                if last["health"]["build"] == incumbent
                    && last["documentBuild"] == incumbent
                    && last["discovery"]["build"] == successor
                    && last["active"] == "activated"
                    && last["waiting"] == "installed"
                    && last["controlled"] == true
                {
                    return Ok(last);
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for successor {successor} while controller {incumbent} was held: {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn fetched_asset_digests(
        driver: &WebDriver,
        expected: &BTreeMap<String, String>,
    ) -> Result<Value> {
        let paths = expected.keys().cloned().collect::<Vec<_>>();
        let result = driver
            .execute_async(
                r#"
                const paths = arguments[0];
                const done = arguments[arguments.length - 1];
                (async () => {
                    const digests = {};
                    for (const path of paths) {
                        const response = await fetch(path);
                        if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`);
                        const bytes = await response.arrayBuffer();
                        const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
                        digests[path] = Array.from(hash, byte => byte.toString(16).padStart(2, "0")).join("");
                    }
                    done({ digests });
                })().catch(error => done({ error: String(error) }));
                "#,
                vec![serde_json::to_value(paths)?],
            )
            .await?;
        ensure!(
            result.json()["digests"] == serde_json::to_value(expected)?,
            "generation asset digests were incoherent: expected={expected:?} actual={}",
            result.json()
        );
        Ok(result.json().clone())
    }

    async fn opaque_origin_build_probe(driver: &WebDriver, build: &str) -> Result<Value> {
        let result = driver
            .execute_async(
                r#"
                const build = arguments[0];
                const done = arguments[arguments.length - 1];
                const token = `tonk-cors-${crypto.randomUUID()}`;
                const iframe = document.createElement("iframe");
                iframe.setAttribute("sandbox", "allow-scripts");
                const finish = value => {
                    window.removeEventListener("message", receive);
                    iframe.remove();
                    done(value);
                };
                const timeout = setTimeout(
                    () => finish({ error: "opaque relay timed out" }),
                    10000,
                );
                const receive = async event => {
                    if (event.source !== iframe.contentWindow || event.data?.token !== token) return;
                    if (event.data.type === "request") {
                        try {
                            // A sandboxed opaque document is not a service-
                            // worker client in Chrome. Mirror the production
                            // portal boundary: the trusted parent performs the
                            // authorized fetch and stamps immutable provenance.
                            const options = await fetch("/api/health", {
                                method: "OPTIONS",
                            });
                            const response = await fetch("/api/health", {
                                headers: { "x-tonk-build": build },
                            });
                            iframe.contentWindow.postMessage({
                                token,
                                type: "result",
                                optionsStatus: options.status,
                                allowed: options.headers.get("access-control-allow-headers"),
                                status: response.status,
                                body: await response.json(),
                            }, "*");
                        } catch (error) {
                            iframe.contentWindow.postMessage({
                                token,
                                type: "result",
                                error: String(error),
                            }, "*");
                        }
                        return;
                    }
                    if (event.data.type === "outcome") {
                        clearTimeout(timeout);
                        finish({ ...event.data, opaqueOrigin: event.origin === "null" });
                    }
                };
                window.addEventListener("message", receive);
                iframe.srcdoc = `<script>
                    addEventListener("message", event => {
                        if (event.data?.token !== ${JSON.stringify(token)} || event.data?.type !== "result") return;
                        parent.postMessage({ ...event.data, type: "outcome" }, "*");
                    });
                    parent.postMessage({ token: ${JSON.stringify(token)}, type: "request" }, "*");
                <\/script>`;
                document.body.appendChild(iframe);
                "#,
                vec![build.into()],
            )
            .await?;
        ensure!(
            result.json()["opaqueOrigin"] == true
                && result.json()["optionsStatus"] == 204
                && result.json()["allowed"]
                    .as_str()
                    .is_some_and(|allowed| allowed.split(", ").any(|name| name == "x-tonk-build"))
                && result.json()["status"] == 200
                && result.json()["body"]["build"] == build,
            "opaque relay did not preserve trusted build provenance or the worker OPTIONS contract: {}",
            result.json()
        );
        Ok(result.json().clone())
    }

    #[dialog_common::test]
    async fn it_adopts_a_complete_second_generation_without_mixing_assets(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let build_a = &generation_a.build;
        let build_b = &generation_b.build;
        let initial = wait_for_mounted_build(&driver, build_a).await?;
        assert_eq!(
            initial["health"]["build"].as_str(),
            Some(build_a.as_str()),
            "{initial}"
        );
        assert_eq!(
            initial["health"]["workerWasm"].as_str(),
            Some(generation_a.worker_wasm.as_str()),
            "{initial}"
        );
        fetched_asset_digests(&driver, &generation_a.probes).await?;
        create_state_sentinels(&driver).await?;
        // Hold the old page/controller across publication so requests issued
        // while B is live must still resolve as one coherent A graph.
        set_update_hold(&driver, true).await?;
        promote_second_generation(&env)?;
        request_registration_update(&driver).await?;
        let held = wait_for_successor_while_controller_is_held(&driver, build_a, build_b).await?;
        assert_eq!(
            held["health"]["build"].as_str(),
            Some(build_a.as_str()),
            "{held}"
        );
        assert_eq!(
            held["health"]["workerWasm"].as_str(),
            Some(generation_a.worker_wasm.as_str()),
            "{held}"
        );
        fetched_asset_digests(&driver, &generation_a.probes).await?;

        set_update_hold(&driver, false).await?;
        // This is the user's explicit adoption reload. The still-coherent A
        // document observes the already-installed B worker during boot, nudges
        // activation, and performs at most one guarded alignment reload.
        driver.refresh().await?;
        let state = wait_for_mounted_build(&driver, build_b).await?;
        assert_eq!(state["controlled"], true, "{state}");
        assert_eq!(state["active"], "activated", "{state}");
        assert!(state["installing"].is_null(), "{state}");
        assert!(state["waiting"].is_null(), "{state}");
        assert_eq!(state["mounted"], true, "{state}");
        assert_eq!(state["documents"], 3, "{state}");
        assert_eq!(state["roots"]["1"], true, "{state}");
        assert_eq!(state["roots"]["2"], false, "{state}");
        assert_eq!(state["roots"]["3"], true, "{state}");
        assert_eq!(
            state["health"]["workerWasm"].as_str(),
            Some(generation_b.worker_wasm.as_str()),
            "{state}"
        );
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
        fetched_asset_digests(&driver, &generation_b.probes).await?;

        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_answers_options_and_relays_opaque_build_provenance(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let build = worker_build_id(&env.service_worker_script)?;
        wait_for_mounted_build(&driver, &build).await?;
        opaque_origin_build_probe(&driver, &build).await?;
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_releases_a_waiting_successor_after_old_streams_try_to_reconnect(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let build_a = generation_a.build;
        let build_b = generation_b.build;
        wait_for_mounted_build(&driver, &build_a).await?;
        set_update_hold(&driver, true).await?;

        let query = tonk_worker::helpers::named_concept_wire_query();
        let opened = driver
            .execute_async(
                r#"
                const query = arguments[0];
                const done = arguments[arguments.length - 1];
                (async () => {
                    const queryResponse = await fetch("/api/profile/branch/main/query", {
                        method: "POST",
                        headers: {
                            "content-type": "application/json",
                            "accept": "text/event-stream",
                        },
                        body: JSON.stringify(query),
                    });
                    const queryReader = queryResponse.body.getReader();
                    const first = await queryReader.read();
                    const lspResponse = await fetch("/api/profile/tonk/branch/main/language-server", {
                        headers: { "accept": "text/event-stream" },
                    });
                    const lspReader = lspResponse.body.getReader();
                    globalThis.__tonkRetirementStreams = { queryReader, lspReader };
                    done({
                        queryStatus: queryResponse.status,
                        queryFirstDone: first.done,
                        lspStatus: lspResponse.status,
                    });
                })().catch(error => done({ error: String(error) }));
                "#,
                vec![query.clone()],
            )
            .await?;
        ensure!(
            opened.json()["queryStatus"] == 200
                && opened.json()["queryFirstDone"] == false
                && opened.json()["lspStatus"] == 200,
            "failed to open incumbent query/LSP streams: {}",
            opened.json()
        );

        promote_second_generation(&env)?;
        request_registration_update(&driver).await?;
        wait_for_successor_while_controller_is_held(&driver, &build_a, &build_b).await?;

        let refused = driver
            .execute_async(
                r#"
                const query = arguments[0];
                const done = arguments[arguments.length - 1];
                const probe = async (url, init) => {
                    const response = await fetch(url, init);
                    const type = response.headers.get("content-type");
                    const body = type?.includes("text/event-stream")
                        ? (await response.body.cancel(), null)
                        : await response.json();
                    return { status: response.status, type, body };
                };
                Promise.all([
                    probe("/api/profile/branch/main/query", {
                        method: "POST",
                        headers: {
                            "content-type": "application/json",
                            "accept": "text/event-stream",
                        },
                        body: JSON.stringify(query),
                    }),
                    probe("/api/profile/tonk/branch/main/language-server", {
                        headers: { "accept": "text/event-stream" },
                    }),
                ]).then(([query, lsp]) => done({ query, lsp }))
                  .catch(error => done({ error: String(error) }));
                "#,
                vec![query.clone()],
            )
            .await?;
        for stream in ["query", "lsp"] {
            assert_eq!(refused.json()[stream]["status"], 503, "{refused:?}");
            assert_eq!(
                refused.json()[stream]["body"]["control"],
                "update-pending",
                "{refused:?}"
            );
            assert!(
                !refused.json()[stream]["type"]
                    .as_str()
                    .is_some_and(|content_type| content_type.contains("text/event-stream")),
                "{refused:?}"
            );
        }

        set_update_hold(&driver, false).await?;
        driver.refresh().await?;
        wait_for_mounted_build(&driver, &build_b).await?;
        let successor = driver
            .execute_async(
                r#"
                const query = arguments[0];
                const done = arguments[arguments.length - 1];
                const open = async (url, init) => {
                    const response = await fetch(url, init);
                    const status = response.status;
                    const type = response.headers.get("content-type");
                    await response.body.cancel();
                    return { status, type };
                };
                Promise.all([
                    open("/api/profile/branch/main/query", {
                        method: "POST",
                        headers: {
                            "content-type": "application/json",
                            "accept": "text/event-stream",
                        },
                        body: JSON.stringify(query),
                    }),
                    open("/api/profile/tonk/branch/main/language-server", {
                        headers: { "accept": "text/event-stream" },
                    }),
                ]).then(([query, lsp]) => done({ query, lsp }))
                  .catch(error => done({ error: String(error) }));
                "#,
                vec![query],
            )
            .await?;
        assert_eq!(successor.json()["query"]["status"], 200, "{successor:?}");
        assert_eq!(successor.json()["lsp"]["status"], 200, "{successor:?}");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_keeps_sibling_tabs_on_their_controller_until_claim_is_safe(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let primary = driver.window().await?;
        let build_a = generation_a.build;
        let build_b = generation_b.build;
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
        driver.refresh().await?;
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
        let caches_before = driver
            .execute_async(
                r#"
                const done = arguments[arguments.length - 1];
                caches.keys().then(done).catch(error => done({ error: String(error) }));
                "#,
                vec![],
            )
            .await?;

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
        assert_eq!(
            &after.json()["cacheNames"],
            caches_before.json(),
            "{after:?}"
        );
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
