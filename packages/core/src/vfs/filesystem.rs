use crate::bundle::{BundleConfig, RandomAccess};
use crate::error::{Result, VfsError};
use crate::vfs::backend::AutomergeHelpers;
use crate::vfs::path_index::PathIndex;
use crate::vfs::types::*;
use crate::vfs::watcher::DocumentWatcher;
use crate::Bundle;
use automerge::transaction::Transactable;
use automerge::Automerge;
use bytes::Bytes;
use samod::storage::StorageKey;
use samod::{DocHandle, DocumentId, Repo};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct VirtualFileSystem {
    samod: Arc<Repo>,
    root_id: DocumentId,
    event_tx: broadcast::Sender<VfsEvent>,
}

#[derive(Debug, Clone)]
pub enum VfsEvent {
    DocumentCreated { path: String, doc_id: DocumentId },
    DocumentUpdated { path: String, doc_id: DocumentId },
    DocumentDeleted { path: String },
    DirectoryCreated { path: String, doc_id: DocumentId },
}

impl VirtualFileSystem {
    pub async fn new(samod: Arc<Repo>) -> Result<Self> {
        // Create the path index document
        let index_doc = Automerge::new();
        let index_handle = samod
            .create(index_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create path index: {e}")))?;

        let root_id = index_handle.document_id().clone();
        let (event_tx, _) = broadcast::channel(100);

        let vfs = Self {
            samod,
            root_id,
            event_tx,
        };

        // Initialize empty path index
        let empty_index = PathIndex::new();
        vfs.write_path_index(&empty_index).await?;

        Ok(vfs)
    }

    /// Create a new VFS from a bundle
    pub async fn from_bundle<R: RandomAccess>(
        samod: Arc<Repo>,
        bundle: &mut Bundle<R>,
    ) -> Result<Self> {
        let root_id_str = bundle.manifest().root_id.clone();
        let root_id = root_id_str
            .parse::<DocumentId>()
            .map_err(|e| VfsError::Other(anyhow::anyhow!("Failed to parse root ID: {}", e)))?;

        let (event_tx, _) = broadcast::channel(100);

        Ok(Self {
            samod,
            root_id,
            event_tx,
        })
    }

    /// Create a new VFS from a root document ID
    /// Used when restoring from local storage where manifest is already persisted
    pub async fn from_root_id(samod: Arc<Repo>, root_id: DocumentId) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(100);

        Ok(Self {
            samod,
            root_id,
            event_tx,
        })
    }

