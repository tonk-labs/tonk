//! Bounded browser transport coverage probes.
//!
//! This ignored native test serves synthetic, content-free resources from the
//! disposable integration harness. It records only categorical counts and never
//! uses a release candidate, user profile, or product service-worker scope.

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "integration-tests", feature = "web-integration-tests")
))]
mod tests {
    use crate::helpers::{TestEnvironment, TestServers};
    use anyhow::{Context, Result, anyhow, ensure};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const FIXTURE_PREFIX: &str = "__tonk_transport_probe__";
    const LOG_LIMIT: usize = 20_000;
    const COUNT_LIMIT: u64 = 8;

    const INDEX_TEMPLATE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>tonk transport root</title>
<body>
<script>
const primary = __PRIMARY__;
const alternate = __ALTERNATE__;
const expected = new Set(['root', 'dedicated_worker', 'service_worker', 'opaque_iframe', 'opaque_iframe_late', 'oopif']);
const observed = new Set();
document.documentElement.dataset.transportObserved = '';
const mark = category => {
  observed.add(category);
  document.documentElement.dataset.transportObserved = [...observed].sort().join(',');
  if ([...expected].every(value => observed.has(value))) {
    document.documentElement.dataset.transportReady = 'true';
  }
};
const fail = category => { document.documentElement.dataset.transportError = category; };
addEventListener('message', event => {
  const category = event.data?.tonkTransportProbe;
  const error = event.data?.tonkTransportProbeError;
  if (error === 'opaque_iframe_late' && event.origin === 'null') fail(error);
  if ((category === 'opaque_iframe' || category === 'opaque_iframe_late') && event.origin === 'null') mark(category);
  if (category === 'oopif' && event.origin === alternate) mark(category);
});

fetch('/__tonk_transport_probe__/root.txt', {cache: 'no-store'})
  .then(response => { if (!response.ok) throw new Error(); mark('root'); })
  .catch(() => fail('root'));

const worker = new Worker('/__tonk_transport_probe__/dedicated-worker.js');
globalThis.__tonkTransportWorker = worker;
worker.onmessage = event => {
  if (event.data?.tonkTransportProbe === 'dedicated_worker') mark('dedicated_worker');
};
worker.onerror = () => fail('dedicated_worker');

navigator.serviceWorker.register('/__tonk_transport_probe__/isolated-sw.js', {
  scope: '/__tonk_transport_probe__/isolated-scope/'
}).then(registration => {
  const candidate = registration.installing || registration.waiting || registration.active;
  if (!candidate) { fail('service_worker'); return; }
  if (candidate.state === 'activated') { mark('service_worker'); return; }
  candidate.addEventListener('statechange', () => {
    if (candidate.state === 'activated') mark('service_worker');
    if (candidate.state === 'redundant') fail('service_worker');
  });
}).catch(() => fail('service_worker'));

const opaque = document.createElement('iframe');
opaque.setAttribute('sandbox', 'allow-scripts');
opaque.srcdoc = '<!doctype html><title>tonk transport opaque</title><script src="' +
  primary + '/__tonk_transport_probe__/opaque.js"><\/script>';
document.body.append(opaque);

const oopif = document.createElement('iframe');
oopif.src = alternate + '/__tonk_transport_probe__/oopif.html';
document.body.append(oopif);
</script>
</body>
"#;

    const DEDICATED_WORKER: &str = r#"fetch('/__tonk_transport_probe__/dedicated.txt', {cache: 'no-store'})
  .then(response => { if (!response.ok) throw new Error(); postMessage({tonkTransportProbe: 'dedicated_worker'}); });
"#;

    const ISOLATED_SERVICE_WORKER: &str = r#"addEventListener('install', event => {
  event.waitUntil(fetch('/__tonk_transport_probe__/service-worker.txt', {cache: 'no-store'})
    .then(response => { if (!response.ok) throw new Error(); }));
});
addEventListener('activate', event => event.waitUntil(clients.claim()));
"#;

    const OPAQUE_SCRIPT: &str = r#"parent.postMessage({tonkTransportProbe: 'opaque_iframe'}, '*');
