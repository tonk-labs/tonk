use serde::{Deserialize, Serialize};
use url::Url;

/// Test environment configuration for integration tests.
/// Available on all platforms, but can only be constructed on native.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct TestEnvironment {
    /// URL of the Tonk web server (Caddy proxies /ucan/* to the access service).
    pub tonk_web: Url,
    /// URL of the ChromeDriver server.
    pub chromedriver: Url,
    /// Base URL of the live native access service, reached directly
    /// (unproxied) for test inspection such as captured activation emails.
    pub access_service: Url,
    /// This harness's Caddy root certificate, for CLI children that must
    /// trust its origin. Per-harness rather than process-wide: each run
    /// mints its own CA, so a single `SSL_CERT_FILE` would leave
    /// concurrent runs trusting whichever one started last.
    pub ca_certificate: Option<std::path::PathBuf>,
    /// Writable per-harness A/B deployment fixture. `current` is the atomic
    /// symlink Caddy serves; generation directories contain complete artifacts.
    pub deployment_root: std::path::PathBuf,
    /// Writable service worker in the initial complete generation.
    pub service_worker_script: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::TestEnvironment;
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use dialog_common::helpers::{Provider, Service};
    use port_check::free_local_port;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};
    #[cfg(test)]
    use thirtyfour::extensions::cdp::ChromeDevTools;
    use thirtyfour::{
        CapabilitiesHelper, ChromiumLikeCapabilities, DesiredCapabilities, WebDriver,
    };
    use tonk_access_service::helpers::{AccessServiceAddress, AccessServiceSettings};
    use tonk_worker_api::DeploymentConfig;
    use url::Url;

    /// Reaps a spawned test dependency on every success and failure path.
    struct ManagedChild(Option<Child>);

    impl ManagedChild {
        fn new(child: Child) -> Self {
            Self(Some(child))
        }

        fn child_mut(&mut self) -> &mut Child {
            self.0.as_mut().expect("managed child is present")
        }

        fn terminate(&mut self) -> std::io::Result<()> {
            let Some(mut child) = self.0.take() else {
                return Ok(());
            };
            if child.try_wait()?.is_none() {
                child.kill()?;
                child.wait()?;
            }
            Ok(())
        }
    }

    impl Drop for ManagedChild {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }

    fn copy_artifact_tree(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                copy_artifact_tree(&source_path, &destination_path)?;
            } else if file_type.is_file() {
                std::fs::copy(&source_path, &destination_path)?;
                let mut permissions = std::fs::metadata(&destination_path)?.permissions();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    permissions.set_mode(permissions.mode() | 0o200);
                }
                #[cfg(not(unix))]
                permissions.set_readonly(false);
                std::fs::set_permissions(&destination_path, permissions)?;
            } else {
                return Err(anyhow!(
                    "unsupported test artifact member {}",
                    source_path.display()
                ));
            }
        }
        Ok(())
    }

    impl TestEnvironment {
        /// Creates a new WebDriver instance connected to the test environment.
        pub async fn driver(&self) -> Result<WebDriver> {
            let mut caps = DesiredCapabilities::chrome();
            // NOTE: Discovered arcana while reverse engineering
            // wasm-bindgen-test-runner. TL;DR Chrome will crash when running as
            // root in GHA runners unless you launch with certain flags
            // SEE: https://stackoverflow.com/a/50642913
            caps.add_arg("--disable-dev-shm-usage")?;
            caps.add_arg("--no-sandbox")?;
            if std::env::var("NO_HEADLESS").ok().is_none() {
                caps.set_headless()?;
            }

            caps.add_arg("--host-resolver-rules=MAP tonk.network 127.0.0.1")?;
            caps.accept_insecure_certs(true)?;
            let secure_origin = format!(
                "--unsafely-treat-insecure-origin-as-secure={}",
                self.tonk_web.origin().ascii_serialization()
            );
            caps.add_arg(&secure_origin)?;

            if let Ok(chrome_binary) = std::env::var("CHROME") {
                caps.set_binary(&chrome_binary)?;
            }

            // Diagnostic tap, off unless asked for: makes chromedriver
            // retain every console message (page and service worker
            // alike) for the classic /se/log endpoint, so a wedged async
            // flow can be read from the test log instead of guessed at.
            if std::env::var("TONK_E2E_CHROME_LOG").is_ok() {
                caps.insert_base_capability(
                    "goog:loggingPrefs".to_string(),
                    serde_json::json!({ "browser": "ALL" }),
                );
            }

            let driver = WebDriver::new(&self.chromedriver.to_string(), caps).await?;
            // Bound each navigation well under the suite's patience. The
            // default page-load allowance is five minutes, so one wedged
            // renderer would eat the whole run before `goto` below ever
            // gets its second chance.
            driver
                .set_page_load_timeout(std::time::Duration::from_secs(60))
                .await?;
            #[cfg(test)]
            {
                // Install before the first navigation so failures from the
                // eager service-worker registration are still available when
                // an integration wait times out. The user-facing boot surface
                // deliberately hides technical diagnostics.
                let devtools = ChromeDevTools::new(driver.handle.clone());
                devtools
                    .execute_cdp_with_params(
                        "Page.addScriptToEvaluateOnNewDocument",
                        serde_json::json!({
                            "source": r#"
                                globalThis.__tonkTestErrors = [];
                                const describe = value => {
                                    if (value instanceof Error) return `${value.name}: ${value.message}`;
                                    if (typeof value === "string") return value;
                                    try { return JSON.stringify(value); } catch { return String(value); }
                                };
                                const priorError = console.error.bind(console);
                                console.error = (...values) => {
                                    globalThis.__tonkTestErrors.push(values.map(describe).join(" "));
                                    priorError(...values);
                                };
                                addEventListener("error", event => {
                                    globalThis.__tonkTestErrors.push(`error: ${describe(event.error || event.message)}`);
                                });
                                addEventListener("unhandledrejection", event => {
                                    globalThis.__tonkTestErrors.push(`unhandledrejection: ${describe(event.reason)}`);
                                });
                            "#,
                        }),
                    )
                    .await?;
            }
            goto(&driver, &self.tonk_web.to_string()).await?;
            Ok(driver)
        }
    }

    fn retryable_navigation_error(error: &thirtyfour::error::WebDriverErrorInner) -> bool {
        use thirtyfour::error::WebDriverErrorInner;

        match error {
            WebDriverErrorInner::WebDriverTimeout(_) | WebDriverErrorInner::Timeout(_) => true,
            WebDriverErrorInner::UnknownError(info) => {
                info.value.message.contains("net::ERR_SSL_PROTOCOL_ERROR")
            }
            _ => false,
        }
    }

    /// Navigates, retrying once when the renderer wedges mid-load or the
    /// per-test Caddy server is still finishing its first TLS handshake.
    ///
    /// A navigation whose renderer stops responding surfaces as
    /// chromedriver's 'timed out receiving message from renderer' after the
    /// page-load allowance. The page's own boot watchdog cannot act there —
    /// a hung renderer runs no scripts — so the recovery lives on this side
    /// of the DevTools pipe: one fresh navigation to the same URL, the same
    /// restart a person's reload performs. Caddy can also accept TCP while it
    /// is still minting the origin certificate; Chrome reports that short
    /// startup window as `net::ERR_SSL_PROTOCOL_ERROR`, so the same one-shot
    /// retry crosses that readiness boundary without hiding other failures.
    pub async fn goto(driver: &WebDriver, url: impl AsRef<str>) -> Result<()> {
        let url = url.as_ref();
        match driver.goto(url).await {
            Err(error) if retryable_navigation_error(error.as_inner()) => {
                eprintln!("navigation to {url} failed transiently ({error}); retrying once");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                Ok(driver.goto(url).await?)
            }
            other => Ok(other?),
        }
    }

    /// Creates a WebDriver with a PRF-capable virtual authenticator.
    #[cfg(test)]
    pub(crate) async fn driver_with_prf(env: &TestEnvironment) -> Result<WebDriver> {
        Ok(driver_with_prf_authenticator(env).await?.0)
    }

    /// Creates a WebDriver and returns the PRF-capable virtual authenticator's
    /// id so tests can inspect credential side effects.
    #[cfg(test)]
    pub(crate) async fn driver_with_prf_authenticator(
        env: &TestEnvironment,
    ) -> Result<(WebDriver, String)> {
        let driver = env.driver().await?;
        let devtools = ChromeDevTools::new(driver.handle.clone());
        devtools.execute_cdp("WebAuthn.enable").await?;
        let authenticator = devtools
            .execute_cdp_with_params(
                "WebAuthn.addVirtualAuthenticator",
                serde_json::json!({
                    "options": {
                        "protocol": "ctap2",
                        "ctap2Version": "ctap2_1",
                        "transport": "internal",
                        "hasResidentKey": true,
                        "hasUserVerification": true,
                        "isUserVerified": true,
                        "hasPrf": true,
                        "automaticPresenceSimulation": true,
                    }
                }),
            )
            .await?;
        let authenticator_id = authenticator["authenticatorId"]
            .as_str()
            .ok_or_else(|| anyhow!("Chrome omitted the virtual authenticator id"))?
            .to_string();
        // Polled from the test side: a single waiting script is bounded
        // by chromedriver's script timeout, which a cold machine still
        // compiling the app's wasm can outlast. A boot that WEDGES
        // rather than runs slow is the page's own problem now — its
        // watchdog (index.html) reloads a boot with no signs of life
        // and escalates to clearing caches and workers — so this wait
        // only has to outlast the ladder, not run it.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(150);
        loop {
            let ready = driver
                .execute("return !!window.tonkIdentity;", vec![])
                .await
                .ok()
                .and_then(|ret| ret.json().as_bool());
            if ready == Some(true) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                // Say where boot stopped, not just that it did: the
                // shell's status line distinguishes a wasm that never
                // downloaded from one that failed from one that started
                // and hung.
                let state = driver
                    .execute(
                        r#"return {
                            url: String(location.href),
                            ready: document.readyState,
                            boot: (document.querySelector("[data-boot-status]") || {}).textContent || null,
                            controlled: !!(navigator.serviceWorker && navigator.serviceWorker.controller),
                        };"#,
                        vec![],
                    )
                    .await
                    .map(|ret| ret.json().clone())
                    .unwrap_or(serde_json::Value::Null);
                return Err(anyhow!(
                    "the page never exposed tonkIdentity; page state: {state}"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok((driver, authenticator_id))
    }

    /// Manages test server processes for integration testing.
    pub struct TestServers {
        web_server: ManagedChild,
        chromedriver: ManagedChild,
        access_service:
            Option<Service<AccessServiceAddress, tonk_access_service::helpers::AccessServer>>,
    }

    impl TestServers {
        /// Starts the test servers and returns the server handles and environment configuration.
        ///
        /// Startup order:
        /// 1. Start the access service with deployment discovery configured
        /// 2. Start Caddy web server with access service port
        /// 3. Start ChromeDriver
        pub async fn start() -> Result<(Self, TestEnvironment)> {
            // Chosen before the access service starts: activation links
            // must open on the page origin Caddy will serve, not on the
            // access service's own port.
            let web_port =
                free_local_port().expect("Could not get a free local port for test server");
            let settings = AccessServiceSettings {
                // The identity is filled in by the server itself.
                deployment: Some(DeploymentConfig::default()),
                public_origin: Some(format!("https://tonk.network:{web_port}")),
                ..Default::default()
            };
            let access_service = tonk_access_service::helpers::access_service(settings).await?;
            let access_service_address = access_service.address.clone();

            // Extract port from access service URL (e.g., "http://127.0.0.1:8090" -> "8090")
            let access_service_port = Url::parse(&access_service_address.access_service_url)?
                .port()
                .ok_or_else(|| anyhow!("Access service URL has no port"))?;
            let caddy_data = std::env::temp_dir().join(format!("tonk-e2e-caddy-{web_port}"));
            std::fs::create_dir_all(&caddy_data)?;
            // The runner's variable wins over the compile-time constant:
            // a binary out of the `tests-e2e` archive was compiled in the
            // Nix sandbox, whose source path no longer exists, while
            // `cargo nextest run --workspace-remap` points the runtime
            // variable at the live checkout.
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
                .unwrap_or_else(|_| env!("CARGO_MANIFEST_DIR").to_string());
            let workspace = std::path::Path::new(&manifest_dir)
                .parent()
                .and_then(std::path::Path::parent)
                .ok_or_else(|| anyhow!("tonk-ui manifest has no workspace root"))?;
            let deployment_root = caddy_data.join("deployments");
            std::fs::create_dir_all(&deployment_root)?;
            let service_worker_script = deployment_root
                .join("generation-a")
                .join("service_worker.js");
            let mut test_server = if let Some(test_server) = std::env::var_os("TONK_UI_TEST_SERVER")
            {
                std::process::Command::new(test_server)
            } else {
                // Use the Git worktree view so ignored build products (`target`,
                // linked worktrees, benchmark runs) are not copied into the Nix
                // store every time an integration test starts its web server.
                // Newly added test source must be staged, just as it must be for
                // the committed CI revision that ultimately runs this harness.
                let test_server = format!("git+file:{}#tonk-ui-test-server", workspace.display());
                let mut command = std::process::Command::new("nix");
                command.args(["run", &test_server, "--"]);
                command
            };
            test_server.args([
                &format!("{web_port}"),
                &format!("{access_service_port}"),
                deployment_root
                    .to_str()
                    .ok_or_else(|| anyhow!("service-worker root is not valid UTF-8"))?,
            ]);
            let mut web_server = ManagedChild::new(
                test_server
                    // Pin Caddy's data dir so its per-run internal CA
                    // root lands at a knowable path: it rides on
                    // `TestEnvironment::ca_certificate`, and each native
                    // CLI child is given it as its own SSL_CERT_FILE so
                    // it can speak TLS to the harness origin the
                    // descriptors name.
                    .env("XDG_DATA_HOME", &caddy_data)
                    .stdout(Stdio::piped())
                    // Nix writes build progress to stderr. Inheriting it prevents a
                    // full unread pipe from deadlocking before Caddy starts.
                    .stderr(Stdio::inherit())
                    .spawn()?,
            );

            let stdout = web_server
                .child_mut()
                .stdout
                .take()
                .ok_or_else(|| anyhow!("Failed to capture stdout"))?;

            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = line?;
                if line.contains("Test server live at") {
                    break;
                }
            }
            if let Some(artifact) = std::env::var_os("TONK_UI_TEST_ARTIFACT") {
                let artifact = std::path::PathBuf::from(artifact);
                let generation_a = deployment_root.join("generation-a");
                std::fs::remove_dir_all(&generation_a)?;
                copy_artifact_tree(&artifact, &generation_a)?;
            }
            let mut listening = false;
            for _ in 0..100 {
                if tokio::net::TcpStream::connect(("127.0.0.1", web_port))
                    .await
                    .is_ok()
                {
                    listening = true;
                    break;
                }
                if let Some(status) = web_server.child_mut().try_wait()? {
                    return Err(anyhow!("test web server exited before binding: {status}"));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if !listening {
                return Err(anyhow!("test web server did not bind port {web_port}"));
            }

            // Caddy mints its internal CA lazily; wait for the root and
            // export it process-wide so every CLI child the tests spawn
            // trusts the harness origin (reqwest is built with
            // rustls-tls-native-roots, which honors SSL_CERT_FILE). The
            // name half of the mapping — tonk.network resolving to
            // loopback for NATIVE processes — comes from /etc/hosts,
            // which the CI workflow writes; Chrome gets it via
            // --host-resolver-rules either way.
            let caddy_root = caddy_data
                .join("caddy")
                .join("pki")
                .join("authorities")
                .join("local")
                .join("root.crt");
            // Caddy mints the CA lazily, on its first TLS handshake, so
            // this can take a moment on a cold machine.
            for _ in 0..200 {
                if caddy_root.exists() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if !caddy_root.exists() {
                return Err(anyhow!(
                    "Caddy never minted its root certificate at {}; CLI children could not                      trust the harness origin",
                    caddy_root.display()
                ));
            }
            // Handed to CLI children individually rather than exported:
            // each harness mints its own CA under its own port, so a
            // process-wide `SSL_CERT_FILE` makes concurrent runs trust
            // the wrong root and fail to connect.
            let ca_certificate = Some(caddy_root);

            // Start ChromeDriver
            let chromedriver_port =
                free_local_port().expect("Could not get a free local port for chromedriver");
            let chromedriver_binary =
                std::env::var("CHROMEDRIVER").unwrap_or_else(|_| "chromedriver".to_string());
            let mut chromedriver = ManagedChild::new(
                std::process::Command::new(chromedriver_binary)
                    .args([&format!("--port={chromedriver_port}")])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .spawn()?,
            );

            let stdout = chromedriver
                .child_mut()
                .stdout
                .take()
                .ok_or_else(|| anyhow!("Failed to capture chromedriver stdout"))?;

            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = line?;
                if line.contains("ChromeDriver was started successfully") {
                    break;
                }
            }

            Ok((
                Self {
                    web_server,
                    chromedriver,
                    access_service: Some(access_service),
                },
                TestEnvironment {
                    tonk_web: Url::parse(&format!("https://tonk.network:{web_port}"))?,
                    chromedriver: Url::parse(&format!("http://127.0.0.1:{chromedriver_port}"))?,
                    access_service: Url::parse(&access_service_address.access_service_url)?,
                    ca_certificate,
                    deployment_root,
                    service_worker_script,
                },
            ))
        }

        /// Stops all test server processes.
        pub async fn stop(mut self) -> Result<()> {
            let web_result = self.web_server.terminate();
            let chromedriver_result = self.chromedriver.terminate();
            let access_result = if let Some(access_service) = self.access_service.take() {
                access_service.stop().await
            } else {
                Ok(())
            };
            web_result?;
            chromedriver_result?;
            access_result?;
            Ok(())
        }
    }

    impl Drop for TestServers {
        fn drop(&mut self) {
            let _ = self.web_server.terminate();
            let _ = self.chromedriver.terminate();
            // Dropping providers closes their shutdown senders; explicit
            // success paths still await orderly shutdown in `stop`.
            self.access_service.take();
        }
    }

    #[async_trait]
    impl Provider for TestServers {
        async fn stop(self) -> anyhow::Result<()> {
            TestServers::stop(self).await
        }
    }

    #[dialog_common::provider]
    async fn test_servers(_: ()) -> Result<Service<TestEnvironment, TestServers>> {
        let (server, address) = TestServers::start().await?;
        Ok(Service::new(address, server))
    }

    #[cfg(test)]
    mod tests {
        use super::retryable_navigation_error;
        use thirtyfour::error::{WebDriverErrorInfo, WebDriverErrorInner, WebDriverErrorValue};

        fn unknown_navigation_error(message: &str) -> WebDriverErrorInner {
            WebDriverErrorInner::UnknownError(WebDriverErrorInfo {
                status: 500,
                error: "unknown error".to_string(),
                value: WebDriverErrorValue::new(message.to_string()),
            })
        }

        #[test]
        fn it_retries_the_tls_handshake_race_only() {
            assert!(retryable_navigation_error(&unknown_navigation_error(
                "unknown error: net::ERR_SSL_PROTOCOL_ERROR"
            )));
            assert!(!retryable_navigation_error(&unknown_navigation_error(
                "unknown error: net::ERR_CONNECTION_REFUSED"
            )));
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
