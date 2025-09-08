use crate::bundle::RandomAccess;
use crate::error::{Result, VfsError};
use crate::vfs::backend::AutomergeHelpers;
use crate::vfs::traversal::PathTraverser;
use crate::vfs::types::*;
use crate::vfs::watcher::DocumentWatcher;
use crate::Bundle;
use automerge::Automerge;
use samod::{DocHandle, DocumentId, Repo};
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct VirtualFileSystem {
    samod: Arc<Repo>,
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
    pub async fn new(samod: Arc<Repo>) -> Result<Self> {
        // Create root document with directory structure
        let root_doc = Automerge::new();

        let root_handle = samod
            .create(root_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create root document: {e}")))?;

        let root_id = root_handle.document_id().clone();

        // Initialize root as directory
        AutomergeHelpers::init_as_directory(&root_handle, "/")?;

        let (event_tx, _) = broadcast::channel(100);
        let traverser = PathTraverser::new(samod.clone());

        Ok(Self {
            samod,
            root_id,
            traverser,
            event_tx,
        })
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
        let traverser = PathTraverser::new(Arc::clone(&samod));

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

    // TODO: need a write_document as well
    /// Create a document at the specified path
    pub async fn create_document<T>(&self, path: &str, content: T) -> Result<DocHandle>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static,
    {
        if path == "/" {
            return Err(VfsError::RootPathError);
        }

        // Extract filename from path
        let filename = path
            .rsplit('/')
            .next()
            .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

        // Get the parent directory path
        let parent_path = if path.contains('/') {
            let last_slash = path.rfind('/').unwrap();
            if last_slash == 0 {
                "/"
            } else {
                &path[..last_slash]
            }
        } else {
            "/"
        };

        // Traverse to parent directory, creating directories as needed
        let result = self
            .traverser
            .traverse(self.root_id.clone(), parent_path, true)
            .await?;

        // Get the actual parent directory handle
        let parent_handle = if let Some(ref target_ref) = result.target_ref {
            self.samod
                .find(target_ref.pointer.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find parent directory: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(target_ref.pointer.to_string()))?
        } else {
            result.node_handle.clone()
        };

        // Check if document already exists in parent
        let parent_dir = AutomergeHelpers::read_directory(&parent_handle)?;
        if parent_dir.find_child(filename).is_some() {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Create the new document
        let new_doc = Automerge::new();
        let doc_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create document: {e}")))?;

        // Initialize the document with content
        AutomergeHelpers::init_as_document(&doc_handle, filename, content)?;

        // Add reference to parent directory
        let doc_ref = RefNode::new_document(filename.to_string(), doc_handle.document_id().clone());

        AutomergeHelpers::add_child_to_directory(&parent_handle, &doc_ref)?;

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
                    .map_err(|e| VfsError::SamodError(format!("Failed to find document: {e}")))?;
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

        // Extract filename from path
        let filename = path
            .rsplit('/')
            .next()
            .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

        // Get the parent directory path
        let parent_path = if path.contains('/') {
            let last_slash = path.rfind('/').unwrap();
            if last_slash == 0 {
                "/"
            } else {
                &path[..last_slash]
            }
        } else {
            "/"
        };

        // Traverse to parent directory
        let result = self
            .traverser
            .traverse(self.root_id.clone(), parent_path, false)
            .await?;

        // Get the actual parent directory handle
        let parent_handle = if let Some(ref target_ref) = result.target_ref {
            self.samod
                .find(target_ref.pointer.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find parent directory: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(target_ref.pointer.to_string()))?
        } else {
            result.node_handle.clone()
        };

        // Remove the child from parent directory
        let removed_ref = AutomergeHelpers::remove_child_from_directory(&parent_handle, filename)?;

        if let Some(ref removed) = removed_ref {
            // If it was a directory, we need to recursively delete all children
            if removed.node_type == NodeType::Directory {
                self.remove_directory_contents(path, &removed.pointer)
                    .await?;
            }

            // Emit event
            let _ = self.event_tx.send(VfsEvent::DocumentDeleted {
                path: path.to_string(),
            });

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Recursively remove directory contents
    fn remove_directory_contents<'a>(
        &'a self,
        parent_path: &'a str,
        dir_id: &'a DocumentId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let dir_handle = self
                .samod
                .find(dir_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find directory: {e}")))?;

            if let Some(handle) = dir_handle {
                let dir_node = AutomergeHelpers::read_directory(&handle)?;
                for child in dir_node.children {
                    let child_path = format!("{}/{}", parent_path, child.name);
                    self.remove_document(&child_path).await?;
                }
            }
            Ok(())
        })
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
                .map_err(|e| VfsError::SamodError(format!("Failed to find directory: {e}")))?;

            let dir_node = match dir_handle {
                Some(ref handle) => {
                    let node = self.dir_node(handle).await?;
                    node
                }
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

        // Extract directory name from path
        let dirname = path
            .rsplit('/')
            .next()
            .ok_or_else(|| VfsError::InvalidPath(path.to_string()))?;

        // Get the parent directory path
        let parent_path = if path.contains('/') {
            let last_slash = path.rfind('/').unwrap();
            if last_slash == 0 {
                "/"
            } else {
                &path[..last_slash]
            }
        } else {
            "/"
        };

        // Traverse to parent directory, creating directories as needed
        let result = self
            .traverser
            .traverse(self.root_id.clone(), parent_path, true)
            .await?;

        // Get the actual parent directory handle
        let parent_handle = if let Some(ref target_ref) = result.target_ref {
            self.samod
                .find(target_ref.pointer.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find parent directory: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(target_ref.pointer.to_string()))?
        } else {
            result.node_handle.clone()
        };

        // Check if directory already exists in parent
        let parent_dir = AutomergeHelpers::read_directory(&parent_handle)?;
        if parent_dir.find_child(dirname).is_some() {
            return Err(VfsError::DocumentExists(path.to_string()));
        }

        // Create the new directory document
        let new_doc = Automerge::new();
        let dir_handle = self
            .samod
            .create(new_doc)
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to create directory: {e}")))?;

        // Initialize as directory
        AutomergeHelpers::init_as_directory(&dir_handle, dirname)?;

        // Add reference to parent directory
        let dir_ref = RefNode::new_directory(dirname.to_string(), dir_handle.document_id().clone());
        AutomergeHelpers::add_child_to_directory(&parent_handle, &dir_ref)?;

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
    pub async fn metadata(&self, path: &str) -> Result<Option<(NodeType, Timestamps)>> {
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
    async fn dir_node(&self, handle: &DocHandle) -> Result<DirNode> {
        AutomergeHelpers::read_directory(handle)
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
        let result = self
            .traverser
            .traverse(self.root_id.clone(), path, false)
            .await?;

        if let Some(target_ref) = result.target_ref {
            if target_ref.node_type == NodeType::Directory {
                let dir_handle = self
                    .samod
                    .find(target_ref.pointer.clone())
                    .await
                    .map_err(|e| VfsError::SamodError(format!("Failed to find directory: {e}")))?;
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
        } else if path == "/" || path.is_empty() {
            // Watching root directory
            let root_handle = self
                .samod
                .find(self.root_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find root: {e}")))?
                .ok_or_else(|| VfsError::DocumentNotFound(self.root_id.to_string()))?;
            Ok(Some(DocumentWatcher::new(root_handle)))
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

        let _rx = vfs.subscribe_events();
        // Just test that we can subscribe
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
}
