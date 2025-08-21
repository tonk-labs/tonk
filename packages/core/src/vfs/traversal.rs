use crate::error::{Result, VfsError};
use crate::vfs::automerge_helpers::AutomergeHelpers;
use crate::vfs::types::*;
use samod::{DocHandle, DocumentId, Samod};
use std::sync::Arc;

pub struct TraverseResult {
    pub node_handle: DocHandle,
    pub node: DirNode,
    pub target_ref: Option<RefNode>,
    pub parent_path: String,
}

#[derive(Clone)]
pub struct PathTraverser {
    samod: Arc<Samod>,
}

impl PathTraverser {
    pub fn new(samod: Arc<Samod>) -> Self {
        Self { samod }
    }

    /// Traverses the document tree to find or create nodes along a path
    pub async fn traverse(
        &self,
        root_id: DocumentId,
        path: &str,
        create_missing: bool,
    ) -> Result<TraverseResult> {
        // Normalize the path: remove leading slash and trailing slashes
        let normalized_path = path.trim_start_matches('/').trim_end_matches('/');

        let segments: Vec<&str> = if normalized_path.is_empty() {
            Vec::new()
        } else {
            normalized_path
                .split('/')
                .filter(|s| !s.is_empty())
                .collect()
        };

        // No segments means root path
        if segments.is_empty() {
            let root_handle = self
                .samod
                .find(root_id.clone())
                .await
                .map_err(|e| VfsError::SamodError(format!("Failed to find root document: {e}")))?
                .ok_or_else(|| {
                    VfsError::SamodError(format!("Root document {root_id} not found"))
                })?;

            let root_doc = match AutomergeHelpers::read_directory(&root_handle) {
                Ok(doc) => doc,
                Err(_) if create_missing => {
                    // Initialize root if it doesn't exist and we're creating missing nodes
                    AutomergeHelpers::init_as_directory(&root_handle, "/")?;
                    AutomergeHelpers::read_directory(&root_handle)?
                }
                Err(e) => return Err(e),
            };

            return Ok(TraverseResult {
                node_handle: root_handle,
                node: root_doc,
                target_ref: None,
                parent_path: "".to_string(),
            });
        }

        // Navigate through the tree
        let mut current_node_id = root_id;
        let mut current_handle = self
            .samod
            .find(current_node_id.clone())
            .await
            .map_err(|e| VfsError::SamodError(format!("Failed to find root document: {e}")))?
            .ok_or_else(|| {
                VfsError::SamodError(format!("Root document {current_node_id} not found"))
            })?;

        let mut current_doc = match AutomergeHelpers::read_directory(&current_handle) {
            Ok(doc) => doc,
            Err(_) if create_missing => {
                // Initialize root if needed and we're creating missing directories
                AutomergeHelpers::init_as_directory(&current_handle, "/")?;
                AutomergeHelpers::read_directory(&current_handle)?
            }
            Err(e) => return Err(e),
        };

        let mut current_path = String::new();
        let mut last_segment_ref: Option<RefNode> = None;

        // Process each path segment
        for (i, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                continue;
            }

            let is_last_segment = i == segments.len() - 1;

            // Update current path
            current_path = if current_path.is_empty() {
                format!("/{segment}")
            } else {
                format!("{current_path}/{segment}")
            };

            // Find the child with the matching name
            let child_ref = current_doc.find_child(segment).cloned();

            if let Some(child_ref) = child_ref {
                // Child exists, follow the reference
                last_segment_ref = Some(child_ref.clone());

                // If this is the last segment, we can return without further navigation
                if is_last_segment {
                    return Ok(TraverseResult {
                        node_handle: current_handle,
                        node: current_doc,
                        target_ref: Some(child_ref),
                        parent_path: current_path.clone(),
                    });
                }

                // Only try to navigate further if it's a directory
                if child_ref.node_type == NodeType::Directory {
                    current_node_id = child_ref.pointer;
                    current_handle = self
                        .samod
                        .find(current_node_id.clone())
                        .await
                        .map_err(|e| VfsError::SamodError(format!("Failed to find document: {e}")))?
                        .ok_or_else(|| {
                            VfsError::SamodError(format!("Document {current_node_id} not found"))
                        })?;

                    current_doc = AutomergeHelpers::read_directory(&current_handle)?;
                } else {
                    // Found a document when we need to navigate further, so fail
                    return Err(VfsError::PathNotFound(current_path));
                }
            } else if create_missing && !is_last_segment {
                // Create a new directory document with samod.create()
                let new_doc = automerge::Automerge::new();
                let new_node_handle = self.samod.create(new_doc).await.map_err(|e| {
                    VfsError::SamodError(format!("Failed to create directory document: {e}"))
                })?;
                let new_node_id = new_node_handle.document_id().clone();

                // Initialize the new node as a directory
                AutomergeHelpers::init_as_directory(&new_node_handle, segment)?;

                // Add the new node to the parent's children with duplicate prevention
                let new_dir_ref = RefNode::new_directory(segment.to_string(), new_node_id.clone());

                // Check if a directory with this name already exists (prevents race condition duplicates)
                let updated_current_doc = AutomergeHelpers::read_directory(&current_handle)?;
                if updated_current_doc.find_child(segment).is_some() {
                    // Another concurrent operation already created this directory, use the existing one
                    current_doc = updated_current_doc;
                    let existing_child = current_doc.find_child(segment).unwrap();
                    current_node_id = existing_child.pointer.clone();
                    current_handle = self
                        .samod
                        .find(current_node_id.clone())
                        .await
                        .map_err(|e| VfsError::SamodError(format!("Failed to find document: {e}")))?
                        .ok_or_else(|| {
                            VfsError::SamodError(format!("Document {current_node_id} not found"))
                        })?;
                    current_doc = AutomergeHelpers::read_directory(&current_handle)?;
                } else {
                    // Add our new directory to the parent
                    AutomergeHelpers::add_child_to_directory(&current_handle, &new_dir_ref)?;

                    // Update for next iteration
                    current_handle = new_node_handle;
                    current_doc = AutomergeHelpers::read_directory(&current_handle)?;
                }

                last_segment_ref = None;

                if is_last_segment {
                    return Ok(TraverseResult {
                        node_handle: current_handle,
                        node: current_doc,
                        target_ref: None,
                        parent_path: current_path.clone(),
                    });
                }
            } else if create_missing && is_last_segment {
                // Create the final directory in the path
                let new_doc = automerge::Automerge::new();
                let new_node_handle = self.samod.create(new_doc).await.map_err(|e| {
                    VfsError::SamodError(format!("Failed to create directory document: {e}"))
                })?;
                let new_node_id = new_node_handle.document_id().clone();

                // Initialize the new node as a directory
                AutomergeHelpers::init_as_directory(&new_node_handle, segment)?;

                // Add the new node to the parent's children
                let new_dir_ref = RefNode::new_directory(segment.to_string(), new_node_id.clone());
                AutomergeHelpers::add_child_to_directory(&current_handle, &new_dir_ref)?;

                // Return the newly created directory
                let new_dir_node = AutomergeHelpers::read_directory(&new_node_handle)?;
                return Ok(TraverseResult {
                    node_handle: new_node_handle,
                    node: new_dir_node,
                    target_ref: None,
                    parent_path: current_path.clone(),
                });
            } else if is_last_segment {
                // Target doesn't exist, return parent directory for potential creation
                return Ok(TraverseResult {
                    node_handle: current_handle,
                    node: current_doc,
                    target_ref: None,
                    parent_path: current_path.clone(),
                });
            } else {
                // Node doesn't exist and we're not creating it
                return Err(VfsError::PathNotFound(current_path));
            }
        }

