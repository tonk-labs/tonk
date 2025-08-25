use crate::error::{Result, VfsError};
use crate::vfs::VirtualFileSystem;
use rand::rng;
use samod::storage::InMemoryStorage;
use samod::{ConnDirection, DocHandle, DocumentId, PeerId, Samod};
use std::sync::Arc;
use tokio_tungstenite::connect_async;
use tracing::info;

/// Core synchronization engine that orchestrates CRDT operations and VFS interactions.
///
/// SyncEngine combines samod (CRDT synchronization) with a virtual file system layer,
/// providing a unified interface for document synchronization and file operations.
/// The engine manages peer connections, document creation/retrieval, and real-time
/// synchronization of changes across distributed systems.
///
/// # Examples
///
/// ```no_run
/// # use tonk_core::sync::SyncEngine;
/// # async fn example() {
/// let engine = SyncEngine::new().await.unwrap();
/// let vfs = engine.vfs();
/// // Use VFS for file operations...
/// # }
/// ```
pub struct SyncEngine {
    samod: Arc<Samod>,
    vfs: Arc<VirtualFileSystem>,
}

impl SyncEngine {
    /// Create a new SyncEngine with a randomly generated peer ID.
    ///
    /// The engine will use in-memory storage by default. For persistent storage,
    /// use alternative constructors or configure the underlying samod instance.
    ///
    /// # Returns
    /// A Result containing the new SyncEngine instance or an error if initialization fails.
    ///
    /// # Examples
    /// ```no_run
    /// # use tonk_core::sync::SyncEngine;
    /// # async fn example() {
    /// let engine = SyncEngine::new().await.unwrap();
    /// println!("Engine peer ID: {}", engine.peer_id());
    /// # }
    /// ```
    pub async fn new() -> Result<Self> {
        let mut rng = rng();
        Self::with_peer_id(PeerId::new_with_rng(&mut rng)).await
    }

    /// Create a new SyncEngine with a specific peer ID
    pub async fn with_peer_id(peer_id: PeerId) -> Result<Self> {
        // Use samod's built-in InMemoryStorage
        // TODO: add option for FilesystemStorage
        let storage = InMemoryStorage::new();

        // Build samod instance with the storage and peer ID
        let samod = Samod::build_tokio()
            .with_storage(storage)
            .with_peer_id(peer_id)
            .load()
            .await;

        let samod = Arc::new(samod);

        // Create VFS layer on top of samod
        let vfs = Arc::new(VirtualFileSystem::new(samod.clone()).await?);

        info!("SyncEngine initialized with peer ID: {}", samod.peer_id());

        Ok(Self { samod, vfs })
    }

    /// Get access to the VFS layer
    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        Arc::clone(&self.vfs)
    }

    /// Get access to the underlying Samod instance
    pub fn samod(&self) -> Arc<Samod> {
        Arc::clone(&self.samod)
    }

    /// Get the peer ID of this sync engine
    pub fn peer_id(&self) -> PeerId {
        self.samod.peer_id()
    }

    /// Connect to a WebSocket peer
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        info!("Connecting to WebSocket peer at: {}", url);

        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| VfsError::WebSocketError(format!("Failed to connect to {url}: {e}")))?;

        // Use samod's built-in WebSocket support
        let conn_finished = self
            .samod
            .connect_tungstenite(ws_stream, ConnDirection::Outgoing)
            .await;

        info!("Successfully connected to WebSocket peer at: {}", url);
        info!("Connection finished with reason: {:?}", conn_finished);
        Ok(())
    }

    /// Find a document by its ID
    pub async fn find_document(&self, doc_id: DocumentId) -> Result<DocHandle> {
        self.samod
            .find(doc_id.clone())
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find document {doc_id}: {e}")))?
            .ok_or_else(|| VfsError::SamodError(format!("Document {doc_id} not found")))
    }

    /// Create a new document
    pub async fn create_document(&self, initial_doc: automerge::Automerge) -> Result<DocHandle> {
        self.samod
            .create(initial_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {e}")))
    }

    /// Subscribe to document changes
    pub fn subscribe_to_changes(&self) -> tokio::sync::broadcast::Receiver<DocumentId> {
        // TODO: This is a placeholder for change notifications
        // We'll need to implement this based on samod's event system
        let (tx, rx) = tokio::sync::broadcast::channel(100);
        std::mem::drop(tx); // Close the sender to indicate no events for now
        rx
    }
}

impl Clone for SyncEngine {
    fn clone(&self) -> Self {
        Self {
            samod: Arc::clone(&self.samod),
            vfs: Arc::clone(&self.vfs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_sync_engine_creation() {
        let engine = SyncEngine::new().await.unwrap();
        assert!(!engine.peer_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_sync_engine_with_peer_id() {
        let mut rng = rand::rng();
        let peer_id = PeerId::new_with_rng(&mut rng);
        let engine = SyncEngine::with_peer_id(peer_id.clone()).await.unwrap();
        assert_eq!(engine.peer_id(), peer_id);
    }

    #[tokio::test]
    async fn test_document_creation() {
        let engine = SyncEngine::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = engine.create_document(doc).await.unwrap();
        assert!(!handle.document_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_vfs_integration() {
        let engine = SyncEngine::new().await.unwrap();
        let vfs = engine.vfs();

        // Test that VFS is accessible
        assert!(!vfs.root_id().to_string().is_empty());

        // Test that we can subscribe to VFS events
        let _rx = vfs.subscribe_events();

        // Test that both references point to the same samod instance
        let engine_samod = engine.samod();
        // NOTE: We can't easily compare Arc<Samod> for equality, but we can check peer IDs
        assert_eq!(engine.peer_id(), engine_samod.peer_id());
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
