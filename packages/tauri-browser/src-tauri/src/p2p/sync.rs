use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    pub sender_id: String,
    pub recipient_id: String,
    pub message_type: String,
    pub payload: Vec<u8>,
    pub timestamp: u64,
}

pub struct SyncManager {
    node_id: String,
}

impl SyncManager {
    pub fn new(node_id: String) -> Self {
        Self { node_id }
    }

    pub fn create_sync_message(
        &self,
        recipient_id: String,
        message_type: String,
        payload: Vec<u8>,
    ) -> SyncMessage {
        SyncMessage {
            sender_id: self.node_id.clone(),
            recipient_id,
            message_type,
            payload,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    pub async fn handle_incoming_message(&self, message: SyncMessage) -> Result<()> {
        println!(
            "Handling sync message from {} to {}: {}",
            message.sender_id, message.recipient_id, message.message_type
        );

        // TODO: Forward to Automerge via IPC
        // For now, just validate the message format
        if message.payload.is_empty() {
            return Err(anyhow::anyhow!("Empty message payload"));
        }

        Ok(())
    }
}