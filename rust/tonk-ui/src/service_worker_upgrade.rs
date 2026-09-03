//! Real-browser service-worker load-time upgrade tests.

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

    fn cache_belongs_to_build(name: &str, build: &str) -> bool {
        name == format!("TONK_SHELL_{build}")
            || name == format!("TONK_WORKER_{build}")
            || name == format!("TONK_GENERATION_{build}")
            || name.starts_with(&format!("TONK_SHELL_STAGE_{build}_"))
            || name.starts_with(&format!("TONK_WORKER_STAGE_{build}_"))
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
    /// same-tab reloads but not test environments; the root map records which
    /// observed documents reached the application mount before a later reload.
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
        // Runtime remapping wins for binaries from the `tests-e2e` archive:
        // their compile-time manifest path names the discarded Nix sandbox.
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_string());
        let stamp = Path::new(&manifest_dir)
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
                            evictionGuard: sessionStorage.getItem("tonk:sw-eviction-reload"),
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
                        const generationCache = `TONK_GENERATION_${expectedBuild}`;
                        const generationMarkerUrl = new URL(
                            `/.tonk-generation-${expectedBuild}`,
                            location.origin,
                        ).href;
                        const [healthResponse, cacheNames, manifestResponse, markerResponse] = await Promise.all([
                            fetch("/api/health"),
                            caches.keys(),
                            fetch("/asset-manifest.json", { cache: "no-store" }),
                            caches.match(generationMarkerUrl, { cacheName: generationCache }),
                        ]);
                        const healthBody = await healthResponse.text();
                        const manifestBody = await manifestResponse.text();
                        const markerBody = markerResponse ? await markerResponse.text() : "";
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
                            generationMarker: parse(markerBody),
                            generationMarkerBody: markerBody.slice(0, 200),
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

    async fn wait_for_complete_generation(
        driver: &WebDriver,
        generation: &GenerationContract,
        expected_documents: Option<u64>,
        obsolete_build: Option<&str>,
    ) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let state = wait_for_mounted_build(driver, &generation.build).await?;
            let cache_names = state["cacheNames"].as_array();
            let has_cache = |expected: &str| {
                cache_names.is_some_and(|names| names.iter().any(|name| name == expected))
            };
            let obsolete_pruned = obsolete_build.is_none_or(|build| {
                cache_names.is_some_and(|names| {
                    names.iter().all(|name| {
                        !name
                            .as_str()
                            .is_some_and(|name| cache_belongs_to_build(name, build))
                    })
                })
            });
            let documents_ready = expected_documents
                .is_none_or(|expected| state["documents"].as_u64() == Some(expected));
            if state["health"]["worker"] == "ok"
                && state["health"]["workerWasm"] == generation.worker_wasm
                && state["generationMarker"]["build"] == generation.build
                && state["generationMarker"]["state"] == "adopted"
                && has_cache(&format!("TONK_SHELL_{}", generation.build))
                && has_cache(&format!("TONK_WORKER_{}", generation.build))
                && has_cache(&format!("TONK_GENERATION_{}", generation.build))
                && documents_ready
                && obsolete_pruned
            {
                return Ok(state);
            }
            if let Some(expected) = expected_documents
                && state["documents"]
                    .as_u64()
                    .is_some_and(|actual| actual > expected)
            {
                return Err(anyhow!(
                    "document count exceeded {expected} while waiting for complete generation {}: {state}",
                    generation.build
                ));
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for complete generation {} with documents={expected_documents:?} and obsolete build {obsolete_build:?} pruned: {state}",
                generation.build
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
                            const response = await fetch("/api/health");
                            iframe.contentWindow.postMessage({
                                token,
                                type: "result",
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
                && result.json()["status"] == 200
                && result.json()["body"]["build"] == build,
            "opaque relay did not preserve trusted build provenance: {}",
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
        let initial = wait_for_complete_generation(&driver, &generation_a, None, None).await?;
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
        promote_second_generation(&env)?;
        // An ordinary warm load mounts A immediately while its update check
        // discovers B in the background. B activates automatically, and the
        // update-aware A document performs one guarded alignment reload.
        driver.refresh().await?;
        let state =
            wait_for_complete_generation(&driver, &generation_b, Some(3), Some(build_a)).await?;
        assert_eq!(state["controlled"], true, "{state}");
        assert_eq!(state["active"], "activated", "{state}");
        assert!(state["installing"].is_null(), "{state}");
        assert!(state["waiting"].is_null(), "{state}");
        assert_eq!(state["mounted"], true, "{state}");
        assert_eq!(state["documents"], 3, "{state}");
        assert_eq!(state["roots"]["1"], true, "{state}");
        assert_eq!(state["roots"]["2"], true, "{state}");
        assert_eq!(state["roots"]["3"], true, "{state}");
        assert_eq!(
            state["health"]["workerWasm"].as_str(),
            Some(generation_b.worker_wasm.as_str()),
            "{state}"
        );
        assert!(state["guard"].is_null(), "{state}");
        assert!(state["evictionGuard"].is_null(), "{state}");
        assert!(
            state["cacheNames"]
                .as_array()
                .is_some_and(|names| names.iter().all(|name| !name
                    .as_str()
                    .is_some_and(|name| cache_belongs_to_build(name, build_a)))),
            "the incumbent lifecycle caches must be pruned: {state}"
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
    async fn it_reloads_every_update_aware_tab_after_controller_replacement(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let first_tab = driver.window().await?;
        wait_for_mounted_build(&driver, &generation_a.build).await?;
        create_state_sentinels(&driver).await?;

        let second_tab = driver.new_tab().await?;
        driver.switch_to_window(second_tab.clone()).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        wait_for_mounted_build(&driver, &generation_a.build).await?;

        promote_second_generation(&env)?;
        driver.switch_to_window(first_tab.clone()).await?;
        driver.refresh().await?;
        let first =
            wait_for_complete_generation(&driver, &generation_b, None, Some(&generation_a.build))
                .await?;
        let first_documents = first["documents"]
            .as_u64()
            .context("the first tab reported no document count")?;
        assert!(
            matches!(first_documents, 2 | 3),
            "the first tab must mount B directly or after one alignment reload: {first}"
        );
        assert_eq!(first["roots"]["1"], true, "{first}");
        assert_eq!(first["roots"]["2"], true, "{first}");
        if first_documents == 3 {
            assert_eq!(first["roots"]["3"], true, "{first}");
        }

        driver.switch_to_window(second_tab).await?;
        let second = wait_for_complete_generation(
            &driver,
            &generation_b,
            Some(2),
            Some(&generation_a.build),
        )
        .await?;
        assert_eq!(second["documents"], 2, "{second}");
        assert_eq!(second["roots"]["1"], true, "{second}");
        assert_eq!(second["roots"]["2"], true, "{second}");
        assert!(second["guard"].is_null(), "{second}");
        assert!(second["evictionGuard"].is_null(), "{second}");
        assert!(
            second["cacheNames"]
                .as_array()
                .is_some_and(|names| names.iter().all(|name| !name
                    .as_str()
                    .is_some_and(|name| cache_belongs_to_build(name, &generation_a.build)))),
            "the incumbent lifecycle caches must be pruned: {second}"
        );
        fetched_asset_digests(&driver, &generation_b.probes).await?;
        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.switch_to_window(first_tab).await?;
        let first_after = wait_for_mounted_build(&driver, &generation_b.build).await?;
        assert_eq!(first_after["documents"], first_documents, "{first_after}");
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_recovers_an_evicted_root_into_the_current_generation(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        create_state_sentinels(&driver).await?;
        let removed = driver
            .execute_async(
                r#"
                const cacheName = arguments[0];
                const done = arguments[arguments.length - 1];
                caches.open(cacheName)
                    .then(cache => cache.delete("/"))
                    .then(removed => done({ removed }))
                    .catch(error => done({ error: String(error) }));
                "#,
                vec![format!("TONK_SHELL_{}", generation_a.build).into()],
            )
            .await?;
        ensure!(
            removed.json()["removed"] == true,
            "root eviction failed: {removed:?}"
        );

        promote_second_generation(&env)?;
        driver.refresh().await?;
        let state = wait_for_complete_generation(
            &driver,
            &generation_b,
            Some(2),
            Some(&generation_a.build),
        )
        .await?;
        assert_eq!(state["documents"], 2, "{state}");
        assert_eq!(state["roots"]["1"], true, "{state}");
        assert_eq!(state["roots"]["2"], true, "{state}");
        assert!(state["guard"].is_null(), "{state}");
        assert!(state["evictionGuard"].is_null(), "{state}");
        assert!(
            state["cacheNames"]
                .as_array()
                .is_some_and(|names| names.iter().all(|name| !name
                    .as_str()
                    .is_some_and(|name| cache_belongs_to_build(name, &generation_a.build)))),
            "the evicted incumbent lifecycle caches must be pruned: {state}"
        );
        fetched_asset_digests(&driver, &generation_b.probes).await?;
        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_relays_build_provenance_to_an_opaque_child(env: TestEnvironment) -> Result<()> {
        let driver = env.driver().await?;
        let build = worker_build_id(&env.service_worker_script)?;
        wait_for_mounted_build(&driver, &build).await?;
        opaque_origin_build_probe(&driver, &build).await?;
        driver.quit().await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn it_releases_incumbent_streams_for_an_automatic_successor(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let build_a = generation_a.build;
        let build_b = generation_b.build;
        wait_for_mounted_build(&driver, &build_a).await?;

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
                    const lspResponse = await fetch("/api/language-server", {
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
        // The warm load drives automatic activation. Reaching mounted B proves
        // the incumbent streams did not pin A during the handoff.
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
                    open("/api/language-server", {
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
