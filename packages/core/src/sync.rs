use crate::error::{Result, VfsError};
use crate::vfs::VirtualFileSystem;
use rand::rng;
use samod::storage::InMemoryStorage;
#[cfg(not(target_arch = "wasm32"))]
use samod::RepoBuilder;
use samod::{DocHandle, DocumentId, PeerId, Repo};
use std::sync::Arc;
use tracing::info;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

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
    samod: Arc<Repo>,
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
        let peer_id = PeerId::new_with_rng(&mut rng);
        Self::with_peer_id(peer_id).await
    }

    /// Create a new SyncEngine with a specific peer ID
    pub async fn with_peer_id(peer_id: PeerId) -> Result<Self> {
        // Use samod's built-in InMemoryStorage
        // TODO: add option for FilesystemStorage
        let storage = InMemoryStorage::new();

        // Build samod instance with the storage and peer ID
        #[cfg(not(target_arch = "wasm32"))]
        let samod = {
            let runtime = tokio::runtime::Handle::current();
            RepoBuilder::new(runtime)
                .with_storage(storage)
                .with_peer_id(peer_id)
                .with_threadpool(None)
                .load()
                .await
        };

        #[cfg(target_arch = "wasm32")]
        let samod = {
            let builder = Repo::build_wasm();
            let builder = builder.with_storage(storage);
            let builder = builder.with_peer_id(peer_id);
            let result = builder.load().await;
            result
        };

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

        let conn_finished =
            crate::websocket::native_impl::connect_websocket_to_samod(Arc::clone(&self.samod), url)
                .await?;

        info!("Successfully connected to WebSocket peer at: {}", url);
        info!("Connection finished with reason: {:?}", conn_finished);
        Ok(())
    }

    /// Connect to a WebSocket peer (WASM) and adopt server's root document
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_websocket(&self, url: &str) -> Result<()> {
        info!("Connecting to WebSocket peer at: {}", url);

        #[cfg(target_arch = "wasm32")]
        {
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn log(s: &str);
            }
        }

        crate::websocket::wasm_impl::connect_websocket_to_samod(Arc::clone(&self.samod), url)
            .await?;

        info!("Successfully connected to WebSocket peer at: {}", url);
        Ok(())
    }

    /// Connect to WebSocket and adopt server's root filesystem document
    #[cfg(target_arch = "wasm32")]
    pub async fn connect_websocket_with_server_root(&mut self, url: &str) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn log(s: &str);
            }
        }

        // First establish WebSocket connection
        crate::websocket::wasm_impl::connect_websocket_to_samod(Arc::clone(&self.samod), url)
            .await?;

        // Extract server hostname and port from WebSocket URL
        let http_url = if url.starts_with("ws://") {
            url.replace("ws://", "http://")
        } else if url.starts_with("wss://") {
            url.replace("wss://", "https://")
        } else {
            return Err(VfsError::WebSocketError(
                "Invalid WebSocket URL format".to_string(),
            ));
        };

        // Fetch server's root document ID
        let root_response = web_sys::window()
            .unwrap()
            .fetch_with_str(&format!("{}/root", http_url));

        let response = wasm_bindgen_futures::JsFuture::from(root_response)
            .await
            .map_err(|_| {
                VfsError::WebSocketError("Failed to fetch server root document ID".to_string())
            })?;

        let response: web_sys::Response = response.dyn_into().unwrap();
        let json_promise = response.json().unwrap();
        let json_value = wasm_bindgen_futures::JsFuture::from(json_promise)
            .await
            .map_err(|_| {
                VfsError::WebSocketError("Failed to parse server root response".to_string())
            })?;

        // Extract root document ID from JSON response
        let root_id_str = js_sys::Reflect::get(&json_value, &"rootDocumentId".into())
            .unwrap()
            .as_string()
            .ok_or_else(|| {
                VfsError::WebSocketError("Invalid root document ID in response".to_string())
            })?;

        let root_id: DocumentId = root_id_str.parse().map_err(|_| {
            VfsError::WebSocketError("Failed to parse root document ID".to_string())
        })?;

        // Request the root document from the server and wait for it to sync
        match self.samod.find(root_id.clone()).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                // Poll for the document with increasing delays
                let mut attempts = 0;
                let max_attempts = 20; // 10 seconds total (500ms * 20)

                loop {
                    // Wait before each attempt
                    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(
                        &mut |resolve, _| {
                            web_sys::window()
                                .unwrap()
                                .set_timeout_with_callback_and_timeout_and_arguments_0(
                                    &resolve, 500,
                                )
                                .unwrap();
                        },
                    ))
                    .await
                    .unwrap();

                    // Check if document is now available
                    if let Ok(Some(_)) = self.samod.find(root_id.clone()).await {
                        break;
                    }

                    attempts += 1;
                    if attempts >= max_attempts {
                        return Err(VfsError::WebSocketError(format!(
                            "Root document {} not available after waiting 10 seconds. Server may not be sharing the document properly.", 
                            root_id_str
                        )));
                    }
                }
            }
            Err(e) => {
                return Err(VfsError::SamodError(format!(
                    "Error checking for root document: {}",
                    e
                )));
            }
        }

        // Replace current VFS with one using server's root document
        let new_vfs =
            Arc::new(VirtualFileSystem::from_root(Arc::clone(&self.samod), root_id).await?);
        self.vfs = new_vfs;

        info!("Successfully connected to WebSocket and adopted server root");
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
        // NOTE: We can't easily compare Arc<Repo> for equality, but we can check peer IDs
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