    /// Read the path index from the root document
    async fn read_path_index(&self) -> Result<PathIndex> {
        let handle = self
            .samod
            .find(self.root_id.clone())
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find path index: {e}")))?
            .ok_or_else(|| VfsError::Other(anyhow::anyhow!("Path index not found")))?;

        handle.with_document(|doc| {
            use automerge::ReadDoc;
            // Read the JSON string and deserialize
            let index_json = doc
                .get(automerge::ROOT, "path_index")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| AutomergeHelpers::extract_string_value(&value))
                .ok_or_else(|| VfsError::Other(anyhow::anyhow!("Path index data not found")))?;

            serde_json::from_str(&index_json).map_err(VfsError::SerializationError)
        })
    }

    /// Write the path index to the root document
    async fn write_path_index(&self, index: &PathIndex) -> Result<()> {
        let handle = self
            .samod
            .find(self.root_id.clone()) // root_id points to path index!
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find path index: {e}")))?
            .ok_or_else(|| VfsError::Other(anyhow::anyhow!("Path index not found")))?;

        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            // Serialize to JSON string like other VFS documents
            let index_json = serde_json::to_string(index).map_err(VfsError::SerializationError)?;
            tx.put(automerge::ROOT, "path_index", index_json)?;
            tx.commit();
            Ok(())
        })
    }

    /// Update path index atomically with a closure
    async fn update_path_index<F, R>(&self, update_fn: F) -> Result<R>
    where
        F: FnOnce(&mut PathIndex) -> R,
    {
        let mut index = self.read_path_index().await?;
        let result = update_fn(&mut index);
        self.write_path_index(&index).await?;
        Ok(result)
    }

    /// Create parent directories for a path if they don't exist
    fn ensure_parent_directories<'a>(
        &'a self,
        path: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Extract parent path
            let parent_path = if path.contains('/') {
                let last_slash = path.rfind('/').unwrap();
                if last_slash == 0 {
                    return Ok(()); // Parent is root, no need to create
                }
                &path[..last_slash]
            } else {
                return Ok(()); // No parent needed
            };

            // Check if parent exists
            if self.exists(parent_path).await? {
                return Ok(());
            }

            // Recursively create grandparents first
            self.ensure_parent_directories(parent_path).await?;

            // Create this parent directory
            self.create_directory(parent_path).await?;

            Ok(())
        })
    }

    /// Add a child to its parent directory
    async fn add_to_parent(
        &self,
        path: &str,
        doc_id: DocumentId,
        node_type: NodeType,
    ) -> Result<()> {
        if path == "/" {
            return Ok(());
        }

        let parent_path = if let Some(last_slash) = path.rfind('/') {
            if last_slash == 0 {
                "/"
            } else {
                &path[..last_slash]
            }
        } else {
            return Ok(());
        };

        let parent_handle = if parent_path == "/" {
            self.samod
                .find(self.root_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find root: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(self.root_id.to_string()))?
        } else {
            let index = self.read_path_index().await?;
            if let Some(entry) = index.get_entry(parent_path) {
                let pid = entry
                    .doc_id
                    .parse::<DocumentId>()
                    .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid doc id: {}", e)))?;
                self.samod
                    .find(pid)
                    .await
                    .map_err(|e| VfsError::SamodError(format!("Failed to find parent: {e}")))?
                    .ok_or_else(|| VfsError::DocumentNotFound(parent_path.to_string()))?
            } else {
                return Err(VfsError::DocumentNotFound(parent_path.to_string()));
            }
        };

        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        let now = chrono::Utc::now();

        let ref_node = RefNode {
            pointer: doc_id,
            node_type,
            timestamps: Timestamps {
                created: now,
                modified: now,
            },
            name,
        };

        AutomergeHelpers::add_child_to_directory(&parent_handle, &ref_node)?;
        Ok(())
    }

    /// Remove a child from its parent directory
    async fn remove_from_parent(&self, path: &str) -> Result<()> {
        if path == "/" {
            return Ok(());
        }

        let parent_path = if let Some(last_slash) = path.rfind('/') {
            if last_slash == 0 {
                "/"
            } else {
                &path[..last_slash]
            }
        } else {
            return Ok(());
        };

        let parent_handle = if parent_path == "/" {
            self.samod
                .find(self.root_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find root: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(self.root_id.to_string()))?
        } else {
            let index = self.read_path_index().await?;
            if let Some(entry) = index.get_entry(parent_path) {
                let pid = entry
                    .doc_id
                    .parse::<DocumentId>()
                    .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid doc id: {}", e)))?;
                self.samod
                    .find(pid)
                    .await
                    .map_err(|e| VfsError::SamodError(format!("Failed to find parent: {e}")))?
                    .ok_or_else(|| VfsError::DocumentNotFound(parent_path.to_string()))?
            } else {
                // Parent gone, ignore
                return Ok(());
            }
        };

        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        AutomergeHelpers::remove_child_from_directory(&parent_handle, &name)?;
        Ok(())
    }

    pub async fn to_bytes(&self, config: Option<BundleConfig>) -> Result<Vec<u8>> {
        use crate::bundle::{Manifest, Version};
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        // Get the root document from VFS
        let root_id = self.root_id();

        // Extract config values or use defaults
        let config = config.unwrap_or_default();

        // Merge vendor metadata with default Tonk metadata
        let vendor_metadata = match config.vendor_metadata {
            Some(mut custom) => {
                // Merge custom metadata with default xTonk metadata
                if let Some(obj) = custom.as_object_mut() {
                    obj.insert(
                        "xTonk".to_string(),
                        serde_json::json!({
                            "createdAt": chrono::Utc::now().to_rfc3339(),
                            "exportedFrom": "tonk-core v0.1.0"
                        }),
                    );
                }
                Some(custom)
            }
            None => Some(serde_json::json!({
                "xTonk": {
                    "createdAt": chrono::Utc::now().to_rfc3339(),
                    "exportedFrom": "tonk-core v0.1.0"
                }
            })),
        };

        // Create manifest
        let manifest = Manifest {
            manifest_version: 1,
            version: Version { major: 1, minor: 0 },
            root_id: root_id.to_string(),
            entrypoints: config.entrypoints,
            network_uris: config.network_uris,
            x_notes: config.notes,
            x_vendor: vendor_metadata,
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
            let all_doc_ids = self.collect_all_document_ids().await?;

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

    /// Get the root document ID
    pub fn root_id(&self) -> DocumentId {
        self.root_id.clone()
    }

    /// Get the root document
    pub async fn root_document(&self) -> Result<Automerge> {
        let root_handle = self
            .samod
            .find(self.root_id.clone())
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find root document: {e}")))?
            .ok_or_else(|| VfsError::DocumentNotFound(self.root_id.to_string()))?;

        let doc = root_handle.with_document(|doc| doc.fork());
        Ok(doc)
    }

    /// Subscribe to VFS events
    pub fn subscribe_events(&self) -> broadcast::Receiver<VfsEvent> {
        self.event_tx.subscribe()
    }

    /// Create a document at the specified path
    pub async fn create_document<T>(&self, path: &str, content: T) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        self.create_document_inner(path, content, Bytes::new(), false)
            .await
    }

    /// Create a document at the specified path using bytes
    pub async fn create_document_with_bytes<T>(
        &self,
        path: &str,
        content: T,
        bytes: Bytes,
    ) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        self.create_document_inner(path, content, bytes, true).await
    }

    /// Create a document at the specified path
    async fn create_document_inner<T>(
        &self,
        path: &str,
        content: T,
        bytes: Bytes,
        use_bytes: bool,
    ) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Ensure parent directories exist
        self.ensure_parent_directories(path).await?;

        // Check if already exists
        let index = self.read_path_index().await?;
        if index.has_path(path) {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Create the document in Samod
        let new_doc = Automerge::new();
        let doc_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {e}")))?;

        // Initialize document content (extract filename for internal name)
        let filename = path.rsplit('/').next().unwrap_or(path);
        if use_bytes {
            AutomergeHelpers::init_as_document_with_bytes(&doc_handle, filename, content, bytes)?;
        } else {
            AutomergeHelpers::init_as_document(&doc_handle, filename, content)?;
        }

        // Update path index
        let doc_id = doc_handle.document_id().clone();
        self.update_path_index(|index| {
            index.set_path(path.to_string(), doc_id.to_string(), NodeType::Document);
        })
        .await?;

        // Add to parent directory
        self.add_to_parent(path, doc_id.clone(), NodeType::Document)
            .await?;

        // Emit event
        let _ = self.event_tx.send(VfsEvent::DocumentCreated {
            path: path.to_string(),
            doc_id: doc_handle.document_id().clone(),
        });

        Ok(doc_handle)
    }

    /// Update a document at the specified path
    pub async fn update_document<T>(&self, path: &str, content: T) -> Result<bool>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        self.update_document_inner(path, content, Bytes::new(), false)
            .await
    }

    /// Update a document at the specified path using bytes
    pub async fn update_document_with_bytes<T>(
        &self,
        path: &str,
        content: T,
        bytes: Bytes,
    ) -> Result<bool>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        self.update_document_inner(path, content, bytes, true).await
    }

    /// Update an existing document at the specified path
    async fn update_document_inner<T>(
        &self,
        path: &str,
        content: T,
        bytes: Bytes,
        use_bytes: bool,
    ) -> Result<bool>
    where
        T: serde::Serialize + Send + 'static,
    {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Find the existing document
        match self.find_document(path).await? {
            Some(doc_handle) => {
                // Update content
                if use_bytes {
                    AutomergeHelpers::update_document_content_with_bytes(
                        &doc_handle,
                        content,
                        bytes,
                    )?;
                } else {
                    AutomergeHelpers::update_document_content(&doc_handle, content)?;
                }

                // Update timestamp in index
                self.update_path_index(|index| {
                    if let Some(entry) = index.paths.get_mut(path) {
                        entry.modified = chrono::Utc::now();
                    }
                })
                .await?;

                // Emit event
                let _ = self.event_tx.send(VfsEvent::DocumentUpdated {
                    path: path.to_string(),
                    doc_id: doc_handle.document_id().clone(),
                });

                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Patch a document at a specific JSON path
    pub async fn patch_document(
        &self,
        path: &str,
        json_path: &[String],
        value: serde_json::Value,
    ) -> Result<bool> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Prepend "content" to the path since content is stored under "content" key
        let mut full_path = vec!["content".to_string()];
        full_path.extend(json_path.iter().cloned());

        match self.find_document(path).await? {
            Some(doc_handle) => {
                AutomergeHelpers::patch_document(&doc_handle, &full_path, value)?;

                // Update timestamp in index
                self.update_path_index(|index| {
                    if let Some(entry) = index.paths.get_mut(path) {
                        entry.modified = chrono::Utc::now();
                    }
                })
                .await?;

                // Emit event
                let _ = self.event_tx.send(VfsEvent::DocumentUpdated {
                    path: path.to_string(),
                    doc_id: doc_handle.document_id().clone(),
                });

                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Splice text at a specific JSON path within a document
    pub async fn splice_text(
        &self,
        path: &str,
        json_path: &[String],
        index: usize,
        delete_count: isize,
        insert: &str,
    ) -> Result<bool> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Prepend "content" to the path since content is stored under "content" key
        let mut full_path = vec!["content".to_string()];
        full_path.extend(json_path.iter().cloned());

        match self.find_document(path).await? {
            Some(doc_handle) => {
                AutomergeHelpers::splice_text(
                    &doc_handle,
                    &full_path,
                    index,
                    delete_count,
                    insert,
                )?;

                // Update timestamp in index
                self.update_path_index(|index| {
                    if let Some(entry) = index.paths.get_mut(path) {
                        entry.modified = chrono::Utc::now();
                    }
                })
                .await?;

                // Emit event
                let _ = self.event_tx.send(VfsEvent::DocumentUpdated {
                    path: path.to_string(),
                    doc_id: doc_handle.document_id().clone(),
                });

                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Move a document or directory from one path to another
    pub async fn move_document(&self, from_path: &str, to_path: &str) -> Result<bool> {
        // Check for empty paths
        if from_path.is_empty() {
            return Err(VfsError::InvalidPath(
                "Source path cannot be empty".to_string(),
            ));
        }
        if to_path.is_empty() {
            return Err(VfsError::InvalidPath(
                "Destination path cannot be empty".to_string(),
            ));
        }

        // Check that paths start with '/'
        if !from_path.starts_with('/') {
            return Err(VfsError::InvalidPath(format!(
                "Source path must start with '/': {}",
                from_path
            )));
        }
        if !to_path.starts_with('/') {
            return Err(VfsError::InvalidPath(format!(
                "Destination path must start with '/': {}",
                to_path
            )));
        }

        if from_path == "/" || to_path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Normalize paths to ensure consistent comparison
        let normalized_from = from_path.trim_end_matches('/');
        let normalized_to = to_path.trim_end_matches('/');

        // Check for circular move
        if normalized_to.starts_with(&format!("{}/", normalized_from))
            || normalized_from == normalized_to
        {
            return Err(VfsError::CircularMove(format!(
                "Cannot move '{}' to '{}'",
                from_path, to_path
            )));
        }

        // Ensure destination parent directories exist
        self.ensure_parent_directories(to_path).await?;

        // Get the node type and document ID before moving
        let index = self.read_path_index().await?;
        let entry = index
            .get_entry(from_path)
            .ok_or_else(|| VfsError::PathNotFound(from_path.to_string()))?;
        let node_type = entry.node_type.clone();
        let doc_id = entry
            .doc_id
            .parse::<DocumentId>()
            .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid document ID: {}", e)))?;

        // Check if destination already exists
        if index.has_path(to_path) {
            return Err(VfsError::DocumentExists(to_path.to_string()));
        }

        // Update in index - also update all children if it's a directory
        self.update_path_index(|index| {
            // If it's a directory, we need to move all children too
            if node_type == NodeType::Directory {
                // Collect all children paths
                let mut children_to_move = Vec::new();
                for path in index.all_paths() {
                    if path.starts_with(&format!("{}/", from_path)) {
                        children_to_move.push(path.clone());
                    }
                }

                // Move all children
                for child_path in children_to_move {
                    let new_child_path = child_path.replacen(from_path, to_path, 1);
                    index.move_path(&child_path, &new_child_path).map_err(|e| {
                        VfsError::Other(anyhow::anyhow!("Failed to move child: {}", e))
                    })?;
                }
            }

            // Move the directory/document itself
            index
                .move_path(from_path, to_path)
                .map_err(VfsError::PathNotFound)
        })
        .await??;

        // Update the internal document name if the name changed
        let from_name = from_path.rsplit('/').next().unwrap_or(from_path);
        let to_name = to_path.rsplit('/').next().unwrap_or(to_path);

        if from_name != to_name {
            let doc_handle = self
                .samod
                .find(doc_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find moved document: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(doc_id.to_string()))?;

            AutomergeHelpers::update_document_name(&doc_handle, to_name)?;
        }

        // Update parents
        self.remove_from_parent(from_path).await?;
        self.add_to_parent(to_path, doc_id.clone(), node_type.clone())
            .await?;

        // Emit events
        let _ = self.event_tx.send(VfsEvent::DocumentDeleted {
            path: from_path.to_string(),
        });

        match node_type {
            NodeType::Directory => {
                let _ = self.event_tx.send(VfsEvent::DirectoryCreated {
                    path: to_path.to_string(),
                    doc_id,
                });
            }
            NodeType::Document => {
                let _ = self.event_tx.send(VfsEvent::DocumentCreated {
                    path: to_path.to_string(),
                    doc_id,
                });
            }
        }

        Ok(true)
    }

    /// Find a document at the specified path
    pub async fn find_document(&self, path: &str) -> Result<Option<DocHandle>> {
        let index = self.read_path_index().await?;

        // Look up document ID
        let Some(entry) = index.get_entry(path) else {
            return Ok(None);
        };

        if entry.node_type != NodeType::Document {
            return Err(VfsError::NodeTypeMismatch {
                expected: "document".to_string(),
                actual: "directory".to_string(),
            });
        }

        // Request specific document from Samod
        let doc_id = entry
            .doc_id
            .parse::<DocumentId>()
            .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid document ID: {}", e)))?;

        self.samod
            .find(doc_id)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find document: {e}")))
    }

    /// Remove a document at the specified path
    pub async fn remove_document(&self, path: &str) -> Result<bool> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Remove from index
        let removed = self
            .update_path_index(|index| index.remove_path(path))
            .await?;

        if removed.is_some() {
            // Remove from parent directory
            self.remove_from_parent(path).await?;

            // Emit event
            let _ = self.event_tx.send(VfsEvent::DocumentDeleted {
                path: path.to_string(),
            });
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// List contents of a directory
    pub async fn list_directory(&self, path: &str) -> Result<Vec<RefNode>> {
        let index = self.read_path_index().await?;

        let children = index.list_children(path);

        // Convert PathEntry to RefNode for compatibility
        let ref_nodes: Result<Vec<RefNode>> = children
            .into_iter()
            .map(|(child_path, entry)| {
                // Extract just the filename from the full path
                let name = child_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&child_path)
                    .to_string();

                let pointer = entry
                    .doc_id
                    .parse::<DocumentId>()
                    .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid document ID: {}", e)))?;

                Ok(RefNode {
                    pointer,
                    node_type: entry.node_type.clone(),
                    timestamps: Timestamps {
                        created: entry.created,
                        modified: entry.modified,
                    },
                    name,
                })
            })
            .collect();

        ref_nodes
    }

    /// Create a directory at the specified path
    pub async fn create_directory(&self, path: &str) -> Result<DocHandle> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Check if already exists
        let index = self.read_path_index().await?;
        if index.has_path(path) {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Create the directory document
        let new_doc = Automerge::new();
        let dir_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create directory: {e}")))?;

        // Initialize as directory
        let dirname = path.rsplit('/').next().unwrap_or(path);
        AutomergeHelpers::init_as_directory(&dir_handle, dirname)?;

        // Update path index
        let doc_id = dir_handle.document_id().clone();
        self.update_path_index(|index| {
            index.set_path(path.to_string(), doc_id.to_string(), NodeType::Directory);
        })
        .await?;

        // Add to parent directory
        self.add_to_parent(path, doc_id.clone(), NodeType::Directory)
            .await?;

        // Emit event
        let _ = self.event_tx.send(VfsEvent::DirectoryCreated {
            path: path.to_string(),
            doc_id: dir_handle.document_id().clone(),
        });

        Ok(dir_handle)
    }

    /// Check if a path exists
    pub async fn exists(&self, path: &str) -> Result<bool> {
        let index = self.read_path_index().await?;
        Ok(index.has_path(path))
    }

    /// Get metadata for a path
    pub async fn metadata(&self, path: &str) -> Result<RefNode> {
        let index = self.read_path_index().await?;

        if let Some(entry) = index.get_entry(path) {
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            let pointer = entry
                .doc_id
                .parse::<DocumentId>()
                .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid document ID: {}", e)))?;

            Ok(RefNode {
                pointer,
                node_type: entry.node_type.clone(),
                timestamps: Timestamps {
                    created: entry.created,
                    modified: entry.modified,
                },
                name,
            })
        } else {
            Err(VfsError::PathNotFound(path.to_string()))
        }
    }

    /// Watch a document for changes at the specified path
    pub async fn watch_document(&self, path: &str) -> Result<Option<DocumentWatcher>> {
        if let Some(doc_handle) = self.find_document(path).await? {
            Ok(Some(DocumentWatcher::new(doc_handle)))
        } else {
            Ok(None)
        }
    }

    /// Watch a directory for changes at the specified path
    pub async fn watch_directory(&self, path: &str) -> Result<Option<DocumentWatcher>> {
        // Special case for root directory - watch the path index itself
        if path == "/" || path.is_empty() {
            let root_handle = self
                .samod
                .find(self.root_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find root: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(self.root_id.to_string()))?;
            return Ok(Some(DocumentWatcher::new(root_handle)));
        }

        let index = self.read_path_index().await?;

        let entry = index.get_entry(path);
        if let Some(entry) = entry {
            if entry.node_type == NodeType::Directory {
                let doc_id = entry
                    .doc_id
                    .parse::<DocumentId>()
                    .map_err(|e| VfsError::Other(anyhow::anyhow!("Invalid document ID: {}", e)))?;

                let dir_handle =
                    self.samod.find(doc_id).await.map_err(|e| {
                        VfsError::SamodError(format!("Failed to find directory: {e}"))
                    })?;
                match dir_handle {
                    Some(handle) => Ok(Some(DocumentWatcher::new(handle))),
                    None => Ok(None),
                }
            } else {
                Err(VfsError::NodeTypeMismatch {
                    expected: "directory".to_string(),
                    actual: "document".to_string(),
                })
            }
        } else {
            Ok(None)
        }
    }

    /// Collect all document IDs used by this VFS (for bundle export)
    pub async fn collect_all_document_ids(&self) -> Result<std::collections::HashSet<DocumentId>> {
        let mut doc_ids = std::collections::HashSet::new();

        // Always include the root
        doc_ids.insert(self.root_id.clone());

        // Recursively traverse the VFS to collect all document IDs
        self.collect_document_ids_recursive("/", &mut doc_ids)
            .await?;

        Ok(doc_ids)
    }

    /// Recursively collect document IDs from a directory
    fn collect_document_ids_recursive<'a>(
        &'a self,
        path: &'a str,
        doc_ids: &'a mut std::collections::HashSet<DocumentId>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // List entries in this directory
            let entries = self.list_directory(path).await?;

            for entry in entries {
                // Add this document's ID
                doc_ids.insert(entry.pointer.clone());

                // If it's a directory, recurse into it
                if entry.node_type == NodeType::Directory {
                    let child_path = if path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", path, entry.name)
                    };

                    self.collect_document_ids_recursive(&child_path, doc_ids)
                        .await?;
                }
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tonk_core::TonkCore;

    #[tokio::test]
    async fn test_vfs_creation() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        assert!(!vfs.root_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        let mut rx = vfs.subscribe_events();

        // Create a document
        vfs.create_document("/test.txt", "Initial".to_string())
            .await
            .unwrap();

        // Check for create event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DocumentCreated { path, .. } => {
                    assert_eq!(path, "/test.txt");
                }
                _ => panic!("Expected DocumentCreated event"),
            }
        }

        // Update the document
        vfs.update_document("/test.txt", "Updated".to_string())
            .await
            .unwrap();

        // Check for update event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DocumentUpdated { path, .. } => {
                    assert_eq!(path, "/test.txt");
                }
                _ => panic!("Expected DocumentUpdated event"),
            }
        }
    }

    #[tokio::test]
    async fn test_path_validation() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Test root path validation
        let result = vfs.create_document("/", "content".to_string()).await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        let result = vfs.remove_document("/").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        let result = vfs.create_directory("/").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        let result = vfs.update_document("/", "content".to_string()).await;
        assert!(matches!(result, Err(VfsError::RootPathError)));
    }

    #[tokio::test]
    async fn test_document_creation_and_removal() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        let doc_handle = vfs
            .create_document("/test.txt", "Hello, VFS!".to_string())
            .await
            .unwrap();
        assert!(!doc_handle.document_id().to_string().is_empty());

        // Verify document exists
        let found = vfs.find_document("/test.txt").await.unwrap();
        assert!(found.is_some());

        // Remove the document
        let removed = vfs.remove_document("/test.txt").await.unwrap();
        assert!(removed);

        // Verify document no longer exists
        let found = vfs.find_document("/test.txt").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_document_update() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        let doc_handle = vfs
            .create_document("/test.txt", "Initial content".to_string())
            .await
            .unwrap();
        let doc_id = doc_handle.document_id().clone();

        // Verify initial content
        let handle = vfs.find_document("/test.txt").await.unwrap().unwrap();
        let initial_content: String = handle.with_document(|doc| {
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
        assert_eq!(initial_content, "\"Initial content\"");

        // Update the document
        let updated = vfs
            .update_document("/test.txt", "Updated content".to_string())
            .await
            .unwrap();
        assert!(updated);

        // Verify updated content
        let handle = vfs.find_document("/test.txt").await.unwrap().unwrap();
        let updated_content: String = handle.with_document(|doc| {
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
        assert_eq!(updated_content, "\"Updated content\"");

        // Verify document ID remains the same
        assert_eq!(handle.document_id(), &doc_id);

        // Try to update a non-existent document
        let updated = vfs
            .update_document("/non-existent.txt", "Content".to_string())
            .await
            .unwrap();
        assert!(!updated);
    }

    #[tokio::test]
    async fn test_directory_operations() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a directory
        let dir_handle = vfs.create_directory("/documents").await.unwrap();
        assert!(!dir_handle.document_id().to_string().is_empty());

        // List root directory
        let children = vfs.list_directory("/").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "documents");
        assert_eq!(children[0].node_type, NodeType::Directory);

        // Create a document in the directory
        vfs.create_document("/documents/file.txt", "Content".to_string())
            .await
            .unwrap();

        // List the directory
        let children = vfs.list_directory("/documents").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "file.txt");
        assert_eq!(children[0].node_type, NodeType::Document);
    }

    #[tokio::test]
    async fn test_nested_directory_creation() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document in a nested path (should create parent directories)
        vfs.create_document("/a/b/c/file.txt", "Nested content".to_string())
            .await
            .unwrap();

        // Verify directory structure
        let children = vfs.list_directory("/").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "a");

        let children = vfs.list_directory("/a").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "b");

        let children = vfs.list_directory("/a/b").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "c");

        let children = vfs.list_directory("/a/b/c").await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "file.txt");
    }

    #[tokio::test]
    async fn test_update_in_nested_directory() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document in a nested directory
        vfs.create_document("/a/b/c/file.txt", "Original content".to_string())
            .await
            .unwrap();

        // Update the nested document
        let updated = vfs
            .update_document("/a/b/c/file.txt", "New content".to_string())
            .await
            .unwrap();
        assert!(updated);

        // Verify the update
        let handle = vfs.find_document("/a/b/c/file.txt").await.unwrap().unwrap();
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
        assert_eq!(content, "\"New content\"");
    }

    #[tokio::test]
    async fn test_duplicate_prevention() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        vfs.create_document("/test.txt", "Original".to_string())
            .await
            .unwrap();

        // Try to create a document with the same name
        let result = vfs
            .create_document("/test.txt", "Duplicate".to_string())
            .await;
        assert!(matches!(result, Err(VfsError::DocumentExists(_))));

        // Create a directory
        vfs.create_directory("/mydir").await.unwrap();

        // Try to create a directory with the same name
        let result = vfs.create_directory("/mydir").await;
        assert!(matches!(result, Err(VfsError::DocumentExists(_))));
    }

    #[tokio::test]
    async fn test_watch_document() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        let _doc_handle = vfs
            .create_document("/test.txt", "Initial content".to_string())
            .await
            .unwrap();

        // Watch the document
        let watcher = vfs.watch_document("/test.txt").await.unwrap();
        assert!(watcher.is_some());

        let watcher = watcher.unwrap();
        assert!(!watcher.document_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_watch_directory() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a directory
        vfs.create_directory("/mydir").await.unwrap();

        // Watch the directory
        let watcher = vfs.watch_directory("/mydir").await.unwrap();
        assert!(watcher.is_some());

        // Test watching root directory
        let root_watcher = vfs.watch_directory("/").await.unwrap();
        assert!(root_watcher.is_some());

        let subdir_watcher = watcher.unwrap();
        let subdir_changes = std::sync::Arc::new(std::sync::Mutex::new(0));

        let _subdir_task = tokio::spawn({
            let count = subdir_changes.clone();
            async move {
                subdir_watcher
                    .on_change(move |_| {
                        let mut c = count.lock().unwrap();
                        *c += 1;
                    })
                    .await;
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Add a file to the directory
        vfs.create_document("/mydir/file.txt", "content".to_string())
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        assert!(
            *subdir_changes.lock().unwrap() > 0,
            "Subdir watcher failed to detect file creation"
        );

        let subdir_changes_del = std::sync::Arc::new(std::sync::Mutex::new(0));
        // Note: previous watcher task is still running and will increment its counter too,
        // but we'll spawn a new one to be clean/explicit about this phase.

        // Use the same watcher instance (cloned logic/ref)
        let watcher_for_del = vfs.watch_directory("/mydir").await.unwrap().unwrap();

        let _del_task = tokio::spawn({
            let count = subdir_changes_del.clone();
            async move {
                watcher_for_del
                    .on_change(move |_| {
                        let mut c = count.lock().unwrap();
                        *c += 1;
                    })
                    .await;
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Delete the file
        vfs.remove_document("/mydir/file.txt").await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        assert!(
            *subdir_changes_del.lock().unwrap() > 0,
            "Subdir watcher failed to detect file deletion"
        );
    }

    #[tokio::test]
    async fn test_watch_non_existent_document() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Try to watch a non-existent document
        let watcher = vfs.watch_document("/does-not-exist.txt").await.unwrap();
        assert!(watcher.is_none());
    }

    #[tokio::test]
    async fn test_watch_type_mismatch() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        let _create_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            vfs.create_document("/file.txt", "content".to_string()),
        )
        .await
        .unwrap()
        .unwrap();

        // Try to watch it as a directory
        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            vfs.watch_directory("/file.txt"),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(VfsError::NodeTypeMismatch { .. })));
    }

    #[tokio::test]
    async fn test_move_document_file() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create directories
        vfs.create_directory("/old").await.unwrap();
        vfs.create_directory("/new").await.unwrap();

        // Create a file in /old
        let doc_handle = vfs
            .create_document("/old/file.txt", "Content".to_string())
            .await
            .unwrap();
        let doc_id = doc_handle.document_id().clone();

        // Move the file to /new
        let moved = vfs
            .move_document("/old/file.txt", "/new/file.txt")
            .await
            .unwrap();
        assert!(moved);

        // Verify file no longer exists in old location
        let old_file = vfs.find_document("/old/file.txt").await.unwrap();
        assert!(old_file.is_none());

        // Verify file exists in new location with same doc_id
        let new_file = vfs.find_document("/new/file.txt").await.unwrap();
        assert!(new_file.is_some());
        assert_eq!(new_file.unwrap().document_id(), &doc_id);

        // Verify directory listings
        let old_children = vfs.list_directory("/old").await.unwrap();
        assert_eq!(old_children.len(), 0);

        let new_children = vfs.list_directory("/new").await.unwrap();
        assert_eq!(new_children.len(), 1);
        assert_eq!(new_children[0].name, "file.txt");
        assert_eq!(new_children[0].node_type, NodeType::Document);
    }

    #[tokio::test]
    async fn test_move_document_watchers() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create directories
        vfs.create_directory("/old").await.unwrap();
        vfs.create_directory("/new").await.unwrap();

        // Setup watchers
        let old_watcher = vfs.watch_directory("/old").await.unwrap().unwrap();
        let new_watcher = vfs.watch_directory("/new").await.unwrap().unwrap();

        let old_changes = std::sync::Arc::new(std::sync::Mutex::new(0));
        let new_changes = std::sync::Arc::new(std::sync::Mutex::new(0));

        let _old_task = tokio::spawn({
            let count = old_changes.clone();
            async move {
                old_watcher
                    .on_change(move |_| {
                        *count.lock().unwrap() += 1;
                    })
                    .await;
            }
        });

        let _new_task = tokio::spawn({
            let count = new_changes.clone();
            async move {
                new_watcher
                    .on_change(move |_| {
                        *count.lock().unwrap() += 1;
                    })
                    .await;
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Create a file in /old
        vfs.create_document("/old/file.txt", "Content".to_string())
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        assert!(
            *old_changes.lock().unwrap() > 0,
            "Old directory should detect creation"
        );
        let old_count_after_create = *old_changes.lock().unwrap();
        assert_eq!(
            *new_changes.lock().unwrap(),
            0,
            "New directory should not detect creation"
        );

        // Move the file to /new
        vfs.move_document("/old/file.txt", "/new/file.txt")
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        assert!(
            *old_changes.lock().unwrap() > old_count_after_create,
            "Old directory should detect removal"
        );
        assert!(
            *new_changes.lock().unwrap() > 0,
            "New directory should detect addition"
        );
    }

    #[tokio::test]
    async fn test_move_document_directory() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create nested structure: /source/mydir/file.txt
        vfs.create_document("/source/mydir/file.txt", "Content".to_string())
            .await
            .unwrap();
        vfs.create_directory("/dest").await.unwrap();

        // Get the directory's doc_id before moving
        let metadata = vfs.metadata("/source/mydir").await.unwrap();
        let dir_doc_id = metadata.pointer.clone();

        // Move the directory
        let moved = vfs
            .move_document("/source/mydir", "/dest/mydir")
            .await
            .unwrap();
        assert!(moved);

        // Verify directory no longer exists in old location
        let old_exists = vfs.exists("/source/mydir").await.unwrap();
        assert!(!old_exists);

        // Verify directory exists in new location with same doc_id
        let new_metadata = vfs.metadata("/dest/mydir").await.unwrap();
        assert_eq!(new_metadata.pointer, dir_doc_id);
        assert_eq!(new_metadata.node_type, NodeType::Directory);

        // Verify the file inside the moved directory is still accessible
        let file = vfs.find_document("/dest/mydir/file.txt").await.unwrap();
        assert!(file.is_some());

        // Verify directory listings
        let source_children = vfs.list_directory("/source").await.unwrap();
        assert_eq!(source_children.len(), 0);

        let dest_children = vfs.list_directory("/dest").await.unwrap();
        assert_eq!(dest_children.len(), 1);
        assert_eq!(dest_children[0].name, "mydir");
    }

    #[tokio::test]
    async fn test_move_document_with_rename() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a file
        let doc_handle = vfs
            .create_document("/oldname.txt", "Content".to_string())
            .await
            .unwrap();
        let doc_id = doc_handle.document_id().clone();

        // Move and rename
        let moved = vfs
            .move_document("/oldname.txt", "/newname.txt")
            .await
            .unwrap();
        assert!(moved);

        // Verify old name doesn't exist
        let old_file = vfs.find_document("/oldname.txt").await.unwrap();
        assert!(old_file.is_none());

        // Verify new name exists with same doc_id
        let new_file = vfs.find_document("/newname.txt").await.unwrap();
        assert!(new_file.is_some());
        assert_eq!(new_file.unwrap().document_id(), &doc_id);

        // Test moving directory with rename
        vfs.create_directory("/olddir").await.unwrap();
        let moved = vfs.move_document("/olddir", "/newdir").await.unwrap();
        assert!(moved);

        let old_exists = vfs.exists("/olddir").await.unwrap();
        assert!(!old_exists);

        let new_exists = vfs.exists("/newdir").await.unwrap();
        assert!(new_exists);
    }

    #[tokio::test]
    async fn test_move_document_to_nested_path() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a file at root
        let doc_handle = vfs
            .create_document("/file.txt", "Content".to_string())
            .await
            .unwrap();
        let doc_id = doc_handle.document_id().clone();

        // Move to a deeply nested path that doesn't exist yet
        let moved = vfs
            .move_document("/file.txt", "/a/b/c/d/file.txt")
            .await
            .unwrap();
        assert!(moved);

        // Verify the nested directories were created
        assert!(vfs.exists("/a").await.unwrap());
        assert!(vfs.exists("/a/b").await.unwrap());
        assert!(vfs.exists("/a/b/c").await.unwrap());
        assert!(vfs.exists("/a/b/c/d").await.unwrap());

        // Verify file exists in new location
        let file = vfs.find_document("/a/b/c/d/file.txt").await.unwrap();
        assert!(file.is_some());
        assert_eq!(file.unwrap().document_id(), &doc_id);
    }

    #[tokio::test]
    async fn test_move_document_root_error() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Try to move root
        let result = vfs.move_document("/", "/newroot").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        // Try to move to root
        vfs.create_document("/file.txt", "Content".to_string())
            .await
            .unwrap();
        let result = vfs.move_document("/file.txt", "/").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));
    }

    #[tokio::test]
    async fn test_move_document_to_existing_path() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create two files
        vfs.create_document("/file1.txt", "Content1".to_string())
            .await
            .unwrap();
        vfs.create_document("/file2.txt", "Content2".to_string())
            .await
            .unwrap();

        // Try to move file1 to file2's location
        let result = vfs.move_document("/file1.txt", "/file2.txt").await;
        assert!(matches!(result, Err(VfsError::DocumentExists(_))));

        // Verify both files still exist
        assert!(vfs.find_document("/file1.txt").await.unwrap().is_some());
        assert!(vfs.find_document("/file2.txt").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_move_document_non_existent() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Try to move a non-existent file
        let result = vfs
            .move_document("/nonexistent.txt", "/destination.txt")
            .await;
        assert!(matches!(result, Err(VfsError::PathNotFound(_))));
    }

    #[tokio::test]
    async fn test_move_document_events() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        let mut rx = vfs.subscribe_events();

        // Create and move a file
        vfs.create_document("/file.txt", "Content".to_string())
            .await
            .unwrap();

        // Clear creation event
        let _ = rx.try_recv();

        // Move the file
        vfs.move_document("/file.txt", "/moved.txt").await.unwrap();

        // Check for delete event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DocumentDeleted { path } => {
                    assert_eq!(path, "/file.txt");
                }
                _ => panic!("Expected DocumentDeleted event"),
            }
        }

        // Check for create event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DocumentCreated { path, .. } => {
                    assert_eq!(path, "/moved.txt");
                }
                _ => panic!("Expected DocumentCreated event"),
            }
        }

        // Test directory move events
        vfs.create_directory("/dir").await.unwrap();
        let _ = rx.try_recv(); // Clear directory creation event

        vfs.move_document("/dir", "/moveddir").await.unwrap();

        // Check for delete event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DocumentDeleted { path } => {
                    assert_eq!(path, "/dir");
                }
                _ => panic!("Expected DocumentDeleted event"),
            }
        }

        // Check for directory creation event
        if let Ok(event) = rx.try_recv() {
            match event {
                VfsEvent::DirectoryCreated { path, .. } => {
                    assert_eq!(path, "/moveddir");
                }
                _ => panic!("Expected DirectoryCreated event"),
            }
        }
    }

    #[tokio::test]
    async fn test_move_document_circular_move_prevention() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create nested structure: /parent/child/grandchild
        vfs.create_document("/parent/child/grandchild/file.txt", "Content".to_string())
            .await
            .unwrap();

        // Try to move parent into its own child (circular)
        let result = vfs.move_document("/parent", "/parent/child/parent").await;
        assert!(matches!(result, Err(VfsError::CircularMove(_))));

        // Try to move into grandchild (also circular)
        let result = vfs
            .move_document("/parent", "/parent/child/grandchild/parent")
            .await;
        assert!(matches!(result, Err(VfsError::CircularMove(_))));

        // Try to move into itself
        let result = vfs.move_document("/parent/child", "/parent/child").await;
        assert!(matches!(result, Err(VfsError::CircularMove(_))));

        // Verify structure is unchanged
        assert!(vfs.exists("/parent").await.unwrap());
        assert!(vfs.exists("/parent/child").await.unwrap());
        assert!(vfs.exists("/parent/child/grandchild").await.unwrap());

        // Valid sibling move should still work
        let result = vfs.move_document("/parent/child", "/parent/sibling").await;
        assert!(result.is_ok());
        assert!(vfs.exists("/parent/sibling").await.unwrap());
        assert!(!vfs.exists("/parent/child").await.unwrap());
    }

    #[tokio::test]
    async fn test_move_document_updates_internal_name() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create a document
        vfs.create_document("/original.txt", "Content".to_string())
            .await
            .unwrap();

        // Read the internal name before move
        let doc_handle = vfs.find_document("/original.txt").await.unwrap().unwrap();
        let original_internal_name: String = doc_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "name")
                .ok()
                .flatten()
                .and_then(|(value, _)| {
                    if let automerge::Value::Scalar(s) = value {
                        Some(s.to_string().trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        });
        assert_eq!(original_internal_name, "original.txt");

        // Move and rename the document
        vfs.move_document("/original.txt", "/renamed.txt")
            .await
            .unwrap();

        // Read the internal name after move
        let doc_handle = vfs.find_document("/renamed.txt").await.unwrap().unwrap();
        let new_internal_name: String = doc_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "name")
                .ok()
                .flatten()
                .and_then(|(value, _)| {
                    if let automerge::Value::Scalar(s) = value {
                        Some(s.to_string().trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        });
        assert_eq!(new_internal_name, "renamed.txt");

        // Test with directory
        vfs.create_directory("/olddir").await.unwrap();
        vfs.move_document("/olddir", "/newdir").await.unwrap();

        // Verify directory internal name was updated
        let dir_metadata = vfs.metadata("/newdir").await.unwrap();
        let dir_handle = vfs
            .samod
            .find(dir_metadata.pointer.clone())
            .await
            .unwrap()
            .unwrap();
        let dir_internal_name: String = dir_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "name")
                .ok()
                .flatten()
                .and_then(|(value, _)| {
                    if let automerge::Value::Scalar(s) = value {
                        Some(s.to_string().trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        });
        assert_eq!(dir_internal_name, "newdir");
    }

    #[tokio::test]
    async fn test_move_document_without_rename_no_internal_update() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Create directories and a file
        vfs.create_directory("/dir1").await.unwrap();
        vfs.create_directory("/dir2").await.unwrap();
        vfs.create_document("/dir1/file.txt", "Content".to_string())
            .await
            .unwrap();

        // Get the document handle and read timestamps
        let doc_handle = vfs.find_document("/dir1/file.txt").await.unwrap().unwrap();
        let original_modified_time = doc_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "timestamps")
                .ok()
                .flatten()
                .and_then(|(value, ts_obj_id)| {
                    if let automerge::Value::Object(_) = value {
                        doc.get(ts_obj_id, "modified")
                            .ok()
                            .flatten()
                            .and_then(|(ts_val, _)| match ts_val {
                                automerge::Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                                _ => None,
                            })
                    } else {
                        None
                    }
                })
        });

        // Small delay to ensure time difference would be detectable
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Move without renaming (same filename, different parent)
        vfs.move_document("/dir1/file.txt", "/dir2/file.txt")
            .await
            .unwrap();

        // When moved without renaming, the internal name should stay the same
        // and the document's internal timestamp should NOT be updated
        let doc_handle = vfs.find_document("/dir2/file.txt").await.unwrap().unwrap();
        let internal_name: String = doc_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "name")
                .ok()
                .flatten()
                .and_then(|(value, _)| {
                    if let automerge::Value::Scalar(s) = value {
                        Some(s.to_string().trim_matches('"').to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        });
        assert_eq!(internal_name, "file.txt");

        let new_modified_time = doc_handle.with_document(|doc| {
            use automerge::ReadDoc;
            doc.get(automerge::ROOT, "timestamps")
                .ok()
                .flatten()
                .and_then(|(value, ts_obj_id)| {
                    if let automerge::Value::Object(_) = value {
                        doc.get(ts_obj_id, "modified")
                            .ok()
                            .flatten()
                            .and_then(|(ts_val, _)| match ts_val {
                                automerge::Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                                _ => None,
                            })
                    } else {
                        None
                    }
                })
        });

        // Timestamps should be the same since we didn't update the document
        assert_eq!(original_modified_time, new_modified_time);
    }

    #[tokio::test]
    async fn test_path_index_operations() {
        let tonk = TonkCore::new().await.unwrap();
        let vfs = VirtualFileSystem::new(tonk.samod()).await.unwrap();

        // Read the initial empty path index
        let index = vfs.read_path_index().await.unwrap();
        assert_eq!(index.paths.len(), 0);

        // Update the path index
        vfs.update_path_index(|index| {
            index.set_path(
                "/test.json".to_string(),
                "doc123".to_string(),
                NodeType::Document,
            );
        })
        .await
        .unwrap();

        // Verify the update
        let index = vfs.read_path_index().await.unwrap();
        assert_eq!(index.paths.len(), 1);
        assert!(index.has_path("/test.json"));
        assert_eq!(index.get_doc_id("/test.json"), Some(&"doc123".to_string()));

        // Update again with more entries
        vfs.update_path_index(|index| {
            index.set_path(
                "/dir/file.json".to_string(),
                "doc456".to_string(),
                NodeType::Document,
            );
            index.set_path(
                "/dir".to_string(),
                "doc789".to_string(),
                NodeType::Directory,
            );
        })
        .await
        .unwrap();

        // Verify multiple entries
        let index = vfs.read_path_index().await.unwrap();
        assert_eq!(index.paths.len(), 3);
        assert!(index.has_path("/test.json"));
        assert!(index.has_path("/dir"));
        assert!(index.has_path("/dir/file.json"));
    }
}
