use super::types::NodeType;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Path index is source of truth for VFS structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathIndex {
    /// Direct path-to-document-id mapping
    /// Keys are absolute paths like "/counter-state.json" or "/app/data/config.json"
    pub paths: HashMap<String, PathEntry>,

    /// Last update timestamp for conflict resolution
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub last_updated: DateTime<Utc>,
}

/// Entry for each path in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathEntry {
    /// Document ID in Samod
    pub doc_id: String,

    /// Type of node (document or directory)
    #[serde(rename = "type")]
    pub node_type: NodeType,

    /// Created timestamp
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub created: DateTime<Utc>,

    /// Modified timestamp
    #[serde(with = "chrono::serde::ts_milliseconds")]
    pub modified: DateTime<Utc>,
}

impl PathIndex {
    pub fn new() -> Self {
        Self {
            paths: HashMap::new(),
            last_updated: Utc::now(),
        }
    }

    /// Add or update a path mapping
    pub fn set_path(&mut self, path: String, doc_id: String, node_type: NodeType) {
        let now = Utc::now();

        if let Some(entry) = self.paths.get_mut(&path) {
            // Update existing
            entry.doc_id = doc_id;
            entry.node_type = node_type;
            entry.modified = now;
        } else {
            // Create new
            self.paths.insert(
                path,
                PathEntry {
                    doc_id,
                    node_type,
                    created: now,
                    modified: now,
                },
            );
        }

        self.last_updated = now;
    }

    /// Remove a path mapping
    pub fn remove_path(&mut self, path: &str) -> Option<PathEntry> {
        let result = self.paths.remove(path);
        if result.is_some() {
            self.last_updated = Utc::now();
        }
        result
    }

    /// Get path entry (includes doc_id and metadata)
    pub fn get_entry(&self, path: &str) -> Option<&PathEntry> {
        self.paths.get(path)
    }

    /// Get document ID for a path
    pub fn get_doc_id(&self, path: &str) -> Option<&String> {
        self.paths.get(path).map(|e| &e.doc_id)
    }

    /// Check if path exists
    pub fn has_path(&self, path: &str) -> bool {
        self.paths.contains_key(path)
    }

    /// List all children of a directory path
    pub fn list_children(&self, dir_path: &str) -> Vec<(String, &PathEntry)> {
        let normalized_dir = dir_path.trim_end_matches('/');
        let prefix = if normalized_dir.is_empty() || normalized_dir == "/" {
            "/"
        } else {
            normalized_dir
        };

        self.paths
            .iter()
            .filter_map(|(path, entry)| {
                // Only direct children, not nested
                if path.starts_with(prefix) && path != prefix {
                    let remainder = if prefix == "/" {
                        &path[1..]
                    } else {
                        &path[prefix.len() + 1..]
                    };

                    // Check it's a direct child (no more slashes)
                    if !remainder.contains('/') {
                        return Some((path.clone(), entry));
                    }
                }
                None
            })
            .collect()
    }

    /// Get all paths
    pub fn all_paths(&self) -> Vec<&String> {
        self.paths.keys().collect()
    }

