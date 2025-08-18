use crate::p2p::{manager::AutomergeMessage, P2PManager};
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

type P2PState = Arc<Mutex<P2PManager>>;

#[tauri::command]
pub async fn initialize_p2p(
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager.initialize().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_discovery(
    bundle_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager.start_discovery(bundle_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_to_peer(
    peer_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager.connect_to_peer(peer_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_automerge_message(
    target_id: String,
    message: AutomergeMessage,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .send_automerge_message(target_id, message)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn disconnect_all_peers(
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager.disconnect_all_peers().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_node_id(
    p2p_state: State<'_, P2PState>,
) -> Result<Option<String>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_node_id())
}

#[tauri::command]
pub async fn get_connected_peers(
    p2p_state: State<'_, P2PState>,
) -> Result<Vec<crate::p2p::manager::PeerInfo>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_connected_peers().await)
}