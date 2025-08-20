use crate::error::{Result, VfsError};
use samod::storage::InMemoryStorage;
use samod::{ConnDirection, DocHandle, DocumentId, PeerId, Samod};
use std::sync::Arc;
use tokio_tungstenite::connect_async;
use tracing::{error, info};

pub struct SyncEngine {
    samod: Arc<Samod>,
}

impl SyncEngine {
    /// Create a new SyncEngine with a random peer ID
    pub async fn new() -> Result<Self> {
        Self::with_peer_id(PeerId::random()).await
    }

    /// Create a new SyncEngine with a specific peer ID
    pub async fn with_peer_id(peer_id: PeerId) -> Result<Self> {
        // Use samod's built-in InMemoryStorage
        let storage = InMemoryStorage::new();

        // Build samod instance with the storage and peer ID
        let samod = Samod::build_tokio()
            .with_storage(storage)
            .with_peer_id(peer_id)
            .load()
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to initialize samod: {}", e)))?;

        let samod = Arc::new(samod);

        info!("SyncEngine initialized with peer ID: {}", peer_id);

        Ok(Self { samod })
    }

    /// Get access to the underlying Samod instance
    pub fn samod(&self) -> Arc<Samod> {
        self.samod.clone()
    }

    /// Get the peer ID of this sync engine
    pub fn peer_id(&self) -> PeerId {
        self.samod.get_peer_id()
    }

    /// Connect to a WebSocket peer
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        info!("Connecting to WebSocket peer at: {}", url);

        let (ws_stream, _) = connect_async(url).await.map_err(|e| {
            VfsError::WebSocketError(format!("Failed to connect to {}: {}", url, e))
        })?;

        // Use samod's built-in WebSocket support
        let conn_finished = self
            .samod
            .connect_tungstenite(ws_stream, ConnDirection::Outgoing)
            .await;

        match conn_finished {
            Ok(_) => {
                info!("Successfully connected to WebSocket peer at: {}", url);
                Ok(())
            }
            Err(e) => {
                error!("WebSocket connection failed: {:?}", e);
                Err(VfsError::WebSocketError(format!(
                    "Connection failed: {:?}",
                    e
                )))
            }
        }
    }

    /// Find a document by its ID
    pub async fn find_document(&self, doc_id: DocumentId) -> Result<DocHandle> {
        self.samod
            .find(doc_id)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find document {}: {}", doc_id, e)))
    }

    /// Create a new document
    pub async fn create_document(&self, initial_doc: automerge::Automerge) -> Result<DocHandle> {
        self.samod
            .create(initial_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {}", e)))
    }

    /// Get all document IDs currently stored
    pub async fn list_documents(&self) -> Result<Vec<DocumentId>> {
        // Note: This method might not be available in samod's public API
        // This is a placeholder for potential future functionality
        Ok(Vec::new())
    }

    /// Subscribe to document changes
    pub fn subscribe_to_changes(&self) -> tokio::sync::broadcast::Receiver<DocumentId> {
        // This is a placeholder for change notifications
        // We'll need to implement this based on samod's event system
        let (tx, rx) = tokio::sync::broadcast::channel(100);
        std::mem::drop(tx); // Close the sender to indicate no events for now
        rx
    }
}

impl Clone for SyncEngine {
    fn clone(&self) -> Self {
        Self {
            samod: self.samod.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn test_sync_engine_creation() {
        let engine = SyncEngine::new().await.unwrap();
        assert!(engine.peer_id().to_string().len() > 0);
    }

    #[tokio::test]
    async fn test_sync_engine_with_peer_id() {
        let peer_id = PeerId::random();
        let engine = SyncEngine::with_peer_id(peer_id).await.unwrap();
        assert_eq!(engine.peer_id(), peer_id);
    }

    #[tokio::test]
    async fn test_document_creation() {
        let engine = SyncEngine::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = engine.create_document(doc).await.unwrap();
        assert!(handle.document_id().to_string().len() > 0);
    }

    #[tokio::test]
    async fn test_websocket_connection_failure() {
        let engine = SyncEngine::new().await.unwrap();

        // Test connection to invalid URL
        let result = timeout(
            Duration::from_secs(1),
            engine.connect_websocket("ws://localhost:99999"),
        )
        .await;

        // Should either timeout or fail with connection error
        match result {
            Ok(Err(_)) => (), // Connection failed as expected
            Err(_) => (),     // Timed out as expected
            Ok(Ok(_)) => panic!("Connection should have failed"),
        }
    }
}
