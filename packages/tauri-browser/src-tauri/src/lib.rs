use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::Manager;

mod commands;
mod p2p;

use commands::*;
use p2p::P2PManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_handle = app.handle().clone();
            let p2p_manager = Arc::new(Mutex::new(P2PManager::new(app_handle)));
            app.manage(p2p_manager);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            initialize_p2p,
            start_discovery,
            connect_to_peer,
            send_automerge_message,
            disconnect_all_peers,
            get_node_id,
            get_connected_peers,
            get_discovered_peers,
            configure_mdns_discovery,
            toggle_local_discovery,
            restart_discovery,
            get_p2p_status,
            get_connection_status,
            get_all_connection_attempts,
            reset_connection_attempts,
            configure_connection_manager,
            get_network_interfaces,
            get_primary_network_interface,
            check_network_connectivity,
            refresh_network_info,
            get_discovery_preferences,
            update_discovery_preferences,
            reset_discovery_preferences,
            export_discovery_config,
            import_discovery_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
