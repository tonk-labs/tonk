pub mod bundle;
pub mod error;
pub mod storage;
pub mod sync;
pub mod util;
pub mod vfs;
pub mod websocket;

pub use bundle::Bundle;
pub use util::CloneableFile;
pub use vfs::{
    DirNode, DocNode, DocumentWatcher, NodeType, RefNode, Timestamps, Vfs, VfsEvent,
    VirtualFileSystem,
};

#[cfg(target_arch = "wasm32")]
pub mod wasm;

use crate::{bundle::BundlePath, sync::SyncEngine};
use error::VfsError;
use std::{path::Path, sync::Arc};

pub struct TonkCore {
    bundle: Bundle<std::io::Cursor<Vec<u8>>>,
    engine: SyncEngine,
    vfs: Arc<VirtualFileSystem>,
}

impl TonkCore {
    /// Create a new empty TonkCore
    pub async fn new() -> Result<Self, VfsError> {
        let bundle = Bundle::create_empty()?;
        Self::from_bundle(bundle).await
    }

    /// Load from file
    pub async fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, VfsError> {
        let data = std::fs::read(path).map_err(VfsError::IoError)?;
        Self::from_bytes(data).await
    }

    /// Load from bytes
    pub async fn from_bytes(data: Vec<u8>) -> Result<Self, VfsError> {
        let bundle = Bundle::from_bytes(data)?;
        Self::from_bundle(bundle).await
    }

    /// Load from bundle
    pub async fn from_bundle(
        mut bundle: Bundle<std::io::Cursor<Vec<u8>>>,
    ) -> Result<Self, VfsError> {
        // Create sync engine with storage
        let engine = SyncEngine::new().await?;

        // Initialize VFS from bundle's root document
        let vfs = Arc::new(VirtualFileSystem::from_bundle(engine.samod(), &mut bundle).await?);

        Ok(Self {
            bundle,
            engine,
            vfs,
        })
    }

    /// Save to file
    pub async fn to_file<P: AsRef<Path>>(&mut self, path: P) -> Result<(), VfsError> {
        let bytes = self.to_bytes().await?;
        std::fs::write(path, bytes).map_err(VfsError::IoError)?;
        Ok(())
    }

    /// Export to bytes
    pub async fn to_bytes(&mut self) -> Result<Vec<u8>, VfsError> {
        // Update bundle with current VFS state
        self.sync_vfs_to_bundle().await?;

        // Get bytes from bundle
        self.bundle.to_bytes().map_err(|e| e.into())
    }

    /// Get VFS reference
    pub fn vfs(&self) -> &VirtualFileSystem {
        &self.vfs
    }

    /// Get sync engine reference
    pub fn engine(&self) -> &SyncEngine {
        &self.engine
    }

    /// Get bundle manifest
    pub fn manifest(&self) -> &bundle::Manifest {
        self.bundle.manifest()
    }

    /// Sync VFS state back to bundle
    pub async fn sync_vfs_to_bundle(&mut self) -> Result<(), VfsError> {
        // Get root document from VFS
        let root_handle = self
            .engine
            .samod()
            .find(self.vfs.root_id())
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find root: {e}")))?
            .ok_or_else(|| VfsError::DocumentNotFound(self.vfs.root_id().to_string()))?;

        // Serialize root document
        let root_doc_bytes = root_handle.with_document(|doc| doc.save());

        // Update bundle
        let root_path = self.bundle.manifest().root.clone();
        self.bundle
            .put(&BundlePath::from(root_path.as_str()), root_doc_bytes)?;

        Ok(())
    }
}
