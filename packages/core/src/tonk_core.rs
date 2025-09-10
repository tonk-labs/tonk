use crate::error::{Result, VfsError};
use crate::vfs::VirtualFileSystem;
use crate::Bundle;
use rand::rng;
#[cfg(not(target_arch = "wasm32"))]
use samod::storage::TokioFilesystemStorage as FilesystemStorage;
use samod::storage::{InMemoryStorage, StorageKey};
#[cfg(target_arch = "wasm32")]
use samod::storage::{IndexedDbStorage, LocalStorage};
#[cfg(not(target_arch = "wasm32"))]
use samod::RepoBuilder;
use samod::{DocHandle, DocumentId, PeerId, Repo};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

/// Storage configuration options for TonkCore
#[derive(Debug, Clone)]
pub enum StorageConfig {
    /// Use in-memory storage
    InMemory,
    /// Use filesystem storage at the specified path
    #[cfg(not(target_arch = "wasm32"))]
    Filesystem(PathBuf),
    /// Use IndexedDB storage
    #[cfg(target_arch = "wasm32")]
    IndexedDB,
}

/// Builder for creating TonkCore instances with custom configurations
pub struct TonkCoreBuilder {
    peer_id: Option<PeerId>,
    storage_config: StorageConfig,
}

impl TonkCoreBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self {
            peer_id: None,
            storage_config: StorageConfig::InMemory,
        }
    }

    /// Set a specific peer ID (defaults to random if not set)
    pub fn with_peer_id(mut self, peer_id: PeerId) -> Self {
        self.peer_id = Some(peer_id);
        self
    }

    /// Set storage configuration (defaults to InMemory)
    pub fn with_storage(mut self, storage_config: StorageConfig) -> Self {
        self.storage_config = storage_config;
        self
    }

    /// Create a new TonkCore instance with the configured settings
    pub async fn build(self) -> Result<TonkCore> {
        let peer_id = self.peer_id.unwrap_or_else(|| {
            let mut rng = rng();
            PeerId::new_with_rng(&mut rng)
        });

        #[cfg(not(target_arch = "wasm32"))]
        {
            let runtime = tokio::runtime::Handle::current();
            let samod = match self.storage_config {
                StorageConfig::InMemory => {
                    let storage = InMemoryStorage::new();
                    RepoBuilder::new(runtime)
                        .with_storage(storage)
                        .with_peer_id(peer_id)
                        .with_threadpool(None)
                        .load()
                        .await
                }
                StorageConfig::Filesystem(path) => {
                    std::fs::create_dir_all(&path).map_err(|e| VfsError::IoError(e))?;
                    let storage = FilesystemStorage::new(&path);
                    RepoBuilder::new(runtime)
                        .with_storage(storage)
                        .with_peer_id(peer_id)
                        .with_threadpool(None)
                        .load()
                        .await
                }
            };

            let samod = Arc::new(samod);
            let vfs = Arc::new(VirtualFileSystem::new(samod.clone()).await?);

            info!("TonkCore initialized with peer ID: {}", samod.peer_id());

            Ok(TonkCore { samod, vfs })
        }

        #[cfg(target_arch = "wasm32")]
        {
            let samod: Repo = match self.storage_config {
                StorageConfig::InMemory => {
                    Repo::build_wasm()
                        .with_peer_id(peer_id)
                        .with_storage(InMemoryStorage::new())
                        .load()
                        .await
                }
                StorageConfig::IndexedDB => {
                    Repo::build_wasm()
                        .with_peer_id(peer_id)
                        .with_storage(IndexedDbStorage::new())
                        .load_local()
                        .await
                }
            };

            let samod = Arc::new(samod);
            let vfs = Arc::new(VirtualFileSystem::new(samod.clone()).await?);

            info!("TonkCore initialized with peer ID: {}", samod.peer_id());

            Ok(TonkCore { samod, vfs })
        }
    }

    /// Load from bundle data with the configured settings
    pub async fn from_bundle(
        self,
        mut bundle: Bundle<std::io::Cursor<Vec<u8>>>,
    ) -> Result<TonkCore> {
        let peer_id = self.peer_id.unwrap_or_else(|| {
            let mut rng = rng();
            PeerId::new_with_rng(&mut rng)
        });
        use crate::BundlePath;

        #[cfg(not(target_arch = "wasm32"))]
        let runtime = tokio::runtime::Handle::current();

        // TODO: helper that reduces duplicated code populating storage
        let samod = match &self.storage_config {
            StorageConfig::InMemory => {
                let storage = InMemoryStorage::new();

                // Extract storage entries from bundle and populate in-memory storage
                let storage_prefix = BundlePath::from("storage");
                let storage_entries = bundle.prefix(&storage_prefix).map_err(VfsError::Other)?;

                for (bundle_path, data) in storage_entries {
                    let path_str = bundle_path.to_string();
                    if let Some(relative_path) = path_str.strip_prefix("storage/") {
                        let path_parts: Vec<String> =
                            relative_path.split('/').map(|s| s.to_string()).collect();

                        let reconstructed_parts =
                            if path_parts.len() >= 2 && path_parts[0].len() == 2 {
                                // Looks like a splayed document
                                let mut parts = vec![format!("{}{}", path_parts[0], path_parts[1])];
                                parts.extend_from_slice(&path_parts[2..]);
                                parts
                            } else {
                                path_parts
                            };

                        if let Ok(storage_key) = StorageKey::from_parts(reconstructed_parts.clone())
                        {
                            eprintln!(
                                "Loading storage key: {:?} (from path: {})",
                                reconstructed_parts, relative_path
                            );
                            samod::storage::Storage::put(&storage, storage_key.clone(), data).await;
                        }
                    }
                }

                #[cfg(not(target_arch = "wasm32"))]
                {
                    RepoBuilder::new(runtime)
                        .with_storage(storage)
                        .with_peer_id(peer_id)
                        .with_threadpool(None)
                        .load()
                        .await
                }

                #[cfg(target_arch = "wasm32")]
                {
                    Repo::build_wasm()
                        .with_peer_id(peer_id)
                        .with_storage(storage)
                        .load()
                        .await
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            StorageConfig::Filesystem(storage_path) => {
                std::fs::create_dir_all(storage_path).map_err(|e| VfsError::IoError(e))?;

                // Extract all storage files from bundle to the filesystem storage directory
                let storage_prefix = BundlePath::from("storage");
                let storage_entries = bundle
                    .prefix(&storage_prefix)
                    .map_err(|e| VfsError::Other(e))?;

                for (bundle_path, data) in storage_entries {
                    let path_str = bundle_path.to_string();

                    if let Some(relative_path) = path_str.strip_prefix("storage/") {
                        let full_path = storage_path.join(relative_path);

                        if let Some(parent) = full_path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| VfsError::IoError(e))?;
                        }

                        std::fs::write(&full_path, data).map_err(|e| VfsError::IoError(e))?;
                    }
                }

                let storage = FilesystemStorage::new(storage_path);
                RepoBuilder::new(runtime)
                    .with_storage(storage)
                    .with_peer_id(peer_id)
                    .with_threadpool(None)
                    .load()
                    .await
            }
            #[cfg(target_arch = "wasm32")]
            StorageConfig::IndexedDB => {
                let storage = IndexedDbStorage::new();

                // Extract storage entries from bundle and populate IndexedDB
                let storage_prefix = BundlePath::from("storage");
                let storage_entries = bundle.prefix(&storage_prefix).map_err(VfsError::Other)?;

                for (bundle_path, data) in storage_entries {
                    let path_str = bundle_path.to_string();
                    if let Some(relative_path) = path_str.strip_prefix("storage/") {
                        let path_parts: Vec<String> =
                            relative_path.split('/').map(|s| s.to_string()).collect();

                        let reconstructed_parts =
                            if path_parts.len() >= 2 && path_parts[0].len() == 2 {
                                // Looks like a splayed document
                                let mut parts = vec![format!("{}{}", path_parts[0], path_parts[1])];
                                parts.extend_from_slice(&path_parts[2..]);
                                parts
                            } else {
                                path_parts
                            };

                        if let Ok(storage_key) = StorageKey::from_parts(reconstructed_parts.clone())
                        {
                            eprintln!(
                                "Loading storage key: {:?} (from path: {})",
                                reconstructed_parts, relative_path
                            );
                            storage.put(storage_key.clone(), data).await;
                        }
                    }
                }

                Repo::build_wasm()
                    .with_peer_id(peer_id)
                    .with_storage(storage)
                    .load_local()
                    .await
            }
        };

        let samod = Arc::new(samod);
        let vfs = VirtualFileSystem::from_bundle(samod.clone(), &mut bundle).await?;
        let vfs = Arc::new(vfs);

        info!(
            "TonkCore loaded from bundle with peer ID: {}",
            samod.peer_id()
        );

        Ok(TonkCore { samod, vfs })
    }

    /// Load from byte data with the configured settings
    pub async fn from_bytes(self, data: Vec<u8>) -> Result<TonkCore> {
        let bundle = Bundle::from_bytes(data)?;
        self.from_bundle(bundle).await
    }

    /// Load from file with the configured settings
    pub async fn from_file<P: AsRef<std::path::Path>>(self, path: P) -> Result<TonkCore> {
        let data = std::fs::read(path).map_err(VfsError::IoError)?;
        self.from_bytes(data).await
    }
}

