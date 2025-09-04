use std::process::{Child, Command};
use std::time::Duration;
use tokio::time::sleep;
use tonk_core::TonkCore;

/// Test sync server wrapper
pub struct TestSyncServer {
    process: Option<Child>,
    port: u16,
}

impl TestSyncServer {
    /// Start a test sync server on a random port
    pub async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        // Find an available port
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener); // Release the port

        // Start the sync server from examples
        let mut process = Command::new("node")
            .arg("examples/server/server.ts")
            .env("PORT", port.to_string())
            .spawn()?;

        // Give the server time to start
        sleep(Duration::from_millis(500)).await;

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

impl Drop for TestSyncServer {
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
pub async fn load_from_bundle(bundle_bytes: Vec<u8>) -> Result<TonkCore, Box<dyn std::error::Error>> {
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

/// Simple mock sync server for tests that don't need full server
pub mod mock_server {
    use futures::{SinkExt, StreamExt};
    use tokio::sync::broadcast;
    use warp::Filter;

    pub async fn start_mock_sync_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
        let (tx, _rx) = broadcast::channel(100);

        let ws_route =
            warp::path::end()
                .and(warp::filters::ws::ws())
                .map(move |ws: warp::ws::Ws| {
                    let tx = tx.clone();
                    ws.on_upgrade(move |websocket| handle_connection(websocket, tx))
                });

        warp::serve(ws_route).run(([127, 0, 0, 1], port)).await;

        Ok(())
    }

    async fn handle_connection(ws: warp::ws::WebSocket, tx: broadcast::Sender<Vec<u8>>) {
        let (mut ws_sender, mut ws_receiver) = ws.split();
        let mut rx = tx.subscribe();

        // Echo messages back and broadcast to other clients
        tokio::spawn(async move {
            while let Some(result) = ws_receiver.next().await {
                if let Ok(msg) = result {
                    let bytes = msg.as_bytes();
                    let _ = tx.send(bytes.to_vec());
                }
            }
        });

        // Forward broadcast messages to this client
        while let Ok(msg) = rx.recv().await {
            if ws_sender
                .send(warp::ws::Message::binary(msg))
                .await
                .is_err()
            {
                break;
            }
        }
    }
}

/// Helper to wait for sync between two TonkCore instances
pub async fn wait_for_sync(timeout: Duration) {
    sleep(timeout).await;
}

/// Check if a port is available
pub fn is_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}