setTimeout(() => {
  const script = document.createElement('script');
  script.src = '/__tonk_transport_probe__/opaque-late.js';
  script.onerror = () => parent.postMessage({tonkTransportProbeError: 'opaque_iframe_late'}, '*');
  document.head.append(script);
}, 250);
"#;

    const OPAQUE_LATE_SCRIPT: &str =
        "parent.postMessage({tonkTransportProbe: 'opaque_iframe_late'}, '*');\n";

    const OOPIF_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8"><title>tonk transport oopif</title>
<script>
fetch('/__tonk_transport_probe__/oopif.txt', {cache: 'no-store'})
  .then(response => { if (!response.ok) throw new Error(); parent.postMessage({tonkTransportProbe: 'oopif'}, '*'); });
</script>
"#;

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum Category {
        Root,
        DedicatedWorker,
        ServiceWorker,
        OpaqueIframe,
        Oopif,
    }

    impl Category {
        const ALL: [Self; 5] = [
            Self::Root,
            Self::DedicatedWorker,
            Self::ServiceWorker,
            Self::OpaqueIframe,
            Self::Oopif,
        ];

        fn label(self) -> &'static str {
            match self {
                Self::Root => "root",
                Self::DedicatedWorker => "dedicated_worker",
                Self::ServiceWorker => "service_worker",
                Self::OpaqueIframe => "opaque_iframe",
                Self::Oopif => "cross_origin_iframe",
            }
        }
    }

    struct CollectorChild(Child);

    impl CollectorChild {
        fn wait_ready(&mut self, ready: &Path) -> Result<()> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if ready.is_file() {
                    return Ok(());
                }
                if let Some(status) = self.0.try_wait()? {
                    return Err(anyhow!(
                        "direct CDP collector exited before readiness: {status}"
                    ));
                }
                ensure!(
                    Instant::now() < deadline,
                    "direct CDP collector readiness timed out"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        }

        fn finish(&mut self, stop: &Path, output: &Path) -> Result<Value> {
            std::fs::write(stop, b"stop\n")?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = self.0.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    self.0.kill()?;
                    self.0.wait()?;
                    return Err(anyhow!("direct CDP collector shutdown timed out"));
                }
                std::thread::sleep(Duration::from_millis(25));
            };
            ensure!(
                matches!(status.code(), Some(0 | 2)),
                "direct CDP collector failed: {status}"
            );
            let receipt: Value = serde_json::from_slice(&std::fs::read(output)?)?;
            Ok(receipt)
        }
    }

    impl Drop for CollectorChild {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    async fn debugger_address(env: &TestEnvironment) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut candidates = Vec::new();
            for entry in std::fs::read_dir(&env.browser_profile_root)? {
                let path = entry?.path().join("DevToolsActivePort");
                let Ok(contents) = std::fs::read_to_string(path) else {
                    continue;
                };
                let mut lines = contents.lines();
                let Some(port) = lines.next().and_then(|line| line.parse::<u16>().ok()) else {
                    continue;
                };
                let Some(browser_path) = lines.next() else {
                    continue;
                };
                if !browser_path.starts_with("/devtools/browser/") {
                    continue;
                }
                let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
                if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
                    candidates.push(port);
                }
            }
            candidates.sort_unstable();
            candidates.dedup();
            ensure!(
                candidates.len() <= 1,
                "multiple live debugger endpoints under this test workspace"
            );
            if let Some(port) = candidates.first() {
                return Ok(format!("127.0.0.1:{port}"));
            }
            ensure!(
                Instant::now() < deadline,
                "Chrome debugger endpoint did not appear in the test workspace"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn start_collector(
        env: &TestEnvironment,
        debugger_address: &str,
    ) -> Result<(
        CollectorChild,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    )> {
        let receipt = env.deployment_root.join("transport-cdp-receipt.json");
        let ready = env.deployment_root.join("transport-cdp.ready");
        let stop = env.deployment_root.join("transport-cdp.stop");
        ensure!(
            [&receipt, &ready, &stop]
                .into_iter()
                .all(|path| !path.exists()),
            "direct CDP collector paths must be new"
        );
        let manifest = std::env::var("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_owned());
        let script = Path::new(&manifest)
            .parent()
            .and_then(Path::parent)
            .context("tonk-ui manifest has no repository root")?
            .join("scripts/perf/cdp_transport.py");
        ensure!(
            script.is_file(),
            "direct CDP collector source is unavailable"
        );
        let attach_mode =
            std::env::var("TONK_PERF_CDP_ATTACH_MODE").unwrap_or_else(|_| "recursive".to_owned());
        ensure!(
            matches!(attach_mode.as_str(), "recursive" | "auto-attach-related"),
            "invalid direct CDP attach mode"
        );
        let child = Command::new("python3")
            .arg(script)
            .args(["--debugger-address", debugger_address])
            .args(["--attach-mode", attach_mode.as_str()])
            .arg("--output")
            .arg(&receipt)
            .arg("--ready-file")
            .arg(&ready)
            .arg("--stop-file")
            .arg(&stop)
            .args(["--timeout-seconds", "60"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        Ok((CollectorChild(child), receipt, ready, stop))
    }

    fn validate_direct_receipt(receipt: &Value) -> Result<()> {
        ensure!(
            receipt["schema_version"] == 1,
            "unexpected direct CDP receipt schema"
        );
        ensure!(
            receipt["transport"] == "direct-browser-cdp-flattened-autoattach",
            "unexpected direct CDP transport"
        );
        for category in Category::ALL {
            let label = category.label();
            let requests = receipt["request_counts"][label]
                .as_u64()
                .filter(|count| *count > 0)
                .with_context(|| format!("direct CDP missed {label} request"))?;
            ensure!(
                receipt["finished_counts"][label].as_u64() == Some(requests),
                "direct CDP did not complete every {label} request"
            );
            ensure!(
                receipt["failed_counts"][label].as_u64() == Some(0),
                "direct CDP recorded {label} failure"
            );
            ensure!(
                receipt["pending_counts"][label].as_u64() == Some(0),
                "direct CDP left {label} request pending"
            );
            ensure!(
                receipt["encoded_bytes"][label]
                    .as_f64()
                    .is_some_and(|bytes| bytes > 0.0),
                "direct CDP recorded no {label} bytes"
            );
        }
        let late = "opaque_iframe_late";
        let late_requests = receipt["request_counts"][late]
            .as_u64()
            .filter(|count| *count > 0)
            .context("direct CDP missed delayed opaque-frame request")?;
        ensure!(
            receipt["finished_counts"][late].as_u64() == Some(late_requests)
                && receipt["failed_counts"][late].as_u64() == Some(0)
                && receipt["pending_counts"][late].as_u64() == Some(0)
                && receipt["encoded_bytes"][late]
                    .as_f64()
                    .is_some_and(|bytes| bytes > 0.0),
            "direct CDP did not complete the delayed opaque-frame request"
        );
        ensure!(
            receipt["coverage"]["all_categories_complete"] == true,
            "direct CDP coverage is incomplete"
        );
        Ok(())
    }

    fn write_fixture(root: &Path, primary: &str, alternate: &str) -> Result<()> {
        let fixture = root.join(FIXTURE_PREFIX);
        std::fs::create_dir(&fixture)?;
        let index = INDEX_TEMPLATE
            .replace("__PRIMARY__", &json!(primary).to_string())
            .replace("__ALTERNATE__", &json!(alternate).to_string());
        for (name, contents) in [
            ("index.html", index.as_str()),
            ("root.txt", "root\n"),
            ("dedicated-worker.js", DEDICATED_WORKER),
            ("dedicated.txt", "dedicated\n"),
            ("isolated-sw.js", ISOLATED_SERVICE_WORKER),
            ("service-worker.txt", "service worker\n"),
            ("opaque.js", OPAQUE_SCRIPT),
            ("opaque-late.js", OPAQUE_LATE_SCRIPT),
            ("oopif.html", OOPIF_PAGE),
            ("oopif.txt", "oopif\n"),
        ] {
            std::fs::write(fixture.join(name), contents)?;
        }
        std::fs::create_dir(fixture.join("isolated-scope"))?;
        Ok(())
    }

    fn classify(url: &str) -> Option<Category> {
        let path = url::Url::parse(url).ok()?.path().to_owned();
        match path.as_str() {
            "/__tonk_transport_probe__/root.txt" => Some(Category::Root),
            "/__tonk_transport_probe__/dedicated.txt" => Some(Category::DedicatedWorker),
            "/__tonk_transport_probe__/service-worker.txt" => Some(Category::ServiceWorker),
            "/__tonk_transport_probe__/opaque.js" => Some(Category::OpaqueIframe),
            "/__tonk_transport_probe__/oopif.txt" => Some(Category::Oopif),
            _ => None,
        }
    }

    fn summarize_requests(entries: &[Value]) -> Result<BTreeMap<Category, u64>> {
        ensure!(
            entries.len() <= LOG_LIMIT,
            "performance log exceeded the bounded probe limit"
        );
        let mut counts = BTreeMap::new();
        for entry in entries {
            let Some(raw) = entry["message"].as_str() else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(raw) else {
                continue;
            };
            let event = &message["message"];
            if event["method"] != "Network.requestWillBeSent" {
                continue;
            }
            let Some(category) = event["params"]["request"]["url"]
                .as_str()
                .and_then(classify)
            else {
                continue;
            };
            let count = counts.entry(category).or_insert(0_u64);
            *count += 1;
            ensure!(
                *count <= COUNT_LIMIT,
                "{} request count exceeded the bounded probe limit",
                category.label()
            );
        }
        Ok(counts)
    }

    fn target_counts(targets: &Value) -> (u64, u64) {
        let mut oopif = 0;
        let mut opaque = 0;
        for target in targets["targetInfos"].as_array().into_iter().flatten() {
            if target["type"] != "iframe" {
                continue;
            }
            let title = target["title"].as_str().unwrap_or_default();
            let path = target["url"]
                .as_str()
                .and_then(|url| url::Url::parse(url).ok())
                .map(|url| url.path().to_owned());
            if path.as_deref() == Some("/__tonk_transport_probe__/oopif.html") {
                oopif += 1;
            }
            if title == "tonk transport opaque" {
                opaque += 1;
            }
        }
        (oopif, opaque)
    }

    fn categorical_report(counts: &BTreeMap<Category, u64>, oopif: u64, opaque: u64) -> Value {
        let requests = Category::ALL
            .into_iter()
            .map(|category| {
                (
                    category.label().to_owned(),
                    json!(counts.get(&category).copied().unwrap_or(0)),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        json!({
            "request_counts": requests,
            "target_counts": {"cross_origin_oopif": oopif, "opaque_iframe": opaque},
            "topology": {
                "cross_origin_oopif_confirmed": oopif > 0,
                "opaque_iframe_same_process_inferred": opaque == 0
            },
            "limits": {"log_entries": LOG_LIMIT, "requests_per_category": COUNT_LIMIT}
        })
    }

    #[test]
    fn request_summary_is_bounded_and_categorical() -> Result<()> {
        let entry = |name: &str| {
            json!({"message": json!({"message": {
                "method": "Network.requestWillBeSent",
                "params": {"request": {"url": format!(
                    "https://fixture.invalid/{FIXTURE_PREFIX}/{name}"
                )}}
            }}).to_string()})
        };
        let entries = [
            entry("root.txt"),
            entry("dedicated.txt"),
            entry("service-worker.txt"),
            entry("opaque.js"),
            entry("oopif.txt"),
            entry("unrelated-private-value"),
        ];
        let counts = summarize_requests(&entries)?;
        assert!(
            Category::ALL
                .into_iter()
                .all(|category| counts.get(&category) == Some(&1))
        );
        let report = categorical_report(&counts, 1, 0).to_string();
        assert!(!report.contains("https://"));
        assert!(!report.contains("unrelated-private-value"));
        Ok(())
    }

    #[test]
    fn target_summary_distinguishes_oopif_from_opaque_srcdoc() {
        let targets = json!({"targetInfos": [
            {"type": "iframe", "title": "tonk transport oopif",
             "url": "https://fixture.invalid/__tonk_transport_probe__/oopif.html"},
            {"type": "page", "title": "tonk transport opaque", "url": "about:srcdoc"}
        ]});
        assert_eq!(target_counts(&targets), (1, 0));
    }

    #[tokio::test]
    #[ignore = "requires isolated Chrome and the disposable native test servers"]
    async fn it_probes_direct_cdp_transport_coverage() -> Result<()> {
        ensure!(
            std::env::var("TONK_TEST_BROWSER").as_deref() != Ok("safari"),
            "Chrome remote debugging is required"
        );

        let (servers, env) = TestServers::start().await?;
        let driver = match env.blank_driver().await {
            Ok(driver) => driver,
            Err(error) => {
                servers.stop().await?;
                return Err(error);
            }
        };
        let tested: Result<Value> = async {
            let debugger_address = debugger_address(&env).await?;
            let (mut collector, receipt_path, ready_path, stop_path) =
                start_collector(&env, &debugger_address)?;
            collector.wait_ready(&ready_path)?;
            let primary = env.tonk_web.origin().ascii_serialization();
            let mut alternate = env.tonk_web.clone();
            let other_host = if alternate.host_str() == Some("localhost") {
                "tonk.network"
            } else {
                "localhost"
            };
            alternate
                .set_host(Some(other_host))
                .map_err(|_| anyhow!("could not construct alternate fixture origin"))?;
            let alternate = alternate.origin().ascii_serialization();
            write_fixture(
                &env.deployment_root.join("generation-a"),
                &primary,
                &alternate,
            )?;

            let page = format!("{primary}/{FIXTURE_PREFIX}/index.html");
            driver.goto(page).await?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let status = driver
                    .execute(
                        "return {ready: document.documentElement.dataset.transportReady === 'true', error: document.documentElement.dataset.transportError || null, observed: document.documentElement.dataset.transportObserved || ''}",
                        vec![],
                    )
                    .await?
                    .json()
                    .clone();
                ensure!(status["error"].is_null(), "fixture realm failed: {}", status["error"]);
                if status["ready"] == true {
                    break;
                }
                if Instant::now() >= deadline {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let observed = status["observed"].as_str().unwrap_or("invalid");
                    return match collector.finish(&stop_path, &receipt_path) {
                        Ok(receipt) => {
                            eprintln!("TONK DIRECT CDP INCOMPLETE: {receipt}");
                            Err(anyhow!(
                                "fixture realms did not become ready; observed={observed}"
                            ))
                        }
                        Err(error) => Err(anyhow!(
                            "fixture realms did not become ready; observed={observed}; collector={error}"
                        )),
                    };
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            let receipt = collector.finish(&stop_path, &receipt_path)?;
            eprintln!("TONK DIRECT CDP COVERAGE: {receipt}");
            validate_direct_receipt(&receipt)?;
            Ok(receipt)
        }
        .await;
        let quit = driver.quit().await;
        let stop = servers.stop().await;
        quit?;
        stop?;
        tested?;
        Ok(())
    }
}
