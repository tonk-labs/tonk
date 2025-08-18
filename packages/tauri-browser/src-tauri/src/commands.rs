use crate::p2p::{manager::AutomergeMessage, P2PManager};
use crate::p2p::mdns::{MdnsConfig, TonkServiceInfo};
use crate::p2p::connection::{ConnectionConfig, ConnectionAttempt};
use crate::p2p::network::NetworkInterface;
use crate::p2p::config::DiscoveryPreferences;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;

type P2PState = Arc<Mutex<P2PManager>>;

#[tauri::command]
pub async fn initialize_p2p(p2p_state: State<'_, P2PState>) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager.initialize().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_discovery(
    bundle_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .start_discovery(bundle_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_to_peer(
    peer_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .connect_to_peer(peer_id)
        .await
        .map_err(|e| e.to_string())
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
pub async fn disconnect_all_peers(p2p_state: State<'_, P2PState>) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .disconnect_all_peers()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_node_id(p2p_state: State<'_, P2PState>) -> Result<Option<String>, String> {
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

#[tauri::command]
pub async fn get_discovered_peers(
    p2p_state: State<'_, P2PState>,
) -> Result<Vec<TonkServiceInfo>, String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .get_discovered_peers()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn configure_mdns_discovery(
    config: MdnsConfig,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .configure_mdns(config)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn toggle_local_discovery(
    enabled: bool,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .toggle_discovery(enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restart_discovery(p2p_state: State<'_, P2PState>) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .restart_discovery()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_p2p_status(p2p_state: State<'_, P2PState>) -> Result<serde_json::Value, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(serde_json::json!({
        "nodeId": p2p_manager.get_node_id(),
        "bundleId": p2p_manager.get_bundle_id(),
        "port": p2p_manager.get_port(),
        "connected": p2p_manager.get_node_id().is_some(),
    }))
}

#[tauri::command]
pub async fn get_connection_status(
    peer_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<Option<ConnectionAttempt>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_connection_status(&peer_id).await)
}

#[tauri::command]
pub async fn get_all_connection_attempts(
    p2p_state: State<'_, P2PState>,
) -> Result<HashMap<String, ConnectionAttempt>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_all_connection_attempts().await)
}

#[tauri::command]
pub async fn reset_connection_attempts(
    peer_id: String,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .reset_connection_attempts(&peer_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn configure_connection_manager(
    config: ConnectionConfig,
    p2p_state: State<'_, P2PState>,
) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .configure_connections(config)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_network_interfaces(
    p2p_state: State<'_, P2PState>,
) -> Result<Option<HashMap<String, NetworkInterface>>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_network_interfaces().await)
}

#[tauri::command]
pub async fn get_primary_network_interface(
    p2p_state: State<'_, P2PState>,
) -> Result<Option<NetworkInterface>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_primary_interface().await)
}

#[tauri::command]
pub async fn check_network_connectivity(p2p_state: State<'_, P2PState>) -> Result<bool, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.has_network_connectivity().await)
}

#[tauri::command]
pub async fn refresh_network_info(p2p_state: State<'_, P2PState>) -> Result<(), String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .refresh_network_info()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_discovery_preferences(
    p2p_state: State<'_, P2PState>,
) -> Result<Option<DiscoveryPreferences>, String> {
    let p2p_manager = p2p_state.lock().await;
    Ok(p2p_manager.get_discovery_preferences().await)
}

#[tauri::command]
pub async fn update_discovery_preferences(
    preferences: DiscoveryPreferences,
    p2p_state: State<'_, P2PState>,
) -> Result<Vec<String>, String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .update_discovery_preferences(preferences)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reset_discovery_preferences(p2p_state: State<'_, P2PState>) -> Result<(), String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .reset_discovery_preferences()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_discovery_config(
    p2p_state: State<'_, P2PState>,
) -> Result<Option<String>, String> {
    let p2p_manager = p2p_state.lock().await;
    p2p_manager
        .export_discovery_config()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_discovery_config(
    config_json: String,
    p2p_state: State<'_, P2PState>,
) -> Result<Vec<String>, String> {
    let mut p2p_manager = p2p_state.lock().await;
    p2p_manager
        .import_discovery_config(&config_json)
        .await
        .map_err(|e| e.to_string())
}



