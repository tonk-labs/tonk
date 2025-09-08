use std::process::{Child, Command};
use std::time::Duration;
use tokio::time::sleep;
use tonk_core::TonkCore;

/// TypeScript automerge-repo sync server wrapper
pub struct AutomergeServer {
    process: Option<Child>,
    port: u16,
}

impl AutomergeServer {
    /// Start the TypeScript automerge-repo sync server on a random port
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        // Find an available port
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener); // Release the port

        // Try tsx first, fall back to building and running with node
        let mut process = if Command::new("tsx").arg("--version").output().is_ok() {
            // tsx is available
            Command::new("tsx")
                .arg("examples/server/server.ts")
                .arg(port.to_string())
                .spawn()?
        } else {
            // Build TypeScript and run with node
            Command::new("npm")
                .arg("run")
                .arg("build")
                .current_dir("examples/server")
                .status()?;

            Command::new("node")
                .arg("examples/server/server.js")
                .arg(port.to_string())
                .spawn()?
        };

        // Give the server time to start
        sleep(Duration::from_millis(1000)).await;

        // Check if process is still running
        match process.try_wait() {
            Ok(Some(status)) => {
                return Err(format!("Server exited with status: {:?}", status).into());
            }
            Ok(None) => {
                // Still running, good
            }
            Err(e) => {
                return Err(format!("Failed to check server status: {}", e).into());
            }
        }

        Ok(Self {
            process: Some(process),
            port,
        })
    }

    /// Get the WebSocket URL for this server
    pub fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Get the HTTP URL for this server
    pub fn http_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for AutomergeServer {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

/// Helper to create test bundles with specific content
pub async fn create_test_bundle(
    files: Vec<(&str, &str)>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let tonk = TonkCore::new().await?;

    for (path, content) in files {
        // Create parent directories if needed
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 2 {
            let mut dir_path = String::new();
            for part in &parts[1..parts.len() - 1] {
                dir_path.push('/');
                dir_path.push_str(part);
                if !tonk.vfs().exists(&dir_path).await? {
                    tonk.vfs().create_directory(&dir_path).await?;
                }
            }
        }

        tonk.vfs()
            .create_document(path, content.to_string())
            .await?;
    }

    Ok(tonk.to_bytes().await?)
}

/// Helper to load a TonkCore from bundle bytes (uses in-memory storage)
pub async fn load_from_bundle(
    bundle_bytes: Vec<u8>,
) -> Result<TonkCore, Box<dyn std::error::Error>> {
    Ok(TonkCore::from_bytes(bundle_bytes).await?)
}

/// Helper to create a TonkCore with pre-populated content and return it
pub async fn create_test_tonk(
    files: Vec<(&str, &str)>,
) -> Result<TonkCore, Box<dyn std::error::Error>> {
    let tonk = TonkCore::new().await?;

    for (path, content) in files {
        // Create parent directories if needed
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 2 {
            let mut dir_path = String::new();
            for part in &parts[1..parts.len() - 1] {
                dir_path.push('/');
                dir_path.push_str(part);
                if !tonk.vfs().exists(&dir_path).await? {
                    tonk.vfs().create_directory(&dir_path).await?;
                }
            }
        }

        tonk.vfs()
            .create_document(path, content.to_string())
            .await?;
    }

    Ok(tonk)
}

/// Helper to wait for sync between two TonkCore instances
pub async fn wait_for_sync(timeout: Duration) {
    sleep(timeout).await;
}

/// Check if a port is available
pub fn is_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}
