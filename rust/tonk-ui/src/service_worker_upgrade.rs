//! Real-browser service-worker load-time upgrade tests.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
pub(crate) mod tests {
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

    /// Enter the frame the page mounts the profile's site in. `false` while
    /// the page has none.
    async fn enter_site(driver: &WebDriver) -> bool {
        if driver.enter_default_frame().await.is_err() {
            return false;
        }
        match driver.find(By::Css("tonk-site > iframe")).await {
            Ok(frame) => frame.enter_frame().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Run an asynchronous `script` in the profile's site, whose worker
    /// holds the database, and come back to the page.
    async fn in_site(driver: &WebDriver, script: &str, arguments: Vec<Value>) -> Result<Value> {
        ensure!(enter_site(driver).await, "the page frames no site");
        let result = driver.execute_async(script, arguments).await;
        driver.enter_default_frame().await?;
        Ok(result?.json().clone())
    }

    /// How the profile's worker says it is: answered by its script, with
    /// the wasm it booted, when it started, and what it has logged.
    async fn worker_health(driver: &WebDriver) -> Result<Value> {
        in_site(
            driver,
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
        .await
    }

    /// One look at the page and at the profile's site in it: which build
    /// each is, which worker controls it, and how far it has come up.
    async fn observed(driver: &WebDriver) -> Value {
        let _ = driver.enter_default_frame().await;
        let page = driver
            .execute_async(
                r##"
                const done = arguments[arguments.length - 1];
                (async () => {
                    const registration = await navigator.serviceWorker.getRegistration();
                    const site = document.querySelector("tonk-site");
                    done({
                        build: document.querySelector('meta[name="tonk-worker-build"]')?.content || null,
                        controlled: !!navigator.serviceWorker.controller,
                        active: registration?.active?.state || null,
                        installing: registration?.installing?.state || null,
                        waiting: registration?.waiting?.state || null,
                        caches: await caches.keys(),
                        mounted: !!site,
                        ready: !!site?.hasAttribute("data-ready"),
                        guard: sessionStorage.getItem("tonk:sw-upgrade-reload"),
                        documents: Number(sessionStorage.getItem("tonk:test:sw-documents")) || 0,
                        roots: JSON.parse(sessionStorage.getItem("tonk:test:sw-roots") || "{}"),
                    });
                })().catch(error => done({ error: String(error) }));
                "##,
                vec![],
            )
            .await
            .map(|value| value.json().clone())
            .unwrap_or(Value::Null);
        let site = in_site(
            driver,
            r##"
            const done = arguments[arguments.length - 1];
            (async () => {
                const registration = await navigator.serviceWorker.getRegistration();
                // Asking a worker anything keeps it busy, and the browser
                // neither looks for a newer one nor lets a successor take
                // over from a busy one: while one is on its way, only what
                // the registration says is read.
                const settled = !registration?.installing && !registration?.waiting;
                const health = settled
                    ? await fetch("/api/health")
                        .then(response => response.json())
                        .catch(error => ({ error: String(error) }))
                    : {};
                done({
                    controlled: !!navigator.serviceWorker.controller,
                    active: registration?.active?.state || null,
                    installing: registration?.installing?.state || null,
                    waiting: registration?.waiting?.state || null,
                    worker: health.worker ?? null,
                    workerWasm: health.workerWasm ?? null,
                    startedAt: health.startedAt ?? null,
                    failure: health.error ?? null,
                    // When this document of the site loaded: it changes
                    // when the frame loads again.
                    loaded: Math.round(performance.timeOrigin),
                    rendered: !!document.querySelector("tonk-display"),
                    guest: globalThis.__tonkTestGuestGeneration ?? null,
                });
            })().catch(error => done({ error: String(error) }));
            "##,
            vec![],
        )
        .await
        .unwrap_or(Value::Null);
        serde_json::json!({ "page": page, "site": site })
    }

    /// The last of what the profile's worker logged, for a failure report.
    async fn worker_log(driver: &WebDriver) -> String {
        let health = worker_health(driver).await.unwrap_or(Value::Null);
        health["body"]["log"]
            .as_array()
            .map(|log| {
                log.iter()
                    .rev()
                    .take(60)
                    .rev()
                    .filter_map(|entry| {
                        let message = entry["message"].as_str()?;
                        let at = entry["t"].as_u64().unwrap_or_default();
                        Some(format!(
                            "{at} {}",
                            message.chars().take(200).collect::<String>()
                        ))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// Wait for the profile's worker to be one that started at another time
    /// than `previous`, and say when it started.
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
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for a different worker; health={last}, state={}",
                observed(driver).await
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn wait_for_guest_selector(driver: &WebDriver, selector: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            if enter_site(driver).await && driver.find(By::Css(selector.to_owned())).await.is_ok() {
                return Ok(());
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for guest selector {selector:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Leave something in the profile's own storage, where a person's data
    /// is, for an upgrade to keep.
    async fn create_state_sentinels(driver: &WebDriver) -> Result<()> {
        let result = in_site(
            driver,
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
            result["ok"] == true,
            "failed to create state sentinels: {result}"
        );
        Ok(())
    }

    async fn state_sentinels(driver: &WebDriver) -> Result<Value> {
        in_site(
            driver,
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
        .await
    }

    /// Wait for the page to have mounted its site under the profile's
    /// worker that started at `started_at`.
    async fn wait_for_mounted_worker(driver: &WebDriver, started_at: u64) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let last = observed(driver).await;
            if last["site"]["startedAt"].as_u64() == Some(started_at)
                && last["site"]["rendered"] == true
                && last["page"]["ready"] == true
            {
                return Ok(last);
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the site to mount under worker {started_at}: {last}"
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
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

    /// Whether the app's worker of `build` kept the cache `name`.
    fn cache_belongs_to_build(name: &str, build: &str) -> bool {
        name == format!("TONK_APP_{build}")
    }

    #[derive(Debug)]
    pub(crate) struct GenerationContract {
        pub(crate) build: String,
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
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(guest_glue(root)?)?,
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

    /// The guest glue of `root`, which the portal injects into every guest.
    fn guest_glue(root: &Path) -> Result<std::path::PathBuf> {
        std::fs::read_dir(root.join("guest"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("guest-") && name.ends_with(".js"))
            })
            .ok_or_else(|| anyhow!("generation has no guest glue"))
    }

    /// Record `generation` in the guest realm, so a test can tell which
    /// guest runtime a mounted site is running.
    ///
    /// The glue is named for its content, as every build names it: a site's
    /// worker keeps a file so named for good, and one changed under the
    /// same name would never be asked for again.
    fn mark_guest_generation(root: &Path, generation: &str) -> Result<()> {
        use std::hash::{Hash as _, Hasher as _};

        let glue = guest_glue(root)?;
        let source = std::fs::read_to_string(&glue)?;
        let marker = "globalThis.__tonkTestGuestGeneration = ";
        let unmarked = source
            .split_once(&format!("\n{marker}"))
            .map_or(source.as_str(), |(before, _)| before);
        let marked = format!("{unmarked}\n{marker}{generation:?};\n");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        marked.hash(&mut hasher);
        let named = format!("guest-{:016x}.js", hasher.finish());
        let previous = glue
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("the guest glue has no name"))?
            .to_owned();
        std::fs::remove_file(&glue)?;
        std::fs::write(glue.with_file_name(&named), marked)?;
        let manifest = root.join("guest/manifest.json");
        let listed = std::fs::read_to_string(&manifest)?;
        ensure!(
            listed.contains(&previous),
            "the guest manifest does not name its glue"
        );
        std::fs::write(&manifest, listed.replace(&previous, &named))?;
        Ok(())
    }

    /// A and B whose pages are identical and whose sites differ: another
    /// worker and another guest runtime, the shape of a deploy that changed
    /// nothing of the page itself.
    fn prepare_site_only_generation(
        env: &TestEnvironment,
    ) -> Result<(GenerationContract, GenerationContract)> {
        let generation_a = env.deployment_root.join("generation-a");
        let generation_b = env.deployment_root.join("generation-b");
        instrument_generation_documents(&generation_a)?;
        mark_guest_generation(&generation_a, "A")?;
        stamp_generation(&generation_a)?;
        let generation_a_contract = generation_contract(&generation_a)?;
        copy_artifact_tree(&generation_a, &generation_b)?;
        mark_guest_generation(&generation_b, "B")?;
        append_wasm_generation_marker(&generation_b.join("worker_bg.wasm"))?;
        stamp_generation(&generation_b)?;
        let generation_b_contract = generation_contract(&generation_b)?;
        ensure!(
            generation_a_contract.build != generation_b_contract.build,
            "A and B must have distinct build ids"
        );
        ensure!(
            generation_a_contract.worker_wasm != generation_b_contract.worker_wasm,
            "A and B must have distinct site workers"
        );
        let page = |root: &Path| -> Result<Value> {
            let version = std::fs::read_to_string(root.join("version.json"))?;
            Ok(serde_json::from_str::<Value>(&version)?["page"].clone())
        };
        ensure!(
            page(&generation_a)? == page(&generation_b)?,
            "a change to the site alone must keep the page build"
        );
        Ok((generation_a_contract, generation_b_contract))
    }

    /// Which guest runtime the mounted top-level site is running.
    async fn wait_for_guest_generation(driver: &WebDriver, generation: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut last = Value::Null;
        loop {
            if enter_site(driver).await
                && let Ok(value) = driver
                    .execute(
                        "return globalThis.__tonkTestGuestGeneration ?? null;",
                        vec![],
                    )
                    .await
            {
                last = value.json().clone();
                if last == generation {
                    driver.enter_default_frame().await?;
                    return Ok(());
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for guest generation {generation}; last={last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The mounted top-level guest's rendered text, once it is non-empty and
    /// unchanged across two reads.
    async fn settled_guest_text(driver: &WebDriver) -> Result<String> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut last = String::new();
        loop {
            let mut current = String::new();
            if enter_site(driver).await
                && let Ok(text) = driver
                    .execute(
                        r#"
                        const skipped = new Set(["STYLE", "SCRIPT", "TEMPLATE"]);
                        const text = node => [...node.childNodes].map(child =>
                            child.nodeType === Node.TEXT_NODE
                                ? child.textContent
                                : child.nodeType === Node.ELEMENT_NODE && !skipped.has(child.tagName)
                                    ? (child.shadowRoot ? text(child.shadowRoot) + " " : "") + text(child)
                                    : "").join(" ");
                        return document.body ? text(document.body).replace(/\s+/g, " ") : "";
                        "#,
                        vec![],
                    )
                    .await
            {
                current = text.json().as_str().unwrap_or_default().trim().to_owned();
            }
            driver.enter_default_frame().await?;
            if !current.is_empty() && current == last {
                return Ok(current);
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for settled guest text; last={last:?}"
            );
            last = current;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    pub(crate) fn prepare_profile_library_generations(
        env: &TestEnvironment,
    ) -> Result<(GenerationContract, GenerationContract)> {
        let (_, generation_b) = prepare_second_generation(env)?;
        let generation_a_root = env.deployment_root.join("generation-a");
        let profile_library = generation_a_root.join("library/profile.yaml");
        let current = std::fs::read_to_string(&profile_library)
            .with_context(|| format!("read {}", profile_library.display()))?;
        let marker = "data-spaces-view aria-label=\"spaces\">";
        // The sentence is reduced from profile.yaml at eff85b2ab^, the last
        // revision before that historical empty-state row was removed.
        let historical = current.replacen(
            marker,
            &format!("{marker}\n          <div class=\"sempty\">no spaces yet</div>"),
            1,
        );
        ensure!(
            historical != current,
            "the historical profile-library fixture must differ"
        );
        std::fs::write(&profile_library, historical)
            .with_context(|| format!("write {}", profile_library.display()))?;
        stamp_generation(&generation_a_root)?;
        let generation_a = generation_contract(&generation_a_root)?;
        ensure!(
            generation_a.build != generation_b.build,
            "the historical and current generations must have distinct build ids"
        );
        Ok((generation_a, generation_b))
    }

    #[cfg(unix)]
    pub(crate) fn promote_second_generation(env: &TestEnvironment) -> Result<()> {
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

    /// Wait for the page to be the document of `build`, with its site up.
    async fn wait_for_mounted_build(driver: &WebDriver, build: &str) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let last = observed(driver).await;
            if last["page"]["build"] == build
                && last["page"]["ready"] == true
                && last["site"]["worker"] == "ok"
                && last["site"]["rendered"] == true
            {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for the page of build {build} with its site up: {last}\nprofile worker log:\n{}",
                    worker_log(driver).await
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Wait for the profile's site to be served by the worker built with
    /// `generation`, and up under it.
    pub(crate) async fn wait_for_site_generation(
        driver: &WebDriver,
        generation: &GenerationContract,
    ) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let last = observed(driver).await;
            let site = &last["site"];
            if site["worker"] == "ok"
                && site["workerWasm"] == generation.worker_wasm.as_str()
                && site["controlled"] == true
                && site["active"] == "activated"
                && site["installing"].is_null()
                && site["waiting"].is_null()
                && site["rendered"] == true
                && last["page"]["ready"] == true
            {
                return Ok(last);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for the site under the worker of {}: {last}\nprofile worker log:\n{}",
                    generation.worker_wasm,
                    worker_log(driver).await
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn wait_for_hub_snapshot(driver: &WebDriver) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        let mut last = Value::Null;
        loop {
            if enter_site(driver).await && driver.find(By::Css(".hub-page")).await.is_ok() {
                let snapshot = driver
                    .execute(
                        r#"
                        return {
                            text: document.body.innerText,
                            spaces: [...document.querySelectorAll(".space-card > a.srow")].map(link => ({
                                href: link.getAttribute("href"),
                                name: link.textContent.trim(),
                            })),
                            createEnabled: !!document.querySelector("button.snew:not(:disabled)"),
                        };
                        "#,
                        vec![],
                    )
                    .await?;
                last = snapshot.json().clone();
                // The roster can render before branch-defined controls
                // upgrade. A populated Hub must also be ready to create.
                if last["spaces"]
                    .as_array()
                    .is_some_and(|spaces| !spaces.is_empty())
                    && last["createEnabled"] == true
                {
                    return Ok(last);
                }
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for the populated, interactive Hub: {last}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Wait for the whole of `generation` to be what runs: the page is its
    /// document under its worker, which keeps only its files, and the
    /// profile's site is up under the worker built with it. `obsolete_build`
    /// is a build nothing may be kept of any more.
    pub(crate) async fn wait_for_complete_generation(
        driver: &WebDriver,
        generation: &GenerationContract,
        expected_documents: Option<u64>,
        obsolete_build: Option<&str>,
    ) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        loop {
            let state = observed(driver).await;
            let (page, site) = (&state["page"], &state["site"]);
            let kept = page["caches"].as_array();
            let keeps = |build: &str| {
                kept.is_some_and(|names| {
                    names.iter().any(|name| {
                        name.as_str()
                            .is_some_and(|name| cache_belongs_to_build(name, build))
                    })
                })
            };
            let settled = |side: &Value| {
                side["controlled"] == true
                    && side["active"] == "activated"
                    && side["installing"].is_null()
                    && side["waiting"].is_null()
            };
            let documents_ready = expected_documents
                .is_none_or(|expected| page["documents"].as_u64() == Some(expected));
            if page["build"] == generation.build.as_str()
                && settled(page)
                && page["ready"] == true
                && keeps(&generation.build)
                && obsolete_build.is_none_or(|build| !keeps(build))
                && settled(site)
                && site["worker"] == "ok"
                && site["workerWasm"] == generation.worker_wasm.as_str()
                && site["rendered"] == true
                && documents_ready
            {
                return Ok(state);
            }
            if let Some(expected) = expected_documents
                && page["documents"]
                    .as_u64()
                    .is_some_and(|actual| actual > expected)
            {
                return Err(anyhow!(
                    "the page loaded more than {expected} times while waiting for complete generation {}: {state}",
                    generation.build
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "timed out waiting for complete generation {} with documents={expected_documents:?} and nothing kept of {obsolete_build:?}: {state}\nprofile worker log:\n{}",
                    generation.build,
                    worker_log(driver).await
                );
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// What the page is served under each of `expected`'s addresses, which
    /// has to be this generation's bytes and no other's.
    async fn fetched_asset_digests(
        driver: &WebDriver,
        expected: &BTreeMap<String, String>,
    ) -> Result<Value> {
        driver.enter_default_frame().await?;
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

    /// Have the browser look for a newer worker for the profile's site, the
    /// way it does on its own: when the site is loaded. A page may not ask
    /// for it, the site's policy letting none of its documents start a
    /// worker, so another tab loads the app, and with it the site. The tab
    /// this was called in is left as it was, and is the one returned to.
    async fn update_site_worker(driver: &WebDriver) -> Result<()> {
        driver.enter_default_frame().await?;
        let here = driver.window().await?;
        let app = driver.current_url().await?.join("/")?;
        let other = driver.new_tab().await?;
        driver.switch_to_window(other).await?;
        driver.goto(app.as_str()).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !enter_site(driver).await {
            ensure!(
                tokio::time::Instant::now() < deadline,
                "the tab opened to find the update never framed the site"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        driver.enter_default_frame().await?;
        driver.switch_to_window(here).await?;
        Ok(())
    }

    /// How many documents the page has been in this tab.
    async fn documents(driver: &WebDriver) -> Result<u64> {
        driver.enter_default_frame().await?;
        let count = driver
            .execute(
                r#"return Number(sessionStorage.getItem("tonk:test:sw-documents")) || 0;"#,
                vec![],
            )
            .await?;
        Ok(count.json().as_u64().unwrap_or(0))
    }

    /// Cut the browser off from the network, or put it back.
    async fn set_offline(driver: &WebDriver, offline: bool) -> Result<()> {
        driver.enter_default_frame().await?;
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools.execute_cdp("Network.enable").await?;
        devtools
            .execute_cdp_with_params(
                "Network.emulateNetworkConditions",
                serde_json::json!({
                    "offline": offline,
                    "latency": 0,
                    "downloadThroughput": if offline { 0 } else { -1 },
                    "uploadThroughput": if offline { 0 } else { -1 },
                }),
            )
            .await?;
        Ok(())
    }

    #[dialog_common::test]
    async fn profile_library_reconciles_across_a_persisted_worker_upgrade(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_profile_library_generations(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        create_state_sentinels(&driver).await?;
        // A fresh profile has no spaces; give the upgrade a persisted roster to preserve.
        crate::account_flow::tests::create_space(&driver, "Upgrade fixture").await?;

        driver.enter_default_frame().await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let historical = wait_for_hub_snapshot(&driver)
            .await
            .context("the hub of generation A")?;
        ensure!(
            historical["text"]
                .as_str()
                .is_some_and(|text| text.contains("no spaces yet")),
            "generation A did not render the historical profile facet: {historical}"
        );
        let spaces = historical["spaces"].clone();

        promote_second_generation(&env)?;
        driver.enter_default_frame().await?;
        driver.refresh().await?;
        let current =
            wait_for_complete_generation(&driver, &generation_b, None, Some(&generation_a.build))
                .await?;
        let current_started_at = current["site"]["startedAt"]
            .as_u64()
            .context("the current worker reported no start time")?;

        driver.goto(env.tonk_web.as_str()).await?;
        let repaired = wait_for_hub_snapshot(&driver)
            .await
            .context("the hub after the upgrade")?;
        ensure!(
            repaired["text"]
                .as_str()
                .is_some_and(|text| !text.contains("no spaces yet")),
            "generation B retained the obsolete profile facet: {repaired}"
        );
        ensure!(repaired["createEnabled"] == true, "{repaired}");
        ensure!(
            repaired["spaces"] == spaces,
            "the worker update changed the persisted space roster: before={spaces} after={repaired}"
        );

        driver
            .find(By::Css(".space-card > a.srow"))
            .await?
            .click()
            .await?;
        driver.enter_default_frame().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if driver.current_url().await?.path().starts_with("/space/") {
                break;
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "the retained Hub space link did not navigate"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        driver.goto(env.tonk_web.join("settings")?.as_str()).await?;
        wait_for_guest_selector(&driver, "account-settings").await?;
        driver.enter_default_frame().await?;

        // The repaired library is the profile's own, kept with it: a worker
        // that starts again with no network still shows it.
        set_offline(&driver, true).await?;
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools.execute_cdp("ServiceWorker.enable").await?;
        devtools.execute_cdp("ServiceWorker.stopAllWorkers").await?;
        let offline_result: Result<()> = async {
            driver.refresh().await?;
            driver.goto(env.tonk_web.as_str()).await?;
            let offline = wait_for_hub_snapshot(&driver)
                .await
                .context("the hub with no network")?;
            ensure!(offline["spaces"] == spaces, "{offline}");
            ensure!(
                offline["text"]
                    .as_str()
                    .is_some_and(|text| !text.contains("no spaces yet")),
                "the offline load lost the repaired facet: {offline}"
            );
            Ok(())
        }
        .await;
        set_offline(&driver, false).await?;
        offline_result?;

        driver.enter_default_frame().await?;
        driver.refresh().await?;
        wait_for_site_generation(&driver, &generation_b).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        let reconnected = wait_for_hub_snapshot(&driver)
            .await
            .context("the hub once the network is back")?;
        ensure!(reconnected["spaces"] == spaces, "{reconnected}");
        ensure!(
            reconnected["text"]
                .as_str()
                .is_some_and(|text| !text.contains("no spaces yet")),
            "the reconnected sweep did not preserve the repaired facet: {reconnected}"
        );
        let _ = current_started_at;

        let sentinels = state_sentinels(&driver).await?;
        ensure!(sentinels["indexedDb"] == "preserved", "{sentinels}");
        ensure!(sentinels["cache"] == "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    /// A deploy replaces every layer at once, and a visit after it ends on
    /// the new one whole: the page is the new document on the first load,
    /// its worker and the profile's hand over to their successors, the site
    /// loads again under the worker that now serves it, and nothing of the
    /// build before is kept or served.
    #[dialog_common::test]
    async fn it_adopts_a_complete_second_generation_without_mixing_assets(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let initial = wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        assert_eq!(
            initial["site"]["workerWasm"].as_str(),
            Some(generation_a.worker_wasm.as_str()),
            "{initial}"
        );
        fetched_asset_digests(&driver, &generation_a.probes).await?;
        create_state_sentinels(&driver).await?;
        let site_before = initial["site"]["loaded"].clone();

        promote_second_generation(&env)?;
        // The page is asked of the server first, so this load is B's
        // document at once, and it stays: the worker that takes over is its
        // own build's.
        driver.refresh().await?;
        let state = wait_for_complete_generation(
            &driver,
            &generation_b,
            Some(2),
            Some(&generation_a.build),
        )
        .await?;
        let page = &state["page"];
        assert_eq!(page["documents"], 2, "{state}");
        assert_eq!(page["roots"]["1"], true, "{state}");
        assert_eq!(page["roots"]["2"], true, "{state}");
        assert!(
            page["guard"].is_null(),
            "the page was not loaded again: {state}"
        );
        assert_ne!(
            state["site"]["loaded"], site_before,
            "the site is a document the new worker served: {state}"
        );
        fetched_asset_digests(&driver, &generation_b.probes).await?;

        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    /// A tab left open across a deploy is the build before, under a worker
    /// that is not. When the new worker takes over, each such tab loads the
    /// page that goes with it, once.
    #[dialog_common::test]
    async fn it_reloads_every_update_aware_tab_after_controller_replacement(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        let first_tab = driver.window().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        create_state_sentinels(&driver).await?;

        let second_tab = driver.new_tab().await?;
        driver.switch_to_window(second_tab.clone()).await?;
        driver.goto(env.tonk_web.as_str()).await?;
        wait_for_mounted_build(&driver, &generation_a.build).await?;

        promote_second_generation(&env)?;
        driver.switch_to_window(first_tab.clone()).await?;
        driver.refresh().await?;
        let first = wait_for_complete_generation(
            &driver,
            &generation_b,
            Some(2),
            Some(&generation_a.build),
        )
        .await?;
        assert_eq!(first["page"]["roots"]["1"], true, "{first}");
        assert_eq!(first["page"]["roots"]["2"], true, "{first}");
        assert!(first["page"]["guard"].is_null(), "{first}");

        // The other tab was never touched: it follows the worker.
        driver.switch_to_window(second_tab).await?;
        let second = wait_for_complete_generation(
            &driver,
            &generation_b,
            Some(2),
            Some(&generation_a.build),
        )
        .await?;
        assert_eq!(second["page"]["documents"], 2, "{second}");
        assert_eq!(second["page"]["roots"]["1"], true, "{second}");
        assert_eq!(second["page"]["roots"]["2"], true, "{second}");
        assert!(
            second["page"]["guard"].is_string(),
            "it loaded again for this page build, and will not for it twice: {second}"
        );
        fetched_asset_digests(&driver, &generation_b.probes).await?;
        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.switch_to_window(first_tab).await?;
        let first_after = wait_for_mounted_build(&driver, &generation_b.build).await?;
        assert_eq!(first_after["page"]["documents"], 2, "{first_after}");
        driver.quit().await?;
        Ok(())
    }

    /// The browser may drop what a worker kept. A page whose kept copy is
    /// gone still loads, and a deploy after that is taken up as any other.
    #[dialog_common::test]
    async fn it_recovers_an_evicted_root_into_the_current_generation(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        create_state_sentinels(&driver).await?;
        driver.enter_default_frame().await?;
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
                vec![format!("TONK_APP_{}", generation_a.build).into()],
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
        assert_eq!(state["page"]["roots"]["1"], true, "{state}");
        assert_eq!(state["page"]["roots"]["2"], true, "{state}");
        assert!(state["page"]["guard"].is_null(), "{state}");
        fetched_asset_digests(&driver, &generation_b.probes).await?;
        let sentinels = state_sentinels(&driver).await?;
        assert_eq!(sentinels["indexedDb"], "preserved", "{sentinels}");
        assert_eq!(sentinels["cache"], "preserved", "{sentinels}");

        driver.quit().await?;
        Ok(())
    }

    /// A subscription is a response that never ends, and a worker with one
    /// open counts as busy for good. The profile's worker lets its streams
    /// go when a successor installs, or the successor would wait behind
    /// them and the site would never leave the build before.
    #[dialog_common::test]
    async fn it_releases_incumbent_streams_for_an_automatic_successor(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;

        let query = tonk_worker::helpers::named_concept_wire_query();
        let opened = in_site(
            &driver,
            r#"
            const query = arguments[0];
            const done = arguments[arguments.length - 1];
            (async () => {
                const profiles = await (await fetch("/api/profiles")).json();
                const queryResponse = await fetch(`/api/repository/profile:tonk/branch/${profiles.active}/query`, {
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
            opened["queryStatus"] == 200
                && opened["queryFirstDone"] == false
                && opened["lspStatus"] == 200,
            "failed to open incumbent query/LSP streams: {opened}"
        );

        promote_second_generation(&env)?;
        // The site stays open, with its streams, while the browser finds
        // the successor. Coming up under B proves they did not hold A.
        update_site_worker(&driver).await?;
        wait_for_site_generation(&driver, &generation_b).await?;
        let successor = in_site(
            &driver,
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
            (async () => {
                const profiles = await (await fetch("/api/profiles")).json();
                const [query_, lsp] = await Promise.all([
                    open(`/api/repository/profile:tonk/branch/${profiles.active}/query`, {
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
                ]);
                done({ query: query_, lsp });
            })().catch(error => done({ error: String(error) }));
            "#,
            vec![query],
        )
        .await?;
        assert_eq!(successor["query"]["status"], 200, "{successor}");
        assert_eq!(successor["lsp"]["status"], 200, "{successor}");

        driver.quit().await?;
        Ok(())
    }

    /// Chrome activates a `skipWaiting` successor only once the outgoing
    /// worker has no pending events. A site busy with ordinary work (asset
    /// traffic, a query every quarter second, each of which schedules a
    /// sync) must not hold the installed successor out of activation.
    #[dialog_common::test]
    async fn it_activates_a_successor_while_the_incumbent_page_is_busy(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_second_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        let query = tonk_worker::helpers::named_concept_wire_query();

        promote_second_generation(&env)?;
        let started = in_site(
            &driver,
            r#"
            const query = arguments[0];
            const done = arguments[arguments.length - 1];
            (async () => {
                const key = "tonk:test:successor-states";
                const record = state => {
                    const states = JSON.parse(sessionStorage.getItem(key) || "{}");
                    states[state] ??= Date.now();
                    sessionStorage.setItem(key, JSON.stringify(states));
                };
                const registration = await navigator.serviceWorker.getRegistration();
                const profiles = await (await fetch("/api/profiles")).json();
                const queryUrl = `/api/repository/profile:tonk/branch/${profiles.active}/query`;
                // The ordinary work of a live site. The loop ends when the
                // site loads again under the successor.
                (async () => {
                    for (;;) {
                        try { await (await fetch("/guest/manifest.json")).arrayBuffer(); } catch {}
                        try {
                            await (await fetch(queryUrl, {
                                method: "POST",
                                headers: { "content-type": "application/json" },
                                body: JSON.stringify(query),
                            })).text();
                        } catch {}
                        await new Promise(resolve => setTimeout(resolve, 250));
                    }
                })();
                registration.addEventListener("updatefound", () => {
                    const incoming = registration.installing;
                    const observe = () => record(incoming.state);
                    incoming.addEventListener("statechange", observe);
                    observe();
                }, { once: true });
                done({ ok: true });
            })().catch(error => done({ error: String(error) }));
            "#,
            vec![query],
        )
        .await?;
        ensure!(
            started["ok"] == true,
            "failed to start the busy site: {started}"
        );
        update_site_worker(&driver).await?;

        let read_states = r#"
            const done = arguments[arguments.length - 1];
            done(JSON.parse(sessionStorage.getItem("tonk:test:successor-states") || "{}"));
        "#;
        if let Err(error) = wait_for_site_generation(&driver, &generation_b).await {
            let states = in_site(&driver, read_states, vec![])
                .await
                .unwrap_or(Value::Null);
            return Err(error.context(format!("successor={states}")));
        }
        let states = in_site(&driver, read_states, vec![]).await?;
        let installed = states["installed"]
            .as_u64()
            .context(format!("successor never reported installed: {states}"))?;
        let activating = states["activating"]
            .as_u64()
            .context(format!("successor never reported activating: {states}"))?;
        let held = activating.saturating_sub(installed);
        assert!(
            held < 10_000,
            "the installed successor waited {held}ms for the busy incumbent to release it: {states}"
        );

        driver.quit().await?;
        Ok(())
    }

    /// A deploy that changed only worker and guest code keeps the open
    /// page: the profile's new worker takes over, the site loads again
    /// under it with the new guest runtime, and the page around it stays.
    #[dialog_common::test]
    async fn it_remounts_guests_without_reloading_when_the_page_is_unchanged(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_site_only_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;
        wait_for_guest_generation(&driver, "A").await?;
        let rendered = settled_guest_text(&driver).await?;
        let before = documents(&driver).await?;

        promote_second_generation(&env)?;
        update_site_worker(&driver).await?;

        wait_for_guest_generation(&driver, "B").await?;
        let state = wait_for_site_generation(&driver, &generation_b).await?;
        assert_eq!(
            documents(&driver).await?,
            before,
            "the page must not reload: {state}"
        );
        assert!(
            state["page"]["guard"].is_null(),
            "the page was not asked to load again: {state}"
        );
        // The site, loaded again, renders the same live data from the new worker.
        let again = settled_guest_text(&driver).await?;
        let differs = rendered
            .chars()
            .zip(again.chars())
            .position(|(before, after)| before != after)
            .unwrap_or_else(|| rendered.chars().count().min(again.chars().count()));
        let around = |text: &str| {
            text.chars()
                .skip(differs.saturating_sub(80))
                .take(240)
                .collect::<String>()
        };
        assert!(
            again == rendered,
            "the site renders something else under the new worker, from character {differs}:\n before: {}\n after:  {}",
            around(&rendered),
            around(&again)
        );

        driver.quit().await?;
        Ok(())
    }

    /// The join failures on the profile's active branch, as the controlling
    /// worker reads them. The failure lives only in that worker's session
    /// overlay.
    async fn join_failures(driver: &WebDriver) -> Result<Value> {
        in_site(
            driver,
            r#"
            const query = arguments[0];
            const done = arguments[arguments.length - 1];
            (async () => {
                const profiles = await (await fetch("/api/profiles")).json();
                const response = await fetch(`/api/repository/profile:tonk/branch/${profiles.active}/query`, {
                    method: "POST",
                    headers: { "content-type": "application/json" },
                    body: JSON.stringify(query),
                });
                done({ status: response.status, rows: await response.json() });
            })().catch(error => done({ error: String(error) }));
            "#,
            vec![tonk_worker::helpers::join_failure_wire_query()],
        )
        .await
    }

    async fn wait_for_join_failure(driver: &WebDriver) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let result = join_failures(driver).await.unwrap_or(Value::Null);
            if result["rows"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
            {
                return Ok(result);
            }
            ensure!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for a join failure: {result}"
            );
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// A worker's session overlay outlives the worker. A failed join leaves
    /// its failure only in the overlay, and nothing re-derives it when a
    /// worker boots, so the successor answers with it only if the handoff
    /// carried it across.
    #[dialog_common::test]
    async fn it_carries_the_session_overlay_to_a_successor_worker(
        env: TestEnvironment,
    ) -> Result<()> {
        let (generation_a, generation_b) = prepare_site_only_generation(&env)?;
        let driver = env.driver().await?;
        wait_for_complete_generation(&driver, &generation_a, None, None).await?;

        // Invite-shaped, so the join runs, but its `access` is not base58:
        // the join fails as malformed without touching the network.
        driver.enter_default_frame().await?;
        let origin = driver.current_url().await?;
        driver
            .goto(
                origin
                    .join("/join?access=not-a-delegation&remote=https%3A%2F%2Fexample.invalid#not-a-seed")?
                    .as_str(),
            )
            .await?;
        let failed = wait_for_join_failure(&driver).await?;
        // Leave /join, whose view would otherwise re-run the join.
        driver.goto(origin.join("/")?.as_str()).await?;
        wait_for_mounted_build(&driver, &generation_a.build).await?;
        let before = join_failures(&driver).await?;
        assert_eq!(before["rows"], failed["rows"], "{before}");

        promote_second_generation(&env)?;
        update_site_worker(&driver).await?;
        wait_for_guest_generation(&driver, "B").await?;
        wait_for_site_generation(&driver, &generation_b).await?;

        let after = join_failures(&driver).await?;
        assert_eq!(
            after["rows"], failed["rows"],
            "the successor restored the predecessor's overlay: {after}"
        );

        driver.quit().await?;
        Ok(())
    }

    /// With no network, the browser's look for a newer worker fails, and
    /// that is all that fails: the page loads from what its worker kept,
    /// and the profile's site comes up under the worker it had.
    #[dialog_common::test]
    async fn it_keeps_the_active_worker_when_the_load_time_update_check_is_offline(
        env: TestEnvironment,
    ) -> Result<()> {
        let driver = env.driver().await?;
        let build = worker_build_id(&env.service_worker_script)?;
        wait_for_mounted_build(&driver, &build).await?;
        let worker_a = wait_for_worker_started_at(&driver, None).await?;
        create_state_sentinels(&driver).await?;

        set_offline(&driver, true).await?;
        let test_result: Result<()> = async {
            driver.refresh().await?;
            let state = wait_for_mounted_worker(&driver, worker_a).await?;
            for side in [&state["page"], &state["site"]] {
                ensure!(side["controlled"] == true, "{state}");
                ensure!(side["active"] == "activated", "{state}");
                ensure!(side["installing"].is_null(), "{state}");
                ensure!(side["waiting"].is_null(), "{state}");
            }
            ensure!(state["page"]["guard"].is_null(), "{state}");

            let sentinels = state_sentinels(&driver).await?;
            ensure!(sentinels["indexedDb"] == "preserved", "{sentinels}");
            ensure!(sentinels["cache"] == "preserved", "{sentinels}");
            Ok(())
        }
        .await;

        let restore_result = set_offline(&driver, false).await;
        let quit_result = driver.quit().await;

        test_result?;
        restore_result?;
        quit_result?;
        Ok(())
    }
}
