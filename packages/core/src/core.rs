use crate::{bundle::Bundle, error::VfsError, sync::SyncEngine, vfs::VirtualFileSystem};
use std::{io::Write, path::Path, sync::Arc};

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
    pub fn manifest(&self) -> &crate::bundle::Manifest {
        self.bundle.manifest()
    }

    /// Connect to websocket sync server
    pub async fn connect_websocket(&self, url: &str) -> Result<(), VfsError> {
        self.engine.connect_websocket(url).await
    }

    /// Connect using network URIs from manifest
    pub async fn connect_from_manifest(&self) -> Result<(), VfsError> {
        for uri in &self.manifest().network_uris {
            if uri.starts_with("ws://") || uri.starts_with("wss://") {
                if let Ok(()) = self.connect_websocket(uri).await {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Sync VFS state back to bundle
    async fn sync_vfs_to_bundle(&mut self) -> Result<(), VfsError> {
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

        // Create a completely new bundle with updated root document
        // This avoids the issue of trying to update existing files in ZIP
        let manifest = self.bundle.manifest().clone();

        // Create new in-memory ZIP
        let mut zip_data = Vec::new();
        {
            let mut zip_writer = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_data));

            // Add manifest
            let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| {
                VfsError::Other(anyhow::anyhow!("Failed to serialize manifest: {e}"))
            })?;
            zip_writer
                .start_file("manifest.json", zip::write::SimpleFileOptions::default())
                .map_err(|e| {
                    VfsError::Other(anyhow::anyhow!("Failed to start manifest file: {e}"))
                })?;
            zip_writer
                .write_all(manifest_json.as_bytes())
                .map_err(|e| VfsError::Other(anyhow::anyhow!("Failed to write manifest: {e}")))?;

            // Add root document
            zip_writer
                .start_file(&manifest.root, zip::write::SimpleFileOptions::default())
                .map_err(|e| VfsError::Other(anyhow::anyhow!("Failed to start root file: {e}")))?;
            zip_writer.write_all(&root_doc_bytes).map_err(|e| {
                VfsError::Other(anyhow::anyhow!("Failed to write root document: {e}"))
            })?;

            // Copy any other files from the existing bundle
            for key in self.bundle.list_keys() {
                let path = key.to_string();
                if path != "manifest.json" && path != manifest.root {
                    if let Some(data) = self.bundle.get(&key)? {
                        zip_writer
                            .start_file(&path, zip::write::SimpleFileOptions::default())
                            .map_err(|e| {
                                VfsError::Other(anyhow::anyhow!(
                                    "Failed to start file {}: {e}",
                                    path
                                ))
                            })?;
                        zip_writer.write_all(&data).map_err(|e| {
                            VfsError::Other(anyhow::anyhow!("Failed to write file {}: {e}", path))
                        })?;
                    }
                }
            }

            zip_writer
                .finish()
                .map_err(|e| VfsError::Other(anyhow::anyhow!("Failed to finish ZIP: {e}")))?;
        }

        // Replace the bundle with the new one
        self.bundle = Bundle::from_bytes(zip_data)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::BundlePath;

    #[tokio::test]
    async fn test_init_from_bundle() {
        // Test internal initialization logic
        let bundle = Bundle::create_empty().unwrap();
        let tonk = TonkCore::from_bundle(bundle).await.unwrap();

        // Verify components are initialized
        assert!(!tonk.vfs.root_id().to_string().is_empty());
        assert!(!tonk.engine.peer_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_sync_vfs_to_bundle() {
        // Test the sync_vfs_to_bundle method
        let mut tonk = TonkCore::new().await.unwrap();

        // Make changes via VFS
        tonk.vfs
            .create_document("/test.txt", "content".to_string())
            .await
            .unwrap();

        // Sync to bundle
        tonk.sync_vfs_to_bundle().await.unwrap();

        // Verify bundle was updated by checking that we can still access the root
        let root_path = tonk.bundle.manifest().root.clone();
        let updated_root = tonk
            .bundle
            .get(&BundlePath::from(root_path.as_str()))
            .unwrap();
        assert!(updated_root.is_some());

        // Verify we can load the root document
        let root_doc = tonk.bundle.root_document().unwrap();

        // The document should be valid - check that it has a type field
        use automerge::ReadDoc;
        let doc_type = root_doc.get(automerge::ROOT, "type").unwrap();
        assert_eq!(doc_type.unwrap().0.to_str().unwrap(), "dir");
    }

    #[tokio::test]
    async fn test_manifest_access() {
        let tonk = TonkCore::new().await.unwrap();
        let manifest = tonk.manifest();
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.root, "root");
        assert_eq!(manifest.version.major, 1);
        assert_eq!(manifest.version.minor, 0);
        assert!(manifest.entrypoints.is_empty());
        assert!(manifest.network_uris.is_empty());
    }

    #[tokio::test]
    async fn test_component_access() {
        let tonk = TonkCore::new().await.unwrap();

        // Test VFS access
        let vfs = tonk.vfs();
        assert!(!vfs.root_id().to_string().is_empty());

        // Test engine access
        let engine = tonk.engine();
        assert!(!engine.peer_id().to_string().is_empty());

        // Test that both have valid IDs
        assert!(!vfs.root_id().to_string().is_empty());
        assert!(!engine.vfs().root_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_empty_bundle_initialization() {
        let tonk = TonkCore::new().await.unwrap();

        // Verify VFS is functional
        let root_contents = tonk.vfs.list_directory("/").await.unwrap();
        assert_eq!(root_contents.len(), 0);

        // Verify we can create files
        tonk.vfs
            .create_document("/init.txt", "initialized".to_string())
            .await
            .unwrap();

        let updated_contents = tonk.vfs.list_directory("/").await.unwrap();
        assert_eq!(updated_contents.len(), 1);
    }

    #[tokio::test]
    #[ignore = "Requires filesystem storage adapter for samod repo"]
    async fn test_bundle_roundtrip_preserves_state() {
        let mut tonk = TonkCore::new().await.unwrap();

        // Create some content
        tonk.vfs.create_directory("/test_dir").await.unwrap();
        tonk.vfs
            .create_document("/test_dir/file.txt", "test content".to_string())
            .await
            .unwrap();

        // Export to bytes
        let bytes = tonk.to_bytes().await.unwrap();

        // Create new instance from bytes
        let tonk2 = TonkCore::from_bytes(bytes).await.unwrap();

        // Verify content exists
        assert!(tonk2.vfs.exists("/test_dir").await.unwrap());
        assert!(tonk2.vfs.exists("/test_dir/file.txt").await.unwrap());

        // Verify manifest is preserved
        assert_eq!(
            tonk.manifest().manifest_version,
            tonk2.manifest().manifest_version
        );
    }

    #[tokio::test]
    async fn test_connect_from_empty_manifest() {
        let tonk = TonkCore::new().await.unwrap();

        // Should succeed without error when no network URIs
        tonk.connect_from_manifest().await.unwrap();
    }
}
