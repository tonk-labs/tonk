pub mod backend;
pub mod filesystem;
pub mod traversal;
pub mod types;
pub mod watcher;

pub use filesystem::*;
pub use types::*;
pub use watcher::DocumentWatcher;

use crate::error::Result;
use crate::tonk_core::TonkCore;
use automerge::AutoSerde;
use samod::{DocHandle, Repo};
use std::sync::Arc;

/// Simplified API for common VFS use cases
pub struct Vfs {
    engine: TonkCore,
    vfs: Arc<VirtualFileSystem>,
}

impl Vfs {
    /// Create a new VFS instance with in-memory storage
    pub async fn new() -> Result<Self> {
        let engine = TonkCore::new().await?;
        let vfs = Arc::new(VirtualFileSystem::new(engine.samod()).await?);

        Ok(Self { engine, vfs })
    }

    /// Create a new VFS instance with a specific peer ID
    pub async fn with_peer_id(peer_id: samod::PeerId) -> Result<Self> {
        let engine = TonkCore::with_peer_id(peer_id).await?;
        let vfs = Arc::new(VirtualFileSystem::new(engine.samod()).await?);

        Ok(Self { engine, vfs })
    }

    /// Connect to a WebSocket peer
    pub async fn connect(&self, url: &str) -> Result<()> {
        self.engine.connect_websocket(url).await
    }

    /// Get VFS handle for file operations
    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        Arc::clone(&self.vfs)
    }

    /// Get the underlying samod instance for advanced operations
    pub fn samod(&self) -> Arc<Repo> {
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
    pub async fn read_file(&self, path: &str) -> Result<Option<String>> {
        if let Some(handle) = self.vfs().find_document(path).await? {
            let json_string = handle.with_document(|doc| {
                let auto_serde = AutoSerde::from(&*doc);
                serde_json::to_string_pretty(&auto_serde)
                    .unwrap_or_else(|e| format!("Error serializing: {e}"))
            });
            Ok(Some(json_string))
        } else {
            Ok(None)
        }
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
        self.vfs().metadata(path).await
    }

    /// Watch a file for changes
    pub async fn watch_file(&self, path: &str) -> Result<Option<DocumentWatcher>> {
        self.vfs().watch_document(path).await
    }

    /// Watch a directory for changes
    pub async fn watch_dir(&self, path: &str) -> Result<Option<DocumentWatcher>> {
        self.vfs().watch_directory(path).await
    }
}

impl Clone for Vfs {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            vfs: Arc::clone(&self.vfs),
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
        assert!(!vfs.peer_id().to_string().is_empty());
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
        assert!(!vfs.samod().peer_id().to_string().is_empty());

        // Test event subscription
        let _rx = vfs.subscribe_events();
    }

    #[tokio::test]
    async fn test_watch_file() {
        let vfs = Vfs::new().await.unwrap();

        // Create a file
        vfs.create_file("/watch-test.txt", "content".to_string())
            .await
            .unwrap();

        // Watch the file
        let watcher = vfs.watch_file("/watch-test.txt").await.unwrap();
        assert!(watcher.is_some());
    }

    #[tokio::test]
    async fn test_watch_dir() {
        let vfs = Vfs::new().await.unwrap();

        // Create a directory
        vfs.create_dir("/watch-dir").await.unwrap();

        // Watch the directory
        let watcher = vfs.watch_dir("/watch-dir").await.unwrap();
        assert!(watcher.is_some());

        // Test watching root
        let root_watcher = vfs.watch_dir("/").await.unwrap();
        assert!(root_watcher.is_some());
    }
}