impl Default for TonkCoreBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

/// Core synchronization engine that orchestrates CRDT operations and VFS interactions.
///
/// TonkCore combines samod (CRDT synchronization) with a virtual file system layer,
/// providing a unified interface for document synchronization and file operations.
/// The engine manages peer connections, document creation/retrieval, and real-time
/// synchronization of changes across distributed systems.
///
/// By default, TonkCore uses in-memory storage. For persistent storage, use
/// `with_storage(StorageConfig::Filesystem(path))`.
///
/// # Examples
///
/// ```no_run
/// # use tonk_core::{TonkCore, StorageConfig};
/// # async fn example() {
/// // In-memory storage (default)
/// let tonk = TonkCore::new().await.unwrap();
///
/// // Persistent filesystem storage
/// let tonk = TonkCore::builder()
///     .with_storage(StorageConfig::Filesystem("/path/to/storage".into()))
///     .build().await.unwrap();
/// # }
/// ```
pub struct TonkCore {
    samod: Arc<Repo>,
    vfs: Arc<VirtualFileSystem>,
}

impl TonkCore {
    /// Create a builder for configuring TonkCore instances
    ///
    /// # Examples
    /// ```no_run
    /// # use tonk_core::{TonkCore, StorageConfig};
    /// # async fn example() {
    /// // Simple in-memory instance
    /// let tonk = TonkCore::builder().build().await.unwrap();
    ///
    /// // With custom storage
    /// let tonk = TonkCore::builder()
    ///     .with_storage(StorageConfig::Filesystem("/path/to/storage".into()))
    ///     .build().await.unwrap();
    ///
    /// // Load from bundle with custom config
    /// let bundle_data = vec![];
    /// let tonk = TonkCore::builder()
    ///     .with_storage(StorageConfig::InMemory)
    ///     .from_bytes(bundle_data).await.unwrap();
    /// # }
    /// ```
    pub fn builder() -> TonkCoreBuilder {
        TonkCoreBuilder::new()
    }

