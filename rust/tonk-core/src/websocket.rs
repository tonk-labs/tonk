use crate::error::Result;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::VfsError;
use samod::{ConnDirection, ConnFinishedReason, Repo};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tokio_tungstenite::connect_async;

/// Handle to control an active WebSocket connection
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct ConnectionHandle {
    task: tokio::task::JoinHandle<ConnFinishedReason>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ConnectionHandle {
    /// Disconnect from the WebSocket server
    ///
    /// This aborts the connection task. The connection may not be gracefully closed.
    pub fn disconnect(self) {
        self.task.abort();
    }

    /// Wait for the connection to finish and get the reason it ended
    pub async fn finished(self) -> std::result::Result<ConnFinishedReason, tokio::task::JoinError> {
        self.task.await
    }

    /// Check if the connection is still active
    pub fn is_connected(&self) -> bool {
        !self.task.is_finished()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn connect(samod: Arc<Repo>, url: &str) -> Result<ConnectionHandle> {
    let (ws_stream, _) = connect_async(url)
        .await
        .map_err(|e| VfsError::WebSocketError(format!("Failed to connect to {url}: {e}")))?;

    let task = tokio::spawn(async move {
        samod
            .connect_tungstenite(ws_stream, ConnDirection::Outgoing)
            .await
    });

    Ok(ConnectionHandle { task })
}

#[cfg(target_arch = "wasm32")]
pub async fn connect_wasm(samod: Arc<Repo>, url: &str) -> Result<ConnFinishedReason> {
    Ok(samod
        .connect_wasm_websocket(url, ConnDirection::Outgoing)
        .await)
}