        // If we get here, we've reached the requested node
        Ok(TraverseResult {
            node_handle: current_handle,
            node: current_doc,
            target_ref: last_segment_ref,
            parent_path: current_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncEngine;
    use crate::vfs::operations::VirtualFileSystem;

    // Helper function to create a test VFS with initialized root
    async fn create_test_vfs() -> (Arc<Samod>, DocumentId, PathTraverser) {
        let engine = SyncEngine::new().await.unwrap();
        let vfs = VirtualFileSystem::new(engine.samod()).await.unwrap();
        let traverser = PathTraverser::new(engine.samod());
        (engine.samod(), vfs.root_id(), traverser)
    }

    // Helper function to create a directory structure for testing
    async fn setup_test_directory_structure(vfs_root: DocumentId, samod: Arc<Samod>) {
        let root_handle = samod.find(vfs_root.clone()).await.unwrap().unwrap();

        // Create /docs directory
        let docs_doc = automerge::Automerge::new();
        let docs_handle = samod.create(docs_doc).await.unwrap();
        AutomergeHelpers::init_as_directory(&docs_handle, "docs").unwrap();
        let docs_ref =
            RefNode::new_directory("docs".to_string(), docs_handle.document_id().clone());
        AutomergeHelpers::add_child_to_directory(&root_handle, &docs_ref).unwrap();

        // Create /docs/readme.txt document
        let readme_doc = automerge::Automerge::new();
        let readme_handle = samod.create(readme_doc).await.unwrap();
        AutomergeHelpers::init_as_document(&readme_handle, "readme.txt", "Hello World").unwrap();
        let readme_ref = RefNode::new_document(
            "readme.txt".to_string(),
            readme_handle.document_id().clone(),
        );
        AutomergeHelpers::add_child_to_directory(&docs_handle, &readme_ref).unwrap();

        // Create /images directory
        let images_doc = automerge::Automerge::new();
        let images_handle = samod.create(images_doc).await.unwrap();
        AutomergeHelpers::init_as_directory(&images_handle, "images").unwrap();
        let images_ref =
            RefNode::new_directory("images".to_string(), images_handle.document_id().clone());
        AutomergeHelpers::add_child_to_directory(&root_handle, &images_ref).unwrap();
    }

    #[tokio::test]
    async fn test_traverse_root_path() {
        let (_, root_id, traverser) = create_test_vfs().await;

        // Test traversing to root with various path formats
        let paths = vec!["/", "", "//", "///"];

        for path in paths {
            let result = traverser
                .traverse(root_id.clone(), path, false)
                .await
                .unwrap();
            assert_eq!(result.node.name, "/");
            assert_eq!(result.node.node_type, NodeType::Directory);
            assert!(result.target_ref.is_none());
            assert_eq!(result.parent_path, "");
        }
    }

    #[tokio::test]
    async fn test_traverse_existing_single_level_path() {
        let (samod, root_id, traverser) = create_test_vfs().await;
        setup_test_directory_structure(root_id.clone(), samod).await;

        // Test traversing to /docs
        let result = traverser
            .traverse(root_id.clone(), "/docs", false)
            .await
            .unwrap();
        assert_eq!(result.node.name, "/"); // Parent is root
        assert!(result.target_ref.is_some());
        let target_ref = result.target_ref.unwrap();
        assert_eq!(target_ref.name, "docs");
        assert_eq!(target_ref.node_type, NodeType::Directory);
        assert_eq!(result.parent_path, "/docs");
    }

    #[tokio::test]
    async fn test_traverse_existing_multi_level_path() {
        let (samod, root_id, traverser) = create_test_vfs().await;
        setup_test_directory_structure(root_id.clone(), samod.clone()).await;

        // Test traversing to /docs/readme.txt
        let result = traverser
            .traverse(root_id.clone(), "/docs/readme.txt", false)
            .await
            .unwrap();
        assert_eq!(result.node.name, "docs"); // Parent is docs directory
        assert!(result.target_ref.is_some());
        let target_ref = result.target_ref.unwrap();
        assert_eq!(target_ref.name, "readme.txt");
        assert_eq!(target_ref.node_type, NodeType::Document);
        assert_eq!(result.parent_path, "/docs/readme.txt");
    }

    #[tokio::test]
    async fn test_path_normalization() {
        let (samod, root_id, traverser) = create_test_vfs().await;
        setup_test_directory_structure(root_id.clone(), samod).await;

        // Test various path formats that should all resolve to /docs
        let paths = vec!["/docs", "docs", "/docs/", "docs/", "//docs//"];

        for path in paths {
            let result = traverser
                .traverse(root_id.clone(), path, false)
                .await
                .unwrap();
            assert!(result.target_ref.is_some());
            assert_eq!(result.target_ref.unwrap().name, "docs");
            assert_eq!(result.parent_path, "/docs");
        }
    }

    #[tokio::test]
    async fn test_create_missing_directories() {
        let (_, root_id, traverser) = create_test_vfs().await;

        // Test creating a nested directory structure
        let result = traverser
            .traverse(root_id.clone(), "/a/b/c", true)
            .await
            .unwrap();
        assert_eq!(result.node.name, "c");
        assert_eq!(result.node.node_type, NodeType::Directory);
        assert!(result.target_ref.is_none()); // New directory has no target_ref
        assert_eq!(result.parent_path, "/a/b/c");

        // Verify the structure was created by traversing without create_missing
        let verify_a = traverser
            .traverse(root_id.clone(), "/a", false)
            .await
            .unwrap();
        assert_eq!(verify_a.target_ref.unwrap().name, "a");

        let verify_b = traverser
            .traverse(root_id.clone(), "/a/b", false)
            .await
            .unwrap();
        assert_eq!(verify_b.target_ref.unwrap().name, "b");

        let verify_c = traverser
            .traverse(root_id.clone(), "/a/b/c", false)
            .await
            .unwrap();
        assert_eq!(verify_c.target_ref.unwrap().name, "c");
    }

    #[tokio::test]
    async fn test_create_final_directory_only() {
        let (samod, root_id, traverser) = create_test_vfs().await;

        // Create /existing manually
        let existing_doc = automerge::Automerge::new();
        let existing_handle = samod.create(existing_doc).await.unwrap();
        AutomergeHelpers::init_as_directory(&existing_handle, "existing").unwrap();
        let root_handle = samod.find(root_id.clone()).await.unwrap().unwrap();
        let existing_ref = RefNode::new_directory(
            "existing".to_string(),
            existing_handle.document_id().clone(),
        );
        AutomergeHelpers::add_child_to_directory(&root_handle, &existing_ref).unwrap();

        // Test creating only the final directory
        let result = traverser
            .traverse(root_id.clone(), "/existing/new", true)
            .await
            .unwrap();
        assert_eq!(result.node.name, "new");
        assert!(result.target_ref.is_none());
        assert_eq!(result.parent_path, "/existing/new");
    }

    #[tokio::test]
    async fn test_traverse_non_existent_path_no_create() {
        let (_, root_id, traverser) = create_test_vfs().await;

        // Test traversing to non-existent path without creating
        let result = traverser
            .traverse(root_id.clone(), "/does/not/exist", false)
            .await;
        assert!(result.is_err());
        match result {
            Err(VfsError::PathNotFound(path)) => assert_eq!(path, "/does"),
            _ => panic!("Expected PathNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_traverse_through_document_fails() {
        let (samod, root_id, traverser) = create_test_vfs().await;

        // Create a document at root
        let doc_doc = automerge::Automerge::new();
        let doc_handle = samod.create(doc_doc).await.unwrap();
        AutomergeHelpers::init_as_document(&doc_handle, "file.txt", "content").unwrap();
        let root_handle = samod.find(root_id.clone()).await.unwrap().unwrap();
        let doc_ref =
            RefNode::new_document("file.txt".to_string(), doc_handle.document_id().clone());
        AutomergeHelpers::add_child_to_directory(&root_handle, &doc_ref).unwrap();

        // Try to traverse through the document
        let result = traverser
            .traverse(root_id.clone(), "/file.txt/subpath", false)
            .await;
        assert!(result.is_err());
        match result {
            Err(VfsError::PathNotFound(path)) => assert_eq!(path, "/file.txt"),
            _ => panic!("Expected PathNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_missing_root_document() {
        let engine = SyncEngine::new().await.unwrap();
        let traverser = PathTraverser::new(engine.samod());
        let mut rng = rand::rng();
        let fake_id = DocumentId::new(&mut rng); // Non-existent document ID

        let result = traverser.traverse(fake_id, "/", false).await;
        assert!(result.is_err());
        match result {
            Err(VfsError::SamodError(msg)) => assert!(msg.contains("not found")),
            _ => panic!("Expected SamodError for missing root"),
        }
    }

    #[tokio::test]
    async fn test_empty_path_segments() {
        let (samod, root_id, traverser) = create_test_vfs().await;
        setup_test_directory_structure(root_id.clone(), samod).await;

        // Test paths with empty segments
        let result = traverser
            .traverse(root_id.clone(), "///docs///", false)
            .await
            .unwrap();
        assert_eq!(result.target_ref.unwrap().name, "docs");
        assert_eq!(result.parent_path, "/docs");
    }

    #[tokio::test]
    async fn test_traverse_result_validation() {
        let (samod, root_id, traverser) = create_test_vfs().await;
        setup_test_directory_structure(root_id.clone(), samod.clone()).await;

        // Test complete TraverseResult for existing path
        let result = traverser
            .traverse(root_id.clone(), "/docs", false)
            .await
            .unwrap();

        // Validate node_handle
        let handle_id = result.node_handle.document_id();
        assert_eq!(handle_id, &root_id); // Should be root's handle

        // Validate node (DirNode)
        assert_eq!(result.node.name, "/");
        assert_eq!(result.node.node_type, NodeType::Directory);
        assert!(result.node.children.len() >= 2); // At least docs and images

        // Validate target_ref
        assert!(result.target_ref.is_some());
        let target = result.target_ref.unwrap();
        assert_eq!(target.name, "docs");
        assert_eq!(target.node_type, NodeType::Directory);

        // Validate parent_path
        assert_eq!(result.parent_path, "/docs");
    }

    #[tokio::test]
    async fn test_concurrent_directory_creation_race_condition() {
        let (samod, root_id, traverser) = create_test_vfs().await;

        // Simulate race condition by creating directory between checks
        // This tests the logic in lines 158-183 of traverse()

        // Start two concurrent traversals that will both try to create /race/test
        let traverser1 = traverser.clone();
        let traverser2 = PathTraverser::new(samod.clone());
        let root_id1 = root_id.clone();
        let root_id2 = root_id.clone();

        let handle1 =
            tokio::spawn(async move { traverser1.traverse(root_id1, "/race/test", true).await });

        let handle2 =
            tokio::spawn(async move { traverser2.traverse(root_id2, "/race/test", true).await });

        // Both should succeed without errors
        let result1 = handle1.await.unwrap();
        let result2 = handle2.await.unwrap();

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        // Verify only one directory was actually created
        let verify_race = traverser
            .traverse(root_id.clone(), "/race", false)
            .await
            .unwrap();
        let race_ref = verify_race.target_ref.unwrap();
        assert_eq!(race_ref.name, "race");

        // Check that race directory has test subdirectory
        let race_handle = traverser
            .samod
            .find(race_ref.pointer.clone())
            .await
            .unwrap()
            .unwrap();
        let race_node = AutomergeHelpers::read_directory(&race_handle).unwrap();
        assert_eq!(race_node.children.len(), 1);
        assert_eq!(race_node.children[0].name, "test");
    }

    #[tokio::test]
    async fn test_special_characters_in_paths() {
        let (_, root_id, traverser) = create_test_vfs().await;

        // Test creating directories with special characters
        let special_names = vec!["test-dir", "test.dir", "test_dir", "test dir", "测试"];

        for name in special_names {
            let path = format!("/{name}");
            let result = traverser
                .traverse(root_id.clone(), &path, true)
                .await
                .unwrap();
            assert_eq!(result.node.name, name);

            // Verify it can be found again
            let verify = traverser
                .traverse(root_id.clone(), &path, false)
                .await
                .unwrap();
            assert_eq!(verify.target_ref.unwrap().name, name);
        }
    }

    #[tokio::test]
    async fn test_very_long_paths() {
        let (_, root_id, traverser) = create_test_vfs().await;

        // Create a deep directory structure (reduced from 20 to 10 for faster tests)
        let mut path = String::new();
        for i in 0..10 {
            path.push_str(&format!("/level{i}"));
        }

        let result = traverser
            .traverse(root_id.clone(), &path, true)
            .await
            .unwrap();
        assert_eq!(result.node.name, "level9");
        assert_eq!(result.parent_path, path);

        // Verify key parts of the structure were created
        let verify_start = traverser
            .traverse(root_id.clone(), "/level0", false)
            .await
            .unwrap();
        assert_eq!(verify_start.target_ref.unwrap().name, "level0");

        let verify_mid = traverser
            .traverse(
                root_id.clone(),
                "/level0/level1/level2/level3/level4",
                false,
            )
            .await
            .unwrap();
        assert_eq!(verify_mid.target_ref.unwrap().name, "level4");

        let verify_end = traverser
            .traverse(root_id.clone(), &path, false)
            .await
            .unwrap();
        assert_eq!(verify_end.target_ref.unwrap().name, "level9");
    }

    #[tokio::test]
    async fn test_uninitialized_root_with_create_missing() {
        let engine = SyncEngine::new().await.unwrap();
        let samod = engine.samod();
        let traverser = PathTraverser::new(samod.clone());

        // Create a root document but don't initialize it
        let root_doc = automerge::Automerge::new();
        let root_handle = samod.create(root_doc).await.unwrap();
        let root_id = root_handle.document_id().clone();

        // Traverse to root with create_missing=true should initialize it
        let result = traverser
            .traverse(root_id.clone(), "/", true)
            .await
            .unwrap();
        assert_eq!(result.node.name, "/");
        assert_eq!(result.node.node_type, NodeType::Directory);

        // Verify it's properly initialized by creating a child
        let child_result = traverser
            .traverse(root_id.clone(), "/child", true)
            .await
            .unwrap();
        assert_eq!(child_result.node.name, "child");
    }
}
