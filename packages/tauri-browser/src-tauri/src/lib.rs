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
            get_connected_peers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
