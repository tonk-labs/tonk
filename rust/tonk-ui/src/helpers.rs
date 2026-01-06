#[cfg(not(target_arch = "wasm32"))]
mod native {
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use dialog_common::helpers::Provider;
    use port_check::free_local_port;
    use serde::{Deserialize, Serialize};
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};
    use thirtyfour::{ChromiumLikeCapabilities, DesiredCapabilities, WebDriver};
    use url::Url;

    /// Test environment configuration for integration tests.
    #[derive(Deserialize, Serialize, Debug, Clone)]
    pub struct TestEnvironment {
        /// URL of the Tonk web server.
        pub tonk_web: Url,
        /// URL of the ChromeDriver server.
        pub chromedriver: Url,
    }

    impl TestEnvironment {
        /// Creates a new WebDriver instance connected to the test environment.
        pub async fn driver(&self) -> Result<WebDriver> {
            let mut caps = DesiredCapabilities::chrome();
            caps.set_headless()?;
            if let Some(chrome_binary) = std::option_env!("CHROME") {
                caps.set_binary(&chrome_binary)?;
            }

            let driver = WebDriver::new(&self.chromedriver.to_string(), caps).await?;
            driver.goto(&self.tonk_web.to_string()).await?;
            Ok(driver)
        }
    }

    /// Manages test server processes for integration testing.
    pub struct TestServers {
        trunk_server: Child,
        chromedriver: Child,
    }

    impl TestServers {
        /// Starts the test servers and returns the server handles and environment configuration.
        pub fn start() -> Result<(Self, TestEnvironment)> {
            let trunk_port =
                free_local_port().expect("Could not get a free local port for test server");
            let mut trunk_server = std::process::Command::new("trunk")
                .args([
                    "serve",
                    "--config",
                    "../../rust/tonk-ui/Trunk.toml",
                    "--port",
                    &format!("{trunk_port}"),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            let stdout = trunk_server
                .stdout
                .take()
                .ok_or_else(|| anyhow!("Failed to capture stdout"))?;

            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = line?;
                if line.contains("server listening at") {
                    break;
                }
            }

            let chromedriver_port =
                free_local_port().expect("Could not get a free local port for chromedriver");
            let mut chromedriver = std::process::Command::new("chromedriver")
                .arg(format!("--port={chromedriver_port}"))
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

            Ok((
                Self {
                    trunk_server,
                    chromedriver,
                },
                TestEnvironment {
                    tonk_web: Url::parse(&format!("http://127.0.0.1:{trunk_port}"))?,
                    chromedriver: Url::parse(&format!("http://127.0.0.1:{chromedriver_port}"))?,
                },
            ))
        }

        /// Stops all test server processes.
        pub fn stop(mut self) -> Result<()> {
            self.trunk_server.kill()?;
            self.chromedriver.kill()?;
            Ok(())
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    impl Provider for TestServers {
        async fn stop(mut self) -> anyhow::Result<()> {
            TestServers::stop(self)
        }
    }

    #[dialog_common::provider]
    async fn test_servers(_: ()) -> Result<Service<TestEnvironment, TestServers>> {
        use dialog_common::helpers::Service;

        let (server, address) = TestServers::start()?;
        Ok(Service::new(address, server))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
