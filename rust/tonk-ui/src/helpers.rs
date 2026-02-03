#[cfg(not(target_arch = "wasm32"))]
mod native {
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use dialog_common::helpers::{Provider, Service};
    use port_check::free_local_port;
    use serde::{Deserialize, Serialize};
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};
    use thirtyfour::{ChromiumLikeCapabilities, DesiredCapabilities, WebDriver};
    use tonk_access_service::helpers::{AccessServiceAddress, AccessServiceSettings};
    use url::Url;

    /// Test environment configuration for integration tests.
    #[derive(Deserialize, Serialize, Debug, Clone)]
    pub struct TestEnvironment {
        /// URL of the Tonk web server.
        pub tonk_web: Url,
        /// URL of the ChromeDriver server.
        pub chromedriver: Url,
        /// Access service connection info.
        pub access_service: AccessServiceAddress,
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

            if let Ok(chrome_binary) = std::env::var("CHROME") {
                caps.set_binary(&chrome_binary)?;
            }

            let driver = WebDriver::new(&self.chromedriver.to_string(), caps).await?;
            driver.goto(&self.tonk_web.to_string()).await?;
            Ok(driver)
        }

        /// Returns the access service URL with /ucan/ path appended.
        pub fn access_service_url(&self) -> String {
            format!("{}/ucan/", self.access_service.access_service_url)
        }
    }

    /// Manages test server processes for integration testing.
    pub struct TestServers {
        web_server: Child,
        chromedriver: Child,
        access_service: Service<AccessServiceAddress, tonk_access_service::helpers::AccessServer>,
    }

    impl TestServers {
        /// Starts the test servers and returns the server handles and environment configuration.
        pub async fn start() -> Result<(Self, TestEnvironment)> {
            let web_port =
                free_local_port().expect("Could not get a free local port for test server");
            let mut web_server = std::process::Command::new("nix")
                .args(["run", ".#tonk-ui-test-server", "--", &format!("{web_port}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdout = web_server
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

            let chromedriver_port =
                free_local_port().expect("Could not get a free local port for chromedriver");
            let mut chromedriver = std::process::Command::new("chromedriver")
                .args([&format!("--port={chromedriver_port}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdout = chromedriver
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

            // Start the access service using tonk-access-service's provider
            let settings = AccessServiceSettings::default();
            let access_service = tonk_access_service::helpers::access_service(settings).await?;

            let access_service_address = access_service.address.clone();

            Ok((
                Self {
                    web_server,
                    chromedriver,
                    access_service,
                },
                TestEnvironment {
                    tonk_web: Url::parse(&format!("http://127.0.0.1:{web_port}"))?,
                    chromedriver: Url::parse(&format!("http://127.0.0.1:{chromedriver_port}"))?,
                    access_service: access_service_address,
                },
            ))
        }

        /// Stops all test server processes.
        pub async fn stop(mut self) -> Result<()> {
            self.web_server.kill()?;
            self.chromedriver.kill()?;
            self.access_service.stop().await?;
            Ok(())
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
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