    /// Move a path (for rename/move operations)
    pub fn move_path(&mut self, from_path: &str, to_path: &str) -> Result<(), String> {
        if let Some(mut entry) = self.paths.remove(from_path) {
            entry.modified = Utc::now();
            self.paths.insert(to_path.to_string(), entry);
            self.last_updated = Utc::now();
            Ok(())
        } else {
            Err(format!("Path not found: {}", from_path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_path_index() {
        let index = PathIndex::new();
        assert_eq!(index.paths.len(), 0);
        assert!(index.all_paths().is_empty());
    }

    #[test]
    fn test_set_path() {
        let mut index = PathIndex::new();

        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        assert_eq!(index.paths.len(), 1);
        assert!(index.has_path("/test.json"));
        assert_eq!(index.get_doc_id("/test.json"), Some(&"doc123".to_string()));
    }

    #[test]
    fn test_update_existing_path() {
        let mut index = PathIndex::new();

        // Create initial entry
        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        let original_created = index.get_entry("/test.json").unwrap().created;

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Update existing entry
        index.set_path(
            "/test.json".to_string(),
            "doc456".to_string(),
            NodeType::Document,
        );

        assert_eq!(index.paths.len(), 1);
        assert_eq!(index.get_doc_id("/test.json"), Some(&"doc456".to_string()));

        let entry = index.get_entry("/test.json").unwrap();
        assert_eq!(entry.created, original_created);
        assert!(entry.modified > original_created);
    }

    #[test]
    fn test_remove_path() {
        let mut index = PathIndex::new();

        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        assert!(index.has_path("/test.json"));

        let removed = index.remove_path("/test.json");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().doc_id, "doc123");
        assert!(!index.has_path("/test.json"));
    }

    #[test]
    fn test_remove_nonexistent_path() {
        let mut index = PathIndex::new();
        let removed = index.remove_path("/nonexistent.json");
        assert!(removed.is_none());
    }

    #[test]
    fn test_get_entry() {
        let mut index = PathIndex::new();

        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        let entry = index.get_entry("/test.json");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().doc_id, "doc123");
        assert_eq!(entry.unwrap().node_type, NodeType::Document);
    }

    #[test]
    fn test_list_children_root() {
        let mut index = PathIndex::new();

        // Add files and directories at root
        index.set_path(
            "/file1.json".to_string(),
            "doc1".to_string(),
            NodeType::Document,
        );
        index.set_path(
            "/file2.json".to_string(),
            "doc2".to_string(),
            NodeType::Document,
        );
        index.set_path("/dir1".to_string(), "doc3".to_string(), NodeType::Directory);
        index.set_path(
            "/dir1/nested.json".to_string(),
            "doc4".to_string(),
            NodeType::Document,
        );

        let children = index.list_children("/");
        assert_eq!(children.len(), 3); // file1, file2, dir1 (not nested.json)

        let names: Vec<String> = children.iter().map(|(path, _)| path.clone()).collect();
        assert!(names.contains(&"/file1.json".to_string()));
        assert!(names.contains(&"/file2.json".to_string()));
        assert!(names.contains(&"/dir1".to_string()));
        assert!(!names.contains(&"/dir1/nested.json".to_string()));
    }

    #[test]
    fn test_list_children_subdirectory() {
        let mut index = PathIndex::new();

        // Add nested structure
        index.set_path("/app".to_string(), "doc1".to_string(), NodeType::Directory);
        index.set_path(
            "/app/data".to_string(),
            "doc2".to_string(),
            NodeType::Directory,
        );
        index.set_path(
            "/app/config.json".to_string(),
            "doc3".to_string(),
            NodeType::Document,
        );
        index.set_path(
            "/app/data/file.json".to_string(),
            "doc4".to_string(),
            NodeType::Document,
        );

        let children = index.list_children("/app");
        assert_eq!(children.len(), 2); // data and config.json (not file.json)

        let names: Vec<String> = children.iter().map(|(path, _)| path.clone()).collect();
        assert!(names.contains(&"/app/data".to_string()));
        assert!(names.contains(&"/app/config.json".to_string()));
        assert!(!names.contains(&"/app/data/file.json".to_string()));
    }

    #[test]
    fn test_list_children_empty_directory() {
        let mut index = PathIndex::new();

        index.set_path(
            "/empty".to_string(),
            "doc1".to_string(),
            NodeType::Directory,
        );

        let children = index.list_children("/empty");
        assert_eq!(children.len(), 0);
    }

    #[test]
    fn test_move_path() {
        let mut index = PathIndex::new();

        index.set_path(
            "/old.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        let result = index.move_path("/old.json", "/new.json");
        assert!(result.is_ok());

        assert!(!index.has_path("/old.json"));
        assert!(index.has_path("/new.json"));
        assert_eq!(index.get_doc_id("/new.json"), Some(&"doc123".to_string()));
    }

    #[test]
    fn test_move_nonexistent_path() {
        let mut index = PathIndex::new();

        let result = index.move_path("/nonexistent.json", "/new.json");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Path not found: /nonexistent.json".to_string()
        );
    }

    #[test]
    fn test_all_paths() {
        let mut index = PathIndex::new();

        index.set_path(
            "/file1.json".to_string(),
            "doc1".to_string(),
            NodeType::Document,
        );
        index.set_path(
            "/file2.json".to_string(),
            "doc2".to_string(),
            NodeType::Document,
        );
        index.set_path(
            "/dir/file3.json".to_string(),
            "doc3".to_string(),
            NodeType::Document,
        );

        let all = index.all_paths();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_node_type() {
        let mut index = PathIndex::new();

        index.set_path(
            "/file.json".to_string(),
            "doc1".to_string(),
            NodeType::Document,
        );
        index.set_path("/dir".to_string(), "doc2".to_string(), NodeType::Directory);

        let file_entry = index.get_entry("/file.json").unwrap();
        let dir_entry = index.get_entry("/dir").unwrap();

        assert_eq!(file_entry.node_type, NodeType::Document);
        assert_eq!(dir_entry.node_type, NodeType::Directory);
    }

    #[test]
    fn test_serialization() {
        let mut index = PathIndex::new();

        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        // Serialize to JSON
        let json = serde_json::to_string(&index).expect("Failed to serialize");

        // Deserialize back
        let deserialized: PathIndex = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.paths.len(), 1);
        assert!(deserialized.has_path("/test.json"));
        assert_eq!(
            deserialized.get_doc_id("/test.json"),
            Some(&"doc123".to_string())
        );
    }

    #[test]
    fn test_timestamps_update() {
        let mut index = PathIndex::new();
        let initial_time = index.last_updated;

        std::thread::sleep(std::time::Duration::from_millis(10));

        index.set_path(
            "/test.json".to_string(),
            "doc123".to_string(),
            NodeType::Document,
        );

        assert!(index.last_updated > initial_time);
    }
}