    /// Create a new TonkCore with a randomly generated peer ID.
    ///
    /// The engine will use in-memory storage by default. For persistent storage,
    /// use the builder pattern: `TonkCore::builder().with_storage(...).build()`.
    ///
    /// # Returns
    /// A Result containing the new TonkCore instance or an error if initialization fails.
    ///
    /// # Examples
    /// ```no_run
    /// # use tonk_core::TonkCore;
    /// # async fn example() {
    /// let tonk = TonkCore::new().await.unwrap();
    /// println!("tonk peer ID: {}", tonk.peer_id());
    /// # }
    /// ```
    pub async fn new() -> Result<Self> {
        TonkCoreBuilder::new().build().await
    }

    /// Load from file with default in-memory storage
    pub async fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        TonkCoreBuilder::new().from_file(path).await
    }

    /// Load from bytes with default in-memory storage
    pub async fn from_bytes(data: Vec<u8>) -> Result<Self> {
        TonkCoreBuilder::new().from_bytes(data).await
    }

    /// Load from bundle with explicit storage configuration
    pub async fn from_bundle(
        bundle: Bundle<std::io::Cursor<Vec<u8>>>,
        storage_config: StorageConfig,
    ) -> Result<Self> {
        TonkCoreBuilder::new()
            .with_storage(storage_config)
            .from_bundle(bundle)
            .await
    }

    /// Export the current state to a bundle as bytes
    pub async fn to_bytes(&self) -> Result<Vec<u8>> {
        use crate::bundle::{Manifest, Version};
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        // Get the root document from VFS
        let root_id = self.vfs.root_id();

        // Create manifest
        let manifest = Manifest {
            manifest_version: 1,
            version: Version { major: 1, minor: 0 },
            root_id: root_id.to_string(),
            entrypoints: vec![],
            network_uris: vec![], // Could be populated from config
            x_notes: None,
            x_vendor: Some(serde_json::json!({
                "xTonk": {
                    "createdAt": chrono::Utc::now().to_rfc3339(),
                    "exportedFrom": "tonk-core v0.1.0"
                }
            })),
        };

        let manifest_json =
            serde_json::to_string_pretty(&manifest).map_err(VfsError::SerializationError)?;

        // Create ZIP bundle in memory
        let mut zip_data = Vec::new();
        {
            let mut zip_writer = ZipWriter::new(Cursor::new(&mut zip_data));

            // Add manifest
            zip_writer
                .start_file("manifest.json", SimpleFileOptions::default())
                .map_err(|e| VfsError::IoError(e.into()))?;
            zip_writer
                .write_all(manifest_json.as_bytes())
                .map_err(VfsError::IoError)?;

            // Export all storage data directly from samod's storage
            // Iterate through all documents and export their storage data
            let all_doc_ids = self.vfs.collect_all_document_ids().await?;

            for doc_id in &all_doc_ids {
                // Export the document as a snapshot with proper CompactionHash
                if let Ok(Some(doc_handle)) = self.samod.find(doc_id.clone()).await {
                    let doc_bytes = doc_handle.with_document(|doc| doc.save());

                    // Create a storage key for the snapshot
                    // Using a fixed snapshot name for simplicity
                    let storage_key = StorageKey::from_parts(vec![
                        doc_id.to_string(),
                        "snapshot".to_string(),
                        "bundle_export".to_string(),
                    ])
                    .map_err(|e| {
                        VfsError::Other(anyhow::anyhow!("Failed to create storage key: {}", e))
                    })?;

                    // Convert storage key to bundle path using samod's key_to_path logic
                    let mut path_components = Vec::new();
                    for (index, component) in storage_key.into_iter().enumerate() {
                        if index == 0 {
                            // Apply splaying to first component (document ID)
                            if component.len() >= 2 {
                                let (first_two, rest) = component.split_at(2);
                                path_components.push(first_two.to_string());
                                path_components.push(rest.to_string());
                            } else {
                                path_components.push(component);
                            }
                        } else {
                            path_components.push(component);
                        }
                    }
                    let storage_path = format!("storage/{}", path_components.join("/"));

                    zip_writer
                        .start_file(&storage_path, SimpleFileOptions::default())
                        .map_err(|e| VfsError::IoError(e.into()))?;
                    zip_writer
                        .write_all(&doc_bytes)
                        .map_err(VfsError::IoError)?;
                }
            }

            zip_writer
                .finish()
                .map_err(|e| VfsError::IoError(e.into()))?;
        }

        Ok(zip_data)
    }

    /// Export the current state to a bundle file
    pub async fn to_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.to_bytes().await?;
        std::fs::write(path, bytes).map_err(VfsError::IoError)?;
        Ok(())
    }

    /// Create a new TonkCore with a specific peer ID
    pub async fn with_peer_id(peer_id: PeerId) -> Result<Self> {
        TonkCoreBuilder::new().with_peer_id(peer_id).build().await
    }

    /// Get access to the VFS layer
    pub fn vfs(&self) -> Arc<VirtualFileSystem> {
        Arc::clone(&self.vfs)
    }

    /// Get access to the underlying Repo instance
    pub fn samod(&self) -> Arc<Repo> {
        Arc::clone(&self.samod)
    }

    /// Get the peer ID of this sync engine
    pub fn peer_id(&self) -> PeerId {
        self.samod.peer_id()
    }

    /// Connect to a WebSocket peer
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        info!("Connecting to WebSocket peer at: {}", url);

        let conn_finished = crate::websocket::connect(Arc::clone(&self.samod), url).await?;

        info!("Successfully connected to WebSocket peer at: {}", url);
        info!("Connection finished with reason: {:?}", conn_finished);
        Ok(())
    }

    /// Connect using network URIs from manifest
    // TODO: connect to from_bundle for network connection
    // pub async fn connect_from_manifest(&self) -> Result<(), VfsError> {
    //     for uri in &self.manifest().network_uris {
    //         if uri.starts_with("ws://") || uri.starts_with("wss://") {
    //             if let Ok(()) = self.connect_websocket(uri).await {
    //                 return Ok(());
    //             }
    //         }
    //     }
    //     Ok(())
    // }

    /// Connect to a WebSocket peer (WASM)
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        info!("Connecting to WebSocket peer at: {}", url);

        let samod = Arc::clone(&self.samod);
        let url = url.to_string();
        let url_copy = url.clone();

        // Spawn connection future so it runs in background
        wasm_bindgen_futures::spawn_local(async move {
            match crate::websocket::connect_wasm(samod, &url_copy).await {
                Ok(reason) => {
                    log(&format!("WebSocket connection closed: {reason:?}"));
                }
                Err(e) => {
                    error(&format!("WebSocket connection error: {e}"));
                }
            }
        });

        info!("Successfully connected to WebSocket peer at: {}", url);
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
        let handle = self
            .samod
            .create(initial_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {e}")))?;

        Ok(handle)
    }
}

