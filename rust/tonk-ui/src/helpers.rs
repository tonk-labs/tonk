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
    use thirtyfour::{ChromiumLikeCapabilities, DesiredCapabilities, WebDriver};
    use tonk_access_service::helpers::{AccessServiceAddress, AccessServiceSettings};
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

            caps.add_arg("--host-resolver-rules=MAP tonk.spot 127.0.0.1")?;
            let secure_origin = format!(
                "--unsafely-treat-insecure-origin-as-secure={}",
                self.tonk_web.origin().ascii_serialization()
            );
            caps.add_arg(&secure_origin)?;

            if let Ok(chrome_binary) = std::env::var("CHROME") {
                caps.set_binary(&chrome_binary)?;
            }

            let driver = WebDriver::new(&self.chromedriver.to_string(), caps).await?;
            driver.goto(&self.tonk_web.to_string()).await?;
            Ok(driver)
        }
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
        /// 1. Start access service first to get its port
        /// 2. Start Caddy web server with access service port (proxies /ucan/*)
        /// 3. Start ChromeDriver
        pub async fn start() -> Result<(Self, TestEnvironment)> {
            // Start the access service first to get its port
            let settings = AccessServiceSettings::default();
            let access_service = tonk_access_service::helpers::access_service(settings).await?;
            let access_service_address = access_service.address.clone();

            // Extract port from access service URL (e.g., "http://127.0.0.1:8090" -> "8090")
            let access_service_port = Url::parse(&access_service_address.access_service_url)?
                .port()
                .ok_or_else(|| anyhow!("Access service URL has no port"))?;

            // Start the web server (Caddy) with access service port for /ucan/* proxying
            let web_port =
                free_local_port().expect("Could not get a free local port for test server");
            let mut web_server = ManagedChild::new(
                std::process::Command::new("nix")
                    .args([
                        "run",
                        ".#tonk-ui-test-server",
                        "--",
                        &format!("{web_port}"),
                        &format!("{access_service_port}"),
                    ])
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
                    tonk_web: Url::parse(&format!("http://tonk.spot:{web_port}"))?,
                    chromedriver: Url::parse(&format!("http://127.0.0.1:{chromedriver_port}"))?,
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
            // Dropping the access-service provider closes its shutdown senders;
            // explicit success paths still await orderly shutdown in `stop`.
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
