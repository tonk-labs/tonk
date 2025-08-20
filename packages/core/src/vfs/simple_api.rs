use crate::error::Result;
use crate::sync::SyncEngine;
use crate::vfs::{RefNode, VfsEvent, VirtualFileSystem};
use samod::{DocHandle, Samod};
use std::sync::Arc;

/// Simplified API for common VFS use cases
pub struct Vfs {
    engine: SyncEngine,
    vfs: Arc<VirtualFileSystem>,
}

impl Vfs {
    /// Create a new VFS instance with in-memory storage
    pub async fn new() -> Result<Self> {
        let engine = SyncEngine::new().await?;
        let vfs = Arc::new(VirtualFileSystem::new(engine.samod()).await?);

        Ok(Self { engine, vfs })
    }

    /// Create a new VFS instance with a specific peer ID
    pub async fn with_peer_id(peer_id: samod::PeerId) -> Result<Self> {
        let engine = SyncEngine::with_peer_id(peer_id).await?;
        let vfs = Arc::new(VirtualFileSystem::new(engine.samod()).await?);

        Ok(Self { engine, vfs })
    }

    /// Connect to a WebSocket peer
    pub async fn connect(&self, url: &str) -> Result<()> {
        self.engine.connect_websocket(url).await
    }

    /// Get VFS handle for file operations
    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        self.vfs.clone()
    }

    /// Get the underlying samod instance for advanced operations
    pub fn samod(&self) -> Arc<Samod> {
        self.engine.samod()
    }

    /// Get the peer ID of this VFS instance
    pub fn peer_id(&self) -> samod::PeerId {
        self.engine.peer_id()
    }

    /// Subscribe to VFS events
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<VfsEvent> {
        self.vfs.subscribe_events()
    }

    /// Create a document at a path
    pub async fn create_file<T>(&self, path: &str, content: T) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        self.vfs().create_document(path, content).await
    }

    /// Read a document at a path
    pub async fn read_file(&self, path: &str) -> Result<Option<DocHandle>> {
        self.vfs().find_document(path).await
    }

    /// Delete a document at a path
    pub async fn delete_file(&self, path: &str) -> Result<bool> {
        self.vfs().remove_document(path).await
    }

    /// Create a directory at a path
    pub async fn create_dir(&self, path: &str) -> Result<DocHandle> {
        self.vfs().create_directory(path).await
    }

    /// List files in a directory
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RefNode>> {
        self.vfs().list_directory(path).await
    }

    /// Check if a path exists
    pub async fn exists(&self, path: &str) -> Result<bool> {
        self.vfs().exists(path).await
    }

    /// Get metadata for a path
    pub async fn metadata(
        &self,
        path: &str,
    ) -> Result<Option<(crate::vfs::NodeType, crate::vfs::Timestamps)>> {
        self.vfs().get_metadata(path).await
    }
}

impl Clone for Vfs {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            vfs: self.vfs.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::rng;

    use super::*;

    #[tokio::test]
    async fn test_vfs_creation() {
        let vfs = Vfs::new().await.unwrap();
        assert!(vfs.peer_id().to_string().len() > 0);
    }

    #[tokio::test]
    async fn test_vfs_with_peer_id() {
        let mut rng = rng();
        let peer_id = samod::PeerId::new_with_rng(&mut rng);
        let vfs = Vfs::with_peer_id(peer_id.clone()).await.unwrap();
        assert_eq!(vfs.peer_id(), peer_id);
    }

    #[tokio::test]
    async fn test_basic_operations() {
        let vfs = Vfs::new().await.unwrap();

        // Test that we can access the underlying components
        assert!(vfs.samod().peer_id().to_string().len() > 0);

        // Test event subscription
        let _rx = vfs.subscribe_events();
    }
}