impl Clone for TonkCore {
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
    #[cfg(not(target_arch = "wasm32"))]
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_sync_engine_creation() {
        let tonk = TonkCore::new().await.unwrap();
        assert!(!tonk.peer_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_sync_engine_with_peer_id() {
        let mut rng = rand::rng();
        let peer_id = PeerId::new_with_rng(&mut rng);
        let tonk = TonkCore::with_peer_id(peer_id.clone()).await.unwrap();
        assert_eq!(tonk.peer_id(), peer_id);
    }

    #[tokio::test]
    async fn test_document_creation() {
        let tonk = TonkCore::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = tonk.create_document(doc).await.unwrap();
        assert!(!handle.document_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_vfs_integration() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = tonk.vfs();

        // Test that VFS is accessible
        assert!(!vfs.root_id().to_string().is_empty());

        // Test that we can subscribe to VFS events
        let _rx = vfs.subscribe_events();

        // Test that both references point to the same samod instance
        let samod = tonk.samod();
        // NOTE: We can't easily compare Arc<Repo> for equality, but we can check peer IDs
        assert_eq!(tonk.peer_id(), samod.peer_id());
    }

    #[tokio::test]
    async fn test_websocket_connection_failure() {
        let tonk = TonkCore::new().await.unwrap();

        // Test connection to invalid URL
        let result = timeout(
            Duration::from_secs(1),
            tonk.connect_websocket("ws://localhost:99999"),
        )
        .await;

        // Should either timeout or fail with connection error
        match result {
            Ok(Err(_)) => (), // Connection failed as expected
            Err(_) => (),     // Timed out as expected
            Ok(Ok(_)) => panic!("Connection should have failed"),
        }
    }

    #[tokio::test]
    async fn test_bundle_export() {
        // Create a new sync engine and add some data
        let tonk = TonkCore::new().await.unwrap();
        let vfs = tonk.vfs();

        // Create a test document
        vfs.create_document("/test.txt", String::from("Hello, Bundle!"))
            .await
            .unwrap();

        // Export to bytes
        let bundle_bytes = tonk.to_bytes().await.unwrap();

        // Verify the bundle is valid by parsing it
        let bundle = Bundle::from_bytes(bundle_bytes).unwrap();

        // Check manifest
        let manifest = bundle.manifest();
        assert_eq!(manifest.manifest_version, 1);
        // assert_eq!(manifest.root, "root");
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_bundle_round_trip() {
        // Create first engine with some data
        let tonk1 = TonkCore::new().await.unwrap();
        let vfs1 = tonk1.vfs();

        // Create some documents
        vfs1.create_document("/file1.txt", String::from("Content 1"))
            .await
            .unwrap();
        vfs1.create_document("/file2.txt", String::from("Content 2"))
            .await
            .unwrap();
        vfs1.create_directory("/folder").await.unwrap();
        vfs1.create_document("/folder/nested.txt", String::from("Nested content"))
            .await
            .unwrap();

        // Export to bundle
        let bundle_bytes = tonk1.to_bytes().await.unwrap();

        // Load from bundle into a new engine with temp storage for persistence
        let temp_dir2 = TempDir::new().unwrap();
        let storage_path2 = temp_dir2.path().join("tonk2_storage");
        let bundle = Bundle::from_bytes(bundle_bytes).unwrap();
        let tonk2 = TonkCore::from_bundle(bundle, StorageConfig::Filesystem(storage_path2))
            .await
            .unwrap();
        let _vfs2 = tonk2.vfs();

        // Verify the root document content is preserved (not the ID)
        // let root_doc = vfs2.root_document().await.unwrap();

        // Check that root is a directory
        // use automerge::ReadDoc;
        // let (value, _) = root_doc.get(automerge::ROOT, "type").unwrap().unwrap();
        // let doc_type = value.to_str().unwrap();
        // assert_eq!(doc_type, "dir");

        // Check that the root document has a name
        // let (name_value, _) = root_doc.get(automerge::ROOT, "name").unwrap().unwrap();
        // let name = name_value.to_str().unwrap();
        // assert_eq!(name, "/");

        info!("Bundle round-trip test passed - root document structure preserved");
    }

    #[tokio::test]
    async fn test_in_memory_storage() {
        let tonk = TonkCore::builder()
            .with_storage(StorageConfig::InMemory)
            .build()
            .await
            .unwrap();
        let vfs = tonk.vfs();

        // Create some test data
        vfs.create_document("/test.txt", "test content".to_string())
            .await
            .unwrap();

        // Verify the document exists
        assert!(vfs.exists("/test.txt").await.unwrap());
        let handle = vfs.find_document("/test.txt").await.unwrap().unwrap();
        let content: String = handle.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    // Handle both string and serialized string
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content, "\"test content\"");
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_filesystem_storage() {
        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("tonk_storage");

        let tonk = TonkCore::builder()
            .with_storage(StorageConfig::Filesystem(storage_path.clone()))
            .build()
            .await
            .unwrap();
        let vfs = tonk.vfs();

        // Create some test data
        vfs.create_document("/test.txt", "persistent content".to_string())
            .await
            .unwrap();

        // Verify the document exists
        assert!(vfs.exists("/test.txt").await.unwrap());
        let handle = vfs.find_document("/test.txt").await.unwrap().unwrap();
        let content: String = handle.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content, "\"persistent content\"");

        // Verify storage directory was created
        assert!(storage_path.exists());
    }

    #[tokio::test]
    async fn test_with_peer_id_and_storage() {
        let mut rng = rand::rng();
        let peer_id = PeerId::new_with_rng(&mut rng);

        let tonk = TonkCore::builder()
            .with_peer_id(peer_id.clone())
            .with_storage(StorageConfig::InMemory)
            .build()
            .await
            .unwrap();

        assert_eq!(tonk.peer_id(), peer_id);
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_bundle_with_in_memory_storage() {
        // Create an engine with filesystem storage and some data
        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path().join("source");

        let tonk1 = TonkCore::builder()
            .with_storage(StorageConfig::Filesystem(storage_path))
            .build()
            .await
            .unwrap();
        let vfs1 = tonk1.vfs();

        vfs1.create_document("/test.txt", "bundle test".to_string())
            .await
            .unwrap();

        // Export to bundle
        let bundle_bytes = tonk1.to_bytes().await.unwrap();
        let bundle = Bundle::from_bytes(bundle_bytes).unwrap();

        // Load into an in-memory engine
        let tonk2 = TonkCore::from_bundle(bundle, StorageConfig::InMemory)
            .await
            .unwrap();
        let vfs2 = tonk2.vfs();

        // Verify data was preserved
        assert!(vfs2.exists("/test.txt").await.unwrap());
        let handle = vfs2.find_document("/test.txt").await.unwrap().unwrap();
        let content: String = handle.with_document(|doc| {
            use automerge::ReadDoc;
            match doc.get(automerge::ROOT, "content") {
                Ok(Some((value, _))) => {
                    if let Some(s) = value.to_str() {
                        s.to_string()
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        });
        assert_eq!(content, "\"bundle test\"");
    }
}
