use crate::error::Result;
#[cfg(not(target_arch = "wasm32"))]
use crate::error::VfsError;
use samod::{ConnDirection, ConnFinishedReason, Repo};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tokio_tungstenite::connect_async;

#[cfg(not(target_arch = "wasm32"))]
pub async fn connect(samod: Arc<Repo>, url: &str) -> Result<ConnFinishedReason> {
    let (ws_stream, _) = connect_async(url)
        .await
        .map_err(|e| VfsError::WebSocketError(format!("Failed to connect to {url}: {e}")))?;

    Ok(samod
        .connect_tungstenite(ws_stream, ConnDirection::Outgoing)
        .await)
}

#[cfg(target_arch = "wasm32")]
pub async fn connect_wasm(samod: Arc<Repo>, url: &str) -> Result<ConnFinishedReason> {
    Ok(samod
        .connect_wasm_websocket(url, ConnDirection::Outgoing)
        .await)
}
