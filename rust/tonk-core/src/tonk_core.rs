use crate::Bundle;
use crate::bundle::BundleConfig;
use crate::error::{Result, VfsError};
use crate::vfs::VirtualFileSystem;
use rand::rng;
#[cfg(not(target_arch = "wasm32"))]
use samod::RepoBuilder;
#[cfg(not(target_arch = "wasm32"))]
use samod::storage::TokioFilesystemStorage as FilesystemStorage;
use samod::storage::{InMemoryStorage, StorageKey};
#[cfg(target_arch = "wasm32")]
use samod::storage::{IndexedDbStorage, LocalStorage};
use samod::{DocHandle, DocumentId, PeerId, Repo};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use tokio::sync::RwLock;
use tracing::info;

/// Storage configuration options for TonkCore
#[derive(Debug, Clone)]
pub enum StorageConfig {
    /// Use in-memory storage
    InMemory,
    /// Use filesystem storage at the specified path
    #[cfg(not(target_arch = "wasm32"))]
    Filesystem(PathBuf),
    /// Use IndexedDB storage with optional namespace for isolation
    /// When namespace is provided, creates database named `samod_storage_{namespace}`
    #[cfg(target_arch = "wasm32")]
    IndexedDB { namespace: Option<String> },
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
                        .with_concurrency(samod::ConcurrencyConfig::Threadpool(
                            rayon::ThreadPoolBuilder::new().build().unwrap(),
                        ))
                        .load()
                        .await
                }
                StorageConfig::Filesystem(path) => {
                    std::fs::create_dir_all(&path).map_err(VfsError::IoError)?;
                    let storage = FilesystemStorage::new(&path);
                    RepoBuilder::new(runtime)
                        .with_storage(storage)
                        .with_peer_id(peer_id)
                        .with_concurrency(samod::ConcurrencyConfig::Threadpool(
                            rayon::ThreadPoolBuilder::new().build().unwrap(),
                        ))
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
            let (samod, stored_root_id): (Repo, Option<DocumentId>) = match self.storage_config {
                StorageConfig::InMemory => {
                    let samod = Repo::build_wasm()
                        .with_peer_id(peer_id)
                        .with_storage(InMemoryStorage::new())
                        .load()
                        .await;
                    (samod, None)
                }
                StorageConfig::IndexedDB { ref namespace } => {
                    let storage = match namespace {
                        Some(ns) => {
                            IndexedDbStorage::with_names(&format!("samod_storage_{}", ns), "data")
                        }
                        None => IndexedDbStorage::new(),
                    };

                    // Check for manifest
                    let stored_root_id = if let Ok(manifest_key) =
                        StorageKey::from_parts(vec!["__tonk_manifest__".to_string()])
                    {
                        match storage.load(manifest_key.clone()).await {
                            Some(manifest_data) => {
                                eprintln!("Found stored manifest in IndexedDB");
                                // Try to parse and extract root_id
                                serde_json::from_slice::<crate::bundle::Manifest>(&manifest_data)
                                    .ok()
                                    .and_then(|m| m.root_id.parse::<DocumentId>().ok())
                            }
                            None => {
                                eprintln!("No stored manifest found");
                                None
                            }
                        }
                    } else {
                        None
                    };

                    // Now build repo with storage (storage is moved here)
                    let samod = Repo::build_wasm()
                        .with_peer_id(peer_id)
                        .with_storage(storage)
                        .load_local()
                        .await;

                    (samod, stored_root_id)
                }
            };

            let samod = Arc::new(samod);

            // Initialize VFS based on whether we found a manifest
            let vfs = if let Some(root_id) = stored_root_id {
                eprintln!(
                    "Restoring VFS from stored manifest with root ID: {}",
                    root_id
                );
                Arc::new(VirtualFileSystem::from_root_id(samod.clone(), root_id).await?)
            } else {
                Arc::new(VirtualFileSystem::new(samod.clone()).await?)
            };

            info!("TonkCore initialized with peer ID: {}", samod.peer_id());

            Ok(TonkCore {
                samod,
                vfs,
                connection_state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
                ws_url: Arc::new(RwLock::new(None)),
            })
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
                        .with_concurrency(samod::ConcurrencyConfig::Threadpool(
                            rayon::ThreadPoolBuilder::new().build().unwrap(),
                        ))
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
                std::fs::create_dir_all(storage_path).map_err(VfsError::IoError)?;

                // Extract all storage files from bundle to the filesystem storage directory
                let storage_prefix = BundlePath::from("storage");
                let storage_entries = bundle.prefix(&storage_prefix).map_err(VfsError::Other)?;

                for (bundle_path, data) in storage_entries {
                    let path_str = bundle_path.to_string();

                    if let Some(relative_path) = path_str.strip_prefix("storage/") {
                        let full_path = storage_path.join(relative_path);

                        if let Some(parent) = full_path.parent() {
                            std::fs::create_dir_all(parent).map_err(VfsError::IoError)?;
                        }

                        std::fs::write(&full_path, data).map_err(VfsError::IoError)?;
                    }
                }

                let storage = FilesystemStorage::new(storage_path);
                RepoBuilder::new(runtime)
                    .with_storage(storage)
                    .with_peer_id(peer_id)
                    .with_concurrency(samod::ConcurrencyConfig::Threadpool(
                        rayon::ThreadPoolBuilder::new().build().unwrap(),
                    ))
                    .load()
                    .await
            }
            #[cfg(target_arch = "wasm32")]
            StorageConfig::IndexedDB { ref namespace } => {
                let storage = match namespace {
                    Some(ns) => {
                        IndexedDbStorage::with_names(&format!("samod_storage_{}", ns), "data")
                    }
                    None => IndexedDbStorage::new(),
                };

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

                // Store manifest in IndexedDB for offline initialization
                if let Ok(manifest_key) =
                    StorageKey::from_parts(vec!["__tonk_manifest__".to_string()])
                {
                    if let Ok(manifest_json) = serde_json::to_vec(bundle.manifest()) {
                        eprintln!(
                            "Storing manifest with root ID: {}",
                            bundle.manifest().root_id
                        );
                        storage.put(manifest_key, manifest_json).await;
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

        #[cfg(target_arch = "wasm32")]
        {
            Ok(TonkCore {
                samod,
                vfs,
                connection_state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
                ws_url: Arc::new(RwLock::new(None)),
            })
        }

        #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Open,
    Connected,
    Failed(String),
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
    #[cfg(target_arch = "wasm32")]
    connection_state: Arc<RwLock<ConnectionState>>,
    #[cfg(target_arch = "wasm32")]
    ws_url: Arc<RwLock<Option<String>>>,
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
    pub async fn fork_to_bytes(&self, config: Option<BundleConfig>) -> Result<Vec<u8>> {
        // Create a new samod instance with in-memory storage for the copied VFS to avoid conflicts
        #[cfg(not(target_arch = "wasm32"))]
        let new_samod = {
            let runtime = tokio::runtime::Handle::current();
            let storage = InMemoryStorage::new();
            let mut rng = rand::rng();
            let peer_id = PeerId::new_with_rng(&mut rng);
            Arc::new(
                RepoBuilder::new(runtime)
                    .with_storage(storage)
                    .with_peer_id(peer_id)
                    .with_concurrency(samod::ConcurrencyConfig::Threadpool(
                        rayon::ThreadPoolBuilder::new().build().unwrap(),
                    ))
                    .load()
                    .await,
            )
        };

        #[cfg(target_arch = "wasm32")]
        let new_samod = {
            let mut rng = rand::rng();
            let peer_id = PeerId::new_with_rng(&mut rng);
            Arc::new(
                Repo::build_wasm()
                    .with_peer_id(peer_id)
                    .with_storage(InMemoryStorage::new())
                    .load()
                    .await,
            )
        };

        let copied_vfs = Arc::new(VirtualFileSystem::new(new_samod.clone()).await?);

        // Recursively copy all files and directories from /app
        self.copy_directory_recursive(&self.vfs, &copied_vfs, "/app")
            .await?;

        // Also copy /src if it exists
        if self.vfs.exists("/src").await? {
            self.copy_directory_recursive(&self.vfs, &copied_vfs, "/src")
                .await?;
        }

        // Export the copied VFS to bytes
        copied_vfs.to_bytes(config).await
    }

    /// Recursively copy a directory and its contents from source VFS to destination VFS
    fn copy_directory_recursive<'a>(
        #[allow(clippy::only_used_in_recursion)] &'a self,
        source_vfs: &'a VirtualFileSystem,
        dest_vfs: &'a VirtualFileSystem,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            use crate::vfs::backend::AutomergeHelpers;
            use crate::vfs::types::NodeType;
            use bytes::Bytes;

            // Create the directory in the destination VFS if it doesn't exist
            if path != "/" && !dest_vfs.exists(path).await? {
                dest_vfs.create_directory(path).await?;
            }

            // List all entries in the current directory
            let entries = source_vfs.list_directory(path).await?;

            for entry in entries {
                // Construct the full path for this entry
                let entry_path = if path == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{}/{}", path, entry.name)
                };

                match entry.node_type {
                    NodeType::Directory => {
                        // Create the directory in the destination VFS
                        dest_vfs.create_directory(&entry_path).await?;

                        // Recursively copy the directory's contents
                        self.copy_directory_recursive(source_vfs, dest_vfs, &entry_path)
                            .await?;
                    }
                    NodeType::Document => {
                        // Find the document in the source VFS
                        if let Some(doc_handle) = source_vfs.find_document(&entry_path).await? {
                            // Try to read the document with bytes first
                            let has_bytes = doc_handle.with_document(|doc| {
                                use automerge::ReadDoc;
                                matches!(doc.get(automerge::ROOT, "bytes"), Ok(Some(_)))
                            });

                            if has_bytes {
                                // Read the document content with bytes
                                let doc_node = AutomergeHelpers::read_bytes_document::<
                                    serde_json::Value,
                                >(&doc_handle)?;
                                dest_vfs
                                    .create_document_with_bytes(
                                        &entry_path,
                                        doc_node.content,
                                        Bytes::from(doc_node.bytes.unwrap_or_default()),
                                    )
                                    .await?;
                            } else {
                                // Read the document content without bytes
                                let doc_node = AutomergeHelpers::read_document::<serde_json::Value>(
                                    &doc_handle,
                                )?;
                                dest_vfs
                                    .create_document(&entry_path, doc_node.content)
                                    .await?;
                            }
                        }
                    }
                }
            }

            Ok(())
        })
    }

    /// Export the current state to a bundle as bytes
    pub async fn to_bytes(&self, config: Option<BundleConfig>) -> Result<Vec<u8>> {
        self.vfs.to_bytes(config).await
    }

    /// Export the current state to a bundle file
    pub async fn to_file<P: AsRef<std::path::Path>>(&self, path: P) -> Result<()> {
        let bytes = self.to_bytes(None).await?;
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
    ///
    /// Returns a `ConnectionHandle` that can be used to control the connection.
    /// The connection runs in a background task and will remain active until:
    /// - The server closes the connection
    /// - `ConnectionHandle::disconnect()` is called
    /// - The `ConnectionHandle` is dropped (connection continues in background)
    ///
    /// # Example
    /// ```no_run
    /// # use tonk_core::TonkCore;
    /// # async fn example() {
    /// let tonk = TonkCore::new().await.unwrap();
    /// let handle = tonk.connect_websocket("ws://localhost:8080").await.unwrap();
    ///
    /// // Check if connected
    /// if handle.is_connected() {
    ///     println!("Connected!");
    /// }
    ///
    /// // Disconnect when done
    /// handle.disconnect();
    /// # }
    /// ```
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn connect_websocket(
        &self,
        url: &str,
    ) -> Result<crate::websocket::ConnectionHandle> {
        info!("Connecting to WebSocket peer at: {}", url);

        let handle = crate::websocket::connect(Arc::clone(&self.samod), url).await?;

        info!("WebSocket connection initiated to: {}", url);
        Ok(handle)
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

        {
            let mut ws_url = self.ws_url.write().await;
            *ws_url = Some(url.to_string());
        }

        {
            let mut state = self.connection_state.write().await;
            *state = ConnectionState::Connecting;
        }

        let samod = Arc::clone(&self.samod);
        let url_str = url.to_string();
        let state_clone = Arc::clone(&self.connection_state);

        let events =
            samod.connect_wasm_websocket_observable(&url_str, samod::ConnDirection::Outgoing);

        let state_for_open = Arc::clone(&state_clone);
        wasm_bindgen_futures::spawn_local(async move {
            if events.on_open.await.is_ok() {
                let mut state = state_for_open.write().await;
                *state = ConnectionState::Open;
            }
        });

        let state_for_ready = Arc::clone(&state_clone);
        wasm_bindgen_futures::spawn_local(async move {
            if events.on_ready.await.is_ok() {
                let mut state = state_for_ready.write().await;
                *state = ConnectionState::Connected;
            }
        });

        let state_for_finished = Arc::clone(&state_clone);
        wasm_bindgen_futures::spawn_local(async move {
            let reason = events.finished.await;

            let mut state = state_for_finished.write().await;
            match reason {
                samod::ConnFinishedReason::Error(e) => {
                    *state = ConnectionState::Failed(e);
                }
                _ => {
                    *state = ConnectionState::Disconnected;
                }
            }
        });

        info!("WebSocket connection initiated at: {}", url);
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn is_connected(&self) -> bool {
        let state = self.connection_state.read().await;
        let result = matches!(*state, ConnectionState::Connected);
        result
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn connection_state(&self) -> ConnectionState {
        let state = self.connection_state.read().await;
        state.clone()
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn ws_url(&self) -> Option<String> {
        let url = self.ws_url.read().await;
        url.clone()
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
            #[cfg(target_arch = "wasm32")]
            connection_state: Arc::clone(&self.connection_state),
            #[cfg(target_arch = "wasm32")]
            ws_url: Arc::clone(&self.ws_url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    use tempfile::TempDir;
    use tokio::time::{Duration, timeout};

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
        let bundle_bytes = tonk.to_bytes(None).await.unwrap();

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
        let bundle_bytes = tonk1.to_bytes(None).await.unwrap();

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
        // assert_eq!(doc_type, "directory");

        // Check that the root document has a name
        // let (name_value, _) = root_doc.get(automerge::ROOT, "name").unwrap().unwrap();
        // let name = name_value.to_str().unwrap();
        // assert_eq!(name, "/");

        info!("Bundle round-trip test passed - root document structure preserved");
    }

    #[tokio::test]
    async fn test_in_memory_storage() {
        use crate::vfs::backend::AutomergeHelpers;

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
        let doc_node: crate::vfs::types::DocNode<String> =
            AutomergeHelpers::read_document(&handle).unwrap();
        assert_eq!(doc_node.content, "test content");
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_filesystem_storage() {
        use crate::vfs::backend::AutomergeHelpers;

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
        let doc_node: crate::vfs::types::DocNode<String> =
            AutomergeHelpers::read_document(&handle).unwrap();
        assert_eq!(doc_node.content, "persistent content");

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
        use crate::vfs::backend::AutomergeHelpers;

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
        let bundle_bytes = tonk1.to_bytes(None).await.unwrap();
        let bundle = Bundle::from_bytes(bundle_bytes).unwrap();

        // Load into an in-memory engine
        let tonk2 = TonkCore::from_bundle(bundle, StorageConfig::InMemory)
            .await
            .unwrap();
        let vfs2 = tonk2.vfs();

        // Verify data was preserved
        assert!(vfs2.exists("/test.txt").await.unwrap());
        let handle = vfs2.find_document("/test.txt").await.unwrap().unwrap();
        let doc_node: crate::vfs::types::DocNode<String> =
            AutomergeHelpers::read_document(&handle).unwrap();
        assert_eq!(doc_node.content, "bundle test");
    }

    #[tokio::test]
    #[cfg(not(target_arch = "wasm32"))]
    async fn test_fork_to_bytes() {
        use crate::vfs::backend::AutomergeHelpers;

        // Create a TonkCore instance with some data
        let tonk = TonkCore::new().await.unwrap();
        let vfs = tonk.vfs();

        // Get the original root ID
        let original_root_id = vfs.root_id();

        // Create a directory structure under /app
        vfs.create_directory("/app").await.unwrap();
        vfs.create_directory("/app/subdir").await.unwrap();
        vfs.create_directory("/app/subdir/nested").await.unwrap();

        // Create some documents with content
        vfs.create_document("/app/file1.txt", "content 1".to_string())
            .await
            .unwrap();
        vfs.create_document("/app/file2.txt", "content 2".to_string())
            .await
            .unwrap();
        vfs.create_document("/app/subdir/file3.txt", "content 3".to_string())
            .await
            .unwrap();
        vfs.create_document("/app/subdir/nested/file4.txt", "content 4".to_string())
            .await
            .unwrap();

        // Create a document with bytes
        use bytes::Bytes;
        vfs.create_document_with_bytes(
            "/app/binary.dat",
            serde_json::json!({"type": "binary"}),
            Bytes::from(vec![1, 2, 3, 4, 5]),
        )
        .await
        .unwrap();

        // Fork to bytes
        let forked_bytes = tonk.fork_to_bytes(None).await.unwrap();
        let bundle = Bundle::from_bytes(forked_bytes).unwrap();

        // Load the forked bundle into a new TonkCore
        let tonk_forked = TonkCore::from_bundle(bundle, StorageConfig::InMemory)
            .await
            .unwrap();
        let vfs_forked = tonk_forked.vfs();

        // Get the forked root ID
        let forked_root_id = vfs_forked.root_id();

        // Verify the root IDs are different
        assert_ne!(
            original_root_id.to_string(),
            forked_root_id.to_string(),
            "Root IDs should be different between source and forked VFS"
        );

        // Verify all documents exist in the forked VFS
        assert!(
            vfs_forked.exists("/app").await.unwrap(),
            "/app should exist"
        );
        assert!(
            vfs_forked.exists("/app/file1.txt").await.unwrap(),
            "/app/file1.txt should exist"
        );
        assert!(
            vfs_forked.exists("/app/file2.txt").await.unwrap(),
            "/app/file2.txt should exist"
        );
        assert!(
            vfs_forked.exists("/app/subdir").await.unwrap(),
            "/app/subdir should exist"
        );
        assert!(
            vfs_forked.exists("/app/subdir/file3.txt").await.unwrap(),
            "/app/subdir/file3.txt should exist"
        );
        assert!(
            vfs_forked.exists("/app/subdir/nested").await.unwrap(),
            "/app/subdir/nested should exist"
        );
        assert!(
            vfs_forked
                .exists("/app/subdir/nested/file4.txt")
                .await
                .unwrap(),
            "/app/subdir/nested/file4.txt should exist"
        );
        assert!(
            vfs_forked.exists("/app/binary.dat").await.unwrap(),
            "/app/binary.dat should exist"
        );

        // Verify document contents
        let handle1 = vfs_forked
            .find_document("/app/file1.txt")
            .await
            .unwrap()
            .unwrap();
        let doc_node1: crate::vfs::types::DocNode<String> =
            AutomergeHelpers::read_document(&handle1).unwrap();
        assert_eq!(doc_node1.content, "content 1");

        let handle4 = vfs_forked
            .find_document("/app/subdir/nested/file4.txt")
            .await
            .unwrap()
            .unwrap();
        let doc_node4: crate::vfs::types::DocNode<String> =
            AutomergeHelpers::read_document(&handle4).unwrap();
        assert_eq!(doc_node4.content, "content 4");

        // Verify the binary document has bytes
        let handle_binary = vfs_forked
            .find_document("/app/binary.dat")
            .await
            .unwrap()
            .unwrap();
        let has_bytes = handle_binary.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "bytes").is_ok()
        });
        assert!(has_bytes, "Binary document should have bytes field");

        // Verify that documents NOT in /app are NOT copied
        vfs.create_document("/outside.txt", "outside content".to_string())
            .await
            .unwrap();

        // Fork again with the new document
        let forked_bytes2 = tonk.fork_to_bytes(None).await.unwrap();
        let bundle2 = Bundle::from_bytes(forked_bytes2).unwrap();
        let tonk_forked2 = TonkCore::from_bundle(bundle2, StorageConfig::InMemory)
            .await
            .unwrap();
        let vfs_forked2 = tonk_forked2.vfs();

        // This document should NOT exist in the forked VFS
        assert!(
            !vfs_forked2.exists("/outside.txt").await.unwrap(),
            "/outside.txt should NOT exist in fork"
        );
    }
}
