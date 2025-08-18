use anyhow::Result;
use iroh::node::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomergeMessage {
    pub message_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub peer_id: String,
    pub bundle_id: String,
    pub connected: bool,
}

pub struct P2PManager {
    node: Option<Node<iroh::blobs::store::mem::Store>>,
    peers: Arc<RwLock<HashMap<String, PeerInfo>>>,
    bundle_id: Option<String>,
    app_handle: AppHandle,
}

impl P2PManager {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            node: None,
            peers: Arc::new(RwLock::new(HashMap::new())),
            bundle_id: None,
            app_handle,
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        // Initialize Iroh node with memory store
        let node = Node::memory().spawn().await?;
        self.node = Some(node);

        println!("P2P Manager initialized successfully");
        Ok(())
    }

    pub async fn start_discovery(&mut self, bundle_id: String) -> Result<()> {
        self.bundle_id = Some(bundle_id.clone());

        // TODO: Implement mDNS and DHT discovery
        println!("Starting discovery for bundle: {}", bundle_id);

        // Emit ready event to frontend
        self.app_handle
            .emit("p2p_ready", serde_json::json!({ "bundleId": bundle_id }))
            .map_err(|e| anyhow::anyhow!("Failed to emit p2p_ready event: {}", e))?;

        Ok(())
    }

    pub async fn connect_to_peer(&self, peer_id: String) -> Result<()> {
        println!("Connecting to peer: {}", peer_id);

        // TODO: Implement actual peer connection using Iroh
        
        // For now, simulate a successful connection
        {
            let mut peers = self.peers.write().await;
            peers.insert(
                peer_id.clone(),
                PeerInfo {
                    peer_id: peer_id.clone(),
                    bundle_id: self.bundle_id.clone().unwrap_or_default(),
                    connected: true,
                },
            );
        }

        // Emit connection event to frontend
        self.app_handle
            .emit("peer_connected", serde_json::json!({ "peerId": peer_id }))
            .map_err(|e| anyhow::anyhow!("Failed to emit peer_connected event: {}", e))?;

        Ok(())
    }

    pub async fn send_automerge_message(
        &self,
        target_id: String,
        message: AutomergeMessage,
    ) -> Result<()> {
        println!("Sending message to peer: {}", target_id);

        // TODO: Implement actual message sending via Iroh connection
        
        // For now, just log the message
        println!("Message type: {}, data length: {}", message.message_type, message.data.len());

        Ok(())
    }

    pub async fn disconnect_all_peers(&self) -> Result<()> {
        println!("Disconnecting all peers");

        {
            let mut peers = self.peers.write().await;
            peers.clear();
        }

        self.app_handle
            .emit("all_peers_disconnected", serde_json::json!({}))
            .map_err(|e| anyhow::anyhow!("Failed to emit all_peers_disconnected event: {}", e))?;

        Ok(())
    }

    pub fn get_node_id(&self) -> Option<String> {
        // TODO: Return actual node ID from Iroh
        // For now, return a mock UUID
        Some(Uuid::new_v4().to_string())
    }

    pub async fn get_connected_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().filter(|p| p.connected).cloned().collect()
    }
}