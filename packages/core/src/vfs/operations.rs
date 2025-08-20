use crate::error::{Result, VfsError};
use crate::vfs::traversal::PathTraverser;
use crate::vfs::types::*;
use automerge::Automerge;
use samod::{DocHandle, DocumentId, Samod};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct VirtualFileSystem {
    samod: Arc<Samod>,
    root_id: DocumentId,
    traverser: PathTraverser,
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
    pub async fn new(samod: Arc<Samod>) -> Result<Self> {
        // Create root document with directory structure
        let root_doc = Automerge::new();

        // TODO: Initialize root as directory - in a real implementation,
        // we'd properly set up the automerge document structure
        let root_handle = samod
            .create(root_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create root document: {}", e)))?;

        let root_id = root_handle.document_id().clone();

        // Update the root document (simplified - needs proper automerge handling)
        root_handle
            .with_document(|_doc| {
                // TODO: In a real implementation, we'd properly update the automerge document
                // with the directory structure using automerge operations
                Ok(())
            })
            .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
                VfsError::Other(anyhow::anyhow!("{}", e))
            })?;

        let (event_tx, _) = broadcast::channel(100);
        let traverser = PathTraverser::new(samod.clone());

        Ok(Self {
            samod,
            root_id,
            traverser,
            event_tx,
        })
    }

    /// Create a new VFS from an existing root document
    pub async fn from_root(samod: Arc<Samod>, root_id: DocumentId) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(100);
        let traverser = PathTraverser::new(samod.clone());

        Ok(Self {
            samod,
            root_id,
            traverser,
            event_tx,
        })
    }

    /// Get the root document ID
    pub fn root_id(&self) -> DocumentId {
        self.root_id.clone()
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
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Traverse to parent directory
        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, true)
            .await?;

        // Check if document already exists
        if result.target_ref.is_some() {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Extract filename from path
        let filename = path
            .rsplit('/')
            .next()
            .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

        // Create document node
        let doc_node = DocNode::new(filename.to_string(), content);
        let mut new_doc = Automerge::new();

        // TODO: In a real implementation, we'd serialize the doc_node into the automerge document
        let doc_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {}", e)))?;

        // Add reference to parent directory
        let doc_ref = RefNode::new_document(filename.to_string(), doc_handle.document_id().clone());
        let mut parent_dir = result.node;
        parent_dir.add_child(doc_ref);

        // Update parent directory (simplified - needs proper automerge handling)
        result
            .node_handle
            .with_document(|doc| {
                // Update automerge document with new directory structure
                Ok(())
            })
            .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
                VfsError::Other(anyhow::anyhow!("{}", e))
            })?;

        // Emit event
        let _ = self.event_tx.send(VfsEvent::DocumentCreated {
            path: path.to_string(),
            doc_id: doc_handle.document_id().clone(),
        });

        Ok(doc_handle)
    }

    /// Find a document at the specified path
    pub async fn find_document(&self, path: &str) -> Result<Option<DocHandle>> {
        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await?;

        if let Some(target_ref) = result.target_ref {
            if target_ref.node_type == NodeType::Document {
                let doc_handle = self
                    .samod
                    .find(target_ref.pointer.clone())
                    .await
                    .map_err(|e| VfsError::SamodError(format!("Failed to find document: {}", e)))?;
                match doc_handle {
                    Some(handle) => Ok(Some(handle)),
                    None => Ok(None),
                }
            } else {
                Err(VfsError::NodeTypeMismatch {
                    expected: "document".to_string(),
                    actual: "directory".to_string(),
                })
            }
        } else {
            Ok(None)
        }
    }

    /// Remove a document at the specified path
    pub async fn remove_document(&self, path: &str) -> Result<bool> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await?;

        if let Some(_target_ref) = result.target_ref {
            // Extract filename from path
            let filename = path
                .rsplit('/')
                .next()
                .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

            // Remove from parent directory
            let mut parent_dir = result.node;
            let removed = parent_dir.remove_child(filename);

            if removed.is_some() {
                // Update parent directory (simplified - needs proper automerge handling)
                result
                    .node_handle
                    .with_document(|_doc| {
                        // Update automerge document with modified directory structure
                        Ok(())
                    })
                    .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
                        VfsError::Other(anyhow::anyhow!("{}", e))
                    })?;

                // Emit event
                let _ = self.event_tx.send(VfsEvent::DocumentDeleted {
                    path: path.to_string(),
                });

                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// List contents of a directory
    pub async fn list_directory(&self, path: &str) -> Result<Vec<RefNode>> {
        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await?;

        if let Some(target_ref) = &result.target_ref {
            if target_ref.node_type != NodeType::Directory {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "directory".to_string(),
                    actual: "document".to_string(),
                });
            }

            // Get the directory document
            let dir_handle = self
                .samod
                .find(target_ref.pointer.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find directory: {}", e)))?;

            let dir_node = match dir_handle {
                Some(ref handle) => self.get_dir_node(handle).await?,
                None => return Err(VfsError::PathNotFound(path.to_string())),
            };
            Ok(dir_node.children)
        } else {
            // Return children of the current directory node
            Ok(result.node.children)
        }
    }

    /// Create a directory at the specified path
    pub async fn create_directory(&self, path: &str) -> Result<DocHandle> {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, true)
            .await?;

        // Check if directory already exists
        if result.target_ref.is_some() {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Extract directory name from path
        let dirname = path
            .rsplit('/')
            .next()
            .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

        // Create directory document
        let new_doc = Automerge::new();

        let dir_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create directory: {}", e)))?;

        // Add reference to parent directory
        let dir_ref = RefNode::new_directory(dirname.to_string(), dir_handle.document_id().clone());
        let mut parent_dir = result.node;
        parent_dir.add_child(dir_ref);

        // Update parent directory
        result.node_handle.with_document(|_doc| Ok(())).map_err(
            |e: Box<dyn std::error::Error + Send + Sync>| VfsError::Other(anyhow::anyhow!("{}", e)),
        )?;

        // Emit event
        let _ = self.event_tx.send(VfsEvent::DirectoryCreated {
            path: path.to_string(),
            doc_id: dir_handle.document_id().clone(),
        });

        Ok(dir_handle)
    }

    /// Check if a path exists
    pub async fn exists(&self, path: &str) -> Result<bool> {
        match self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await
        {
            Ok(result) => Ok(result.target_ref.is_some()),
            Err(VfsError::PathNotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Get metadata for a path
    pub async fn get_metadata(&self, path: &str) -> Result<Option<(NodeType, Timestamps)>> {
        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await?;

        if let Some(target_ref) = result.target_ref {
            Ok(Some((target_ref.node_type, target_ref.timestamps)))
        } else {
            Ok(None)
        }
    }

    // Helper method to extract DirNode from a document handle
    async fn get_dir_node(&self, _handle: &DocHandle) -> Result<DirNode> {
        // TODO: This is a placeholder implementation
        // In a real implementation, we'd properly deserialize from automerge
        Ok(DirNode::new("placeholder".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncEngine;

    #[tokio::test]
    async fn test_vfs_creation() {
        let engine = SyncEngine::new().await.unwrap();
        let vfs = VirtualFileSystem::new(engine.samod()).await.unwrap();

        assert!(vfs.root_id().to_string().len() > 0);
    }

    #[tokio::test]
    async fn test_event_subscription() {
        let engine = SyncEngine::new().await.unwrap();
        let vfs = VirtualFileSystem::new(engine.samod()).await.unwrap();

        let _rx = vfs.subscribe_events();
        // Just test that we can subscribe
    }

    #[tokio::test]
    async fn test_path_validation() {
        let engine = SyncEngine::new().await.unwrap();
        let vfs = VirtualFileSystem::new(engine.samod()).await.unwrap();

        // Test root path validation
        let result = vfs.create_document("/", "content").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        let result = vfs.remove_document("/").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));

        let result = vfs.create_directory("/").await;
        assert!(matches!(result, Err(VfsError::RootPathError)));
    }
}
