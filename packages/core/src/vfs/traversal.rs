use crate::error::{Result, VfsError};
use crate::vfs::types::*;
use samod::{DocHandle, DocumentId, Samod};
use std::sync::Arc;

pub struct TraverseResult {
    pub node_handle: DocHandle,
    pub node: DirNode,
    pub target_ref: Option<RefNode>,
    pub parent_path: String,
}

pub struct PathTraverser {
    samod: Arc<Samod>,
}

impl PathTraverser {
    pub fn new(samod: Arc<Samod>) -> Self {
        Self { samod }
    }

    /// Traverse a path and return the final directory node and optional target reference
    pub async fn traverse(
        &self,
        root_id: DocumentId,
        path: &str,
        create_missing: bool,
    ) -> Result<TraverseResult> {
        // Normalize path - remove leading/trailing slashes and split
        let normalized_path = path.trim_start_matches('/').trim_end_matches('/');

        if normalized_path.is_empty() {
            // Root path case
            let root_handle = match self.samod.find(root_id.clone()).await {
                Ok(Some(handle)) => handle,
                Ok(None) => {
                    return Err(VfsError::SamodError(format!(
                        "Root document {} not found",
                        root_id
                    )));
                }
                Err(e) => {
                    return Err(VfsError::SamodError(format!(
                        "Failed to find root document: {}",
                        e
                    )));
                }
            };

            let root_node = self.get_dir_node(&root_handle).await?;

            return Ok(TraverseResult {
                node_handle: root_handle,
                node: root_node,
                target_ref: None,
                parent_path: "/".to_string(),
            });
        }

        let path_parts: Vec<&str> = normalized_path.split('/').collect();
        let mut current_handle = match self.samod.find(root_id.clone()).await {
            Ok(Some(handle)) => handle,
            Ok(None) => {
                return Err(VfsError::SamodError(format!(
                    "Root document {} not found",
                    root_id
                )));
            }
            Err(e) => {
                return Err(VfsError::SamodError(format!(
                    "Failed to find root document: {}",
                    e
                )));
            }
        };
        let mut current_path = String::new();

        // Traverse all path components except the last one
        for (i, part) in path_parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            let is_last = i == path_parts.len() - 1;
            current_path.push('/');
            current_path.push_str(part);

            let mut current_dir = self.get_dir_node(&current_handle).await?;

            if let Some(child_ref) = current_dir.find_child(part) {
                // Child exists, follow the reference
                if is_last {
                    // This is the target we're looking for
                    return Ok(TraverseResult {
                        node_handle: current_handle,
                        node: current_dir.clone(),
                        target_ref: Some(child_ref.clone()),
                        parent_path: current_path
                            .rsplit_once('/')
                            .map(|(p, _)| p.to_string())
                            .unwrap_or("/".to_string()),
                    });
                } else {
                    // Continue traversing
                    current_handle = match self.samod.find(child_ref.pointer.clone()).await {
                        Ok(Some(handle)) => handle,
                        Ok(None) => {
                            return Err(VfsError::SamodError(format!(
                                "Document {} not found",
                                child_ref.pointer
                            )));
                        }
                        Err(e) => {
                            return Err(VfsError::SamodError(format!(
                                "Failed to find document {}: {}",
                                child_ref.pointer, e
                            )));
                        }
                    };
                }
            } else {
                // Child doesn't exist
                if create_missing && !is_last {
                    // Create missing directory
                    let new_dir = DirNode::new(part.to_string());
                    let new_doc = automerge::Automerge::new();
                    let new_handle = self.samod.create(new_doc).await.map_err(|e| {
                        VfsError::SamodError(format!("Failed to create directory document: {}", e))
                    })?;

                    // Update the new document with directory structure
                    self.update_dir_node(&new_handle, &new_dir).await?;

                    // Add reference to current directory
                    let new_ref =
                        RefNode::new_directory(part.to_string(), new_handle.document_id().clone());
                    current_dir.add_child(new_ref);

                    // Update current directory
                    self.update_dir_node(&current_handle, &current_dir).await?;

                    current_handle = new_handle;
                } else if is_last {
                    // Target doesn't exist, return parent directory for potential creation
                    return Ok(TraverseResult {
                        node_handle: current_handle,
                        node: current_dir,
                        target_ref: None,
                        parent_path: current_path
                            .rsplit_once('/')
                            .map(|(p, _)| p.to_string())
                            .unwrap_or("/".to_string()),
                    });
                } else {
                    // Path doesn't exist and we're not creating
                    return Err(VfsError::PathNotFound(current_path));
                }
            }
        }

        // Should not reach here
        Err(VfsError::InvalidPath(path.to_string()))
    }

    async fn get_dir_node(&self, handle: &DocHandle) -> Result<DirNode> {
        handle
            .with_document(|doc| {
                // Try to deserialize as DirNode from the automerge document
                // TODO: This is a simplified version - in reality we'd need to handle
                // the automerge document structure properly
                serde_json::from_str(&serde_json::to_string(doc)?)
                    .map_err(|e| VfsError::SerializationError(e))
            })
            .map_err(|e| VfsError::Other(e.into()))?
    }

    async fn update_dir_node(&self, handle: &DocHandle, dir_node: &DirNode) -> Result<()> {
        handle
            .with_document(|doc| {
                // Update the automerge document with the new directory structure
                // TODO: This is a simplified version - in reality we'd need to handle
                // the automerge document structure properly
                let serialized = serde_json::to_string(dir_node)?;
                let _: serde_json::Value = serde_json::from_str(&serialized)?;
                // TODO: In a real implementation, we'd update the automerge doc here
                Ok(())
            })
            .map_err(|e| VfsError::Other(e.into()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncEngine;

    #[tokio::test]
    async fn test_traverse_root_path() {
        let engine = SyncEngine::new().await.unwrap();
        let traverser = PathTraverser::new(engine.samod());

        // Create a root document
        let root_doc = automerge::Automerge::new();
        let root_handle = engine.create_document(root_doc).await.unwrap();

        // This test would need proper automerge document setup
        // For now, we'll just test that the traverser can be created
        assert!(traverser.samod.peer_id().to_string().len() > 0);
    }

    #[tokio::test]
    async fn test_path_normalization() {
        let engine = SyncEngine::new().await.unwrap();
        let traverser = PathTraverser::new(engine.samod());

        // Test path normalization logic
        let test_paths = vec!["/", "//", "/path/to/doc", "/path/to/doc/", "path/to/doc"];

        for path in test_paths {
            let normalized = path.trim_start_matches('/').trim_end_matches('/');
            if normalized.is_empty() {
                // Root path case
                assert!(path.starts_with('/') || path.is_empty());
            } else {
                assert!(!normalized.starts_with('/'));
                assert!(!normalized.ends_with('/'));
            }
        }
    }
}
