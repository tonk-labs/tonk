use anyhow::{Context, Result};
use automerge::transaction::Transactable;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::BundlePath;

/// Version information for the bundle
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

/// Manifest structure for bundle metadata
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Manifest {
    #[serde(rename = "manifestVersion")]
    pub manifest_version: u32,
    pub version: Version,
    pub root: String,
    pub entrypoints: Vec<String>,
    #[serde(rename = "networkUris")]
    pub network_uris: Vec<String>,
    #[serde(default, rename = "xNotes")]
    pub x_notes: Option<String>,
    #[serde(default, rename = "xVendor")]
    pub x_vendor: Option<serde_json::Value>,
}

/// Trait for random access to data sources with read and write capabilities.
///
/// This trait provides a unified interface for working with seekable, readable, and
/// writable data sources such as files or in-memory buffers. It extends the standard
/// library traits with additional convenience methods for common operations.
///
/// # Examples
///
/// ```no_run
/// # use tonk_core::bundle::RandomAccess;
/// # use std::io::Cursor;
/// let mut data = Cursor::new(vec![1, 2, 3, 4, 5]);
/// let _pos = data.position();
/// let _result = data.seek_to(2);
/// ```
pub trait RandomAccess: Read + Write + Seek + Send + std::fmt::Debug {
    /// Get the current position in the stream.
    ///
    /// # Returns
    /// The current position as bytes from the beginning of the stream.
    ///
    /// # Errors
    /// Returns an error if the position cannot be determined.
    fn position(&mut self) -> Result<u64> {
        self.stream_position().context("Failed to get position")
    }

    /// Seek to a specific position from the start of the stream.
    ///
    /// # Arguments
    /// * `pos` - The position to seek to, in bytes from the start
    ///
    /// # Errors
    /// Returns an error if the seek operation fails.
    fn seek_to(&mut self, pos: u64) -> Result<()> {
        self.seek(SeekFrom::Start(pos))
            .with_context(|| format!("Failed to seek to position {pos}"))?;
        Ok(())
    }

    /// Read exact number of bytes at current position
    fn read_exact_at(&mut self, buf: &mut [u8]) -> Result<()> {
        self.read_exact(buf).context("Failed to read exact bytes")
    }

    /// Write bytes at current position
    fn write_at(&mut self, data: &[u8]) -> Result<()> {
        self.write_all(data).context("Failed to write bytes")
    }

    /// Flush any buffered writes
    fn flush(&mut self) -> Result<()> {
        Write::flush(self).context("Failed to flush")
    }

    /// Get total size if available
    fn size(&mut self) -> Result<Option<u64>> {
        let current = self.position()?;
        match self.seek(SeekFrom::End(0)) {
            Ok(size) => {
                self.seek_to(current)?;
                Ok(Some(size))
            }
            Err(_) => Ok(None),
        }
    }
}

// Blanket implementation for types that implement the required traits
impl<T> RandomAccess for T where T: Read + Write + Seek + Send + std::fmt::Debug {}

/// Metadata for a ZIP entry stored in our index
#[derive(Debug, Clone)]
pub struct EntryMetadata {
    /// Path within the ZIP file
    pub path: String,
    /// Offset of local file header in ZIP
    pub local_header_offset: u64,
    /// Compressed size
    pub compressed_size: u64,
    /// Uncompressed size  
    pub uncompressed_size: u64,
    /// CRC32 checksum
    pub crc32: u32,
    /// Compression method
    pub compression_method: u16,
}

/// Tree node for efficient path-based lookups
#[derive(Debug)]
struct PathTreeNode {
    /// Child nodes indexed by path component
    children: HashMap<String, PathTreeNode>,
    /// Full paths of entries that end at this node
    entries: Vec<String>,
}

impl PathTreeNode {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            entries: Vec::new(),
        }
    }

    /// Insert a path into the tree
    fn insert(&mut self, path_components: &[String], full_path: String) {
        if path_components.is_empty() {
            self.entries.push(full_path);
            return;
        }

        let component = &path_components[0];
        let child = self
            .children
            .entry(component.clone())
            .or_insert_with(PathTreeNode::new);
        child.insert(&path_components[1..], full_path);
    }

    /// Get all entry paths that match the given prefix
    fn prefix_paths(&self, prefix_components: &[String]) -> Vec<&String> {
        if prefix_components.is_empty() {
            // Return all paths from this subtree
            return self.collect_all_paths();
        }

        let component = &prefix_components[0];
        if let Some(child) = self.children.get(component) {
            child.prefix_paths(&prefix_components[1..])
        } else {
            Vec::new()
        }
    }

    /// Collect all paths from this subtree (recursive)
    fn collect_all_paths(&self) -> Vec<&String> {
        let mut paths = Vec::new();

        // Add paths that end at this node
        paths.extend(self.entries.iter());

        // Recursively collect from children
        for child in self.children.values() {
            paths.extend(child.collect_all_paths());
        }

        paths
    }

    /// Remove a path from the tree
    fn remove_path(&mut self, path_components: &[String], full_path: &str) {
        if path_components.is_empty() {
            self.entries.retain(|p| p != full_path);
            return;
        }

        if let Some(child) = self.children.get_mut(&path_components[0]) {
            child.remove_path(&path_components[1..], full_path);

            // Clean up empty nodes
            if child.entries.is_empty() && child.children.is_empty() {
                self.children.remove(&path_components[0]);
            }
        }
    }
}

/// In-memory index of ZIP entries for fast access
#[derive(Debug)]
pub struct BundleIndex {
    /// Map from path to entry metadata
    entries: HashMap<String, EntryMetadata>,
    /// Tree structure for efficient prefix lookups
    path_tree: PathTreeNode,
}

impl BundleIndex {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            path_tree: PathTreeNode::new(),
        }
    }

    /// Add an entry to the index
    pub fn add_entry(&mut self, metadata: EntryMetadata) {
        let path = metadata.path.clone();

        // Add to entries map
        self.entries.insert(path.clone(), metadata);

        // Add to path tree - filter out empty components (directories ending with /)
        let path_components: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        self.path_tree.insert(&path_components, path);
    }

    /// Get entry metadata by path
    pub fn entry(&self, path: &str) -> Option<&EntryMetadata> {
        self.entries.get(path)
    }

    /// Get all entries matching a prefix
    pub fn prefix_entries(&self, prefix: &str) -> Vec<&EntryMetadata> {
        let prefix_components: Vec<String> = if prefix.is_empty() {
            Vec::new()
        } else {
            prefix
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        };

        let matching_paths = self.path_tree.prefix_paths(&prefix_components);
        matching_paths
            .iter()
            .filter_map(|path| self.entries.get(*path))
            .collect()
    }

    /// Get all entry paths
    pub fn all_paths(&self) -> Vec<&String> {
        self.entries.keys().collect()
    }

    /// Remove a path from the path tree
    pub fn remove_from_path_tree(&mut self, path: &str) {
        let path_components: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        self.path_tree.remove_path(&path_components, path);
    }
}

impl Default for BundleIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct Bundle<R: RandomAccess> {
    /// Random access data source
    data_source: R,
    /// In-memory index of ZIP entries
    index: BundleIndex,
    /// Parsed manifest data
    manifest: Manifest,
}

impl<R: RandomAccess> Bundle<R> {
    /// Create a new bundle from a random access source
    pub fn from_source(mut data_source: R) -> Result<Self> {
        // Read the central directory and build our index
        let index = Self::build_index(&mut data_source)?;

        // Read and parse the manifest
        let manifest = Self::read_manifest(&mut data_source, &index)?;

        let bundle = Bundle {
            data_source,
            index,
            manifest,
        };

        Ok(bundle)
    }

    /// Helper function to create a ZipArchive from the data source
    fn create_archive(&mut self) -> Result<ZipArchive<&mut R>> {
        self.data_source.seek_to(0)?;
        ZipArchive::new(&mut self.data_source).context("Failed to create zip archive")
    }

    /// Build the index by reading the ZIP central directory
    fn build_index(data_source: &mut R) -> Result<BundleIndex> {
        // Reset to the beginning to ensure we can read the ZIP structure properly
        data_source.seek_to(0)?;

        // Use the zip crate to read the central directory
        let mut archive = ZipArchive::new(data_source).context("Failed to open zip archive")?;

        let mut index = BundleIndex::new();

        // Read each entry from the central directory
        for i in 0..archive.len() {
            let file = archive.by_index(i).context("Failed to read zip entry")?;

            // Skip directory entries (paths ending with '/' are typically directories)
            if file.is_dir() {
                continue;
            }

            let metadata = EntryMetadata {
                path: file.name().to_string(),
                local_header_offset: file.header_start(),
                compressed_size: file.compressed_size(),
                uncompressed_size: file.size(),
                crc32: file.crc32(),
                compression_method: match file.compression() {
                    zip::CompressionMethod::Stored => 0,
                    zip::CompressionMethod::Deflated => 8,
                    _ => 0, // Default to stored for unknown methods
                },
            };

            index.add_entry(metadata);
        }

        Ok(index)
    }

    /// Get the root Automerge document from the bundle
    pub fn root_document(&mut self) -> Result<automerge::Automerge> {
        // Get the root path from the manifest
        let root_path = self.manifest.root.clone();

        // Read the document bytes from the bundle
        let doc_bytes = self
            .get(&BundlePath::from(root_path.as_str()))?
            .ok_or_else(|| anyhow::anyhow!("Root document not found in bundle"))?;

        // Load the Automerge document from bytes
        let doc = automerge::Automerge::load(&doc_bytes).context("Failed to load root document")?;

        Ok(doc)
    }

    /// Read a value by key
    pub fn get(&mut self, key: &BundlePath) -> Result<Option<Vec<u8>>> {
        let path = key.to_string();

        // Check if file exists in the index
        if let Some(metadata) = self.index.entry(&path).cloned() {
            self.read_entry_data(&metadata)
        } else {
            Ok(None)
        }
    }

    /// Read the actual data for a ZIP entry
    fn read_entry_data(&mut self, metadata: &EntryMetadata) -> Result<Option<Vec<u8>>> {
        let mut archive = self.create_archive()?;

        let mut file = archive
            .by_name(&metadata.path)
            .context("Failed to find entry in zip")?;

        let mut buffer = Vec::with_capacity(metadata.uncompressed_size as usize);
        file.read_to_end(&mut buffer)
            .context("Failed to read entry data")?;

        Ok(Some(buffer))
    }

    /// Read all key-value pairs that match a key prefix
    pub fn prefix(&mut self, prefix: &BundlePath) -> Result<Vec<(BundlePath, Vec<u8>)>> {
        let prefix_path = prefix.to_string();
        let entries: Vec<EntryMetadata> = self
            .index
            .prefix_entries(&prefix_path)
            .into_iter()
            .cloned()
            .collect();

        let mut results = Vec::new();

        for metadata in entries {
            let path = &metadata.path;

            // Convert path back to BundlePath
            let key = BundlePath::from(path.as_str());

            // Read the data
            if let Some(data) = self.read_entry_data(&metadata)? {
                results.push((key, data));
            }
        }

        Ok(results)
    }

    /// Get all keys in the bundle
    pub fn list_keys(&self) -> Vec<BundlePath> {
        self.index
            .all_paths()
            .into_iter()
            .map(|path| BundlePath::from(path.as_str()))
            .collect()
    }

    /// Get the parsed manifest data
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Read and parse the manifest.json file from the bundle
    fn read_manifest(data_source: &mut R, index: &BundleIndex) -> Result<Manifest> {
        // Check that manifest.json exists in the bundle
        index
            .entry("manifest.json")
            .ok_or_else(|| anyhow::anyhow!("manifest.json not found in bundle"))?;

        // Reset to the beginning to ensure ZipArchive can read the central directory
        data_source.seek_to(0)?;

        // Create a temporary ZipArchive to read the manifest entry
        let mut archive = ZipArchive::new(data_source)
            .context("Failed to create zip archive for manifest reading")?;

        let mut manifest_file = archive
            .by_name("manifest.json")
            .context("Failed to find manifest.json in zip")?;

        let mut manifest_content = String::new();
        manifest_file
            .read_to_string(&mut manifest_content)
            .context("Failed to read manifest.json content")?;

        // Parse the JSON
        let manifest: Manifest =
            serde_json::from_str(&manifest_content).context("Failed to parse manifest.json")?;

        // Validate manifest version
        if manifest.manifest_version != 1 {
            return Err(anyhow::anyhow!(
                "Unsupported manifest version: {}. Expected version 1.",
                manifest.manifest_version
            ));
        }

        Ok(manifest)
    }
}

// Convenience constructors for common cases
impl Bundle<std::io::Cursor<Vec<u8>>> {
    /// Load a bundle from a byte array
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let cursor = std::io::Cursor::new(data);
        Self::from_source(cursor)
    }

    /// Create a new empty bundle with a minimal manifest
    pub fn create_empty() -> Result<Self> {
        // Create and initialize root document as directory
        let mut root_doc = automerge::Automerge::new();

        // Initialize as directory
        // TODO: way to reuse AutomergeHelpers::init_as_directory here?
        {
            let mut tx = root_doc.transaction();
            tx.put(automerge::ROOT, "type", "dir")?;
            tx.put(automerge::ROOT, "name", "/")?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            tx.put_object(automerge::ROOT, "children", automerge::ObjType::List)?;

            tx.commit();
        }

        // Serialize the root doc
        let root_doc_bytes = root_doc.save();

        // Create minimal manifest
        let manifest = serde_json::json!({
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "root",
            "entrypoints": [],
            "networkUris": []
        });

        let manifest_json =
            serde_json::to_string_pretty(&manifest).context("Failed to serialize manifest")?;

        // Create in-memory ZIP with just the manifest
        let mut zip_data = Vec::new();
        {
            let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

            // Add manifest
            zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
            zip_writer.write_all(manifest_json.as_bytes())?;

            // Add root document
            zip_writer.start_file("root", SimpleFileOptions::default())?;
            zip_writer.write_all(&root_doc_bytes)?;

            zip_writer.finish()?;
        }

        // Create bundle from the new ZIP data
        Self::from_bytes(zip_data)
    }

    /// Get the bundle data as bytes (for serialization)
    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        // Read all data from our cursor
        self.data_source.seek_to(0)?;
        let mut bytes = Vec::new();
        self.data_source
            .read_to_end(&mut bytes)
            .context("Failed to read bundle data")?;
        Ok(bytes)
    }
}

impl Bundle<std::fs::File> {
    /// Load a bundle from a file path
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self> {
        use std::fs::OpenOptions;

        // Open the file with read+write permissions to support both reading and writing operations
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context("Failed to open bundle file with read+write permissions")?;
        Self::from_source(file)
    }
}

// Implement for any Read + Write + Seek source
impl<T: Read + Write + Seek + Send + std::fmt::Debug> Bundle<T> {
    /// Load a bundle from any readable, writable and seekable source
    pub fn from_stream(stream: T) -> Result<Self> {
        Self::from_source(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Create a bundle with valid manifest for testing - returns the ZIP data as bytes
    fn create_test_bundle_with_manifest() -> Result<Vec<u8>> {
        let mut zip_data = Vec::new();
        let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

        // Create the manifest.json content
        let manifest_content = r#"{
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "main",
            "entrypoints": ["bin/myapp", "bin/worker", "scripts/setup.sh"],
            "networkUris": [
                "https://api.example.com/v1",
                "wss://realtime.example.com/socket"
            ],
            "xNotes": "Test bundle",
            "xVendor": { "featureFlag": true }
        }"#;

        // Add manifest.json
        zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
        zip_writer.write_all(manifest_content.as_bytes())?;

        // Add test files
        zip_writer.start_file("test_file.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Hello from test bundle!")?;

        zip_writer.start_file("docs/readme.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Bundle documentation")?;

        zip_writer.finish()?;
        Ok(zip_data)
    }

    /// Create a complete test bundle with a variety of files - returns the ZIP data as bytes
    fn create_complete_test_bundle() -> Result<Vec<u8>> {
        let mut zip_data = Vec::new();
        let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

        // Create the manifest.json content
        let manifest_content = r#"{
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "main",
            "entrypoints": ["bin/myapp"],
            "networkUris": []
        }"#;

        // Add manifest.json
        zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
        zip_writer.write_all(manifest_content.as_bytes())?;

        // Root level files
        zip_writer.start_file("welcome.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Hello from the root directory!")?;

        zip_writer.start_file("readme.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"This is a sample collection of text files.")?;

        // Documents directory
        zip_writer.start_file("documents/report.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Quarterly results look promising.")?;

        zip_writer.start_file("documents/summary.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Executive summary complete.")?;

        // Notes directory
        zip_writer.start_file("notes/todo.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Remember to water the plants.")?;

        zip_writer.start_file("notes/ideas.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Build something amazing today!")?;

        // Misc directory with nested structure
        zip_writer.start_file("misc/data.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Random data goes here.")?;

        zip_writer.start_file("misc/subfolder/nested.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"Deep inside the folder structure!")?;

        zip_writer.start_file(
            "misc/subfolder/hidden_message.txt",
            SimpleFileOptions::default(),
        )?;
        zip_writer.write_all(b"You found the secret message!")?;

        zip_writer.finish()?;
        Ok(zip_data)
    }

    /// Create a bundle with invalid manifest version for testing
    fn create_invalid_manifest_bundle() -> Result<Vec<u8>> {
        let mut zip_data = Vec::new();
        let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

        let manifest_content = r#"{
            "manifestVersion": 2,
            "version": { "major": 1, "minor": 0 },
            "root": "main",
            "entrypoints": ["bin/myapp"],
            "networkUris": []
        }"#;

        zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
        zip_writer.write_all(manifest_content.as_bytes())?;
        zip_writer.finish()?;

        Ok(zip_data)
    }

    /// Create a bundle without manifest.json for testing error cases
    fn create_bundle_without_manifest() -> Result<Vec<u8>> {
        let mut zip_data = Vec::new();
        let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

        zip_writer.start_file("some_file.txt", SimpleFileOptions::default())?;
        zip_writer.write_all(b"This bundle has no manifest")?;
        zip_writer.finish()?;

        Ok(zip_data)
    }

    #[test]
    fn test_load_bundle_without_manifest() {
        // Create a bundle without manifest.json - should fail
        let zip_data = create_bundle_without_manifest().expect("Failed to create test bundle");
        let result = Bundle::from_bytes(zip_data);

        assert!(
            result.is_err(),
            "Expected error when loading bundle without manifest.json"
        );
        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("manifest.json not found in bundle"));
    }

    #[test]
    fn test_load_bundle_with_manifest() {
        // Create a bundle with valid manifest
        let zip_data = create_test_bundle_with_manifest().expect("Failed to create test bundle");
        let bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle with manifest");

        // Verify manifest was parsed correctly
        let manifest = bundle.manifest();
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.version.major, 1);
        assert_eq!(manifest.version.minor, 0);
        assert_eq!(manifest.root, "main");
        assert_eq!(manifest.entrypoints.len(), 3);
        assert_eq!(manifest.entrypoints[0], "bin/myapp");
        assert_eq!(manifest.entrypoints[1], "bin/worker");
        assert_eq!(manifest.entrypoints[2], "scripts/setup.sh");
        assert_eq!(manifest.network_uris.len(), 2);
        assert_eq!(manifest.network_uris[0], "https://api.example.com/v1");
        assert_eq!(
            manifest.network_uris[1],
            "wss://realtime.example.com/socket"
        );
        assert_eq!(manifest.x_notes.as_ref().unwrap(), "Test bundle");

        // Verify we can access the files
        let keys = bundle.list_keys();
        assert_eq!(keys.len(), 3); // manifest.json, test_file.txt, docs/readme.txt
    }

    #[test]
    fn test_manifest_version_validation() {
        // Create a bundle with an invalid manifest version
        let zip_data =
            create_invalid_manifest_bundle().expect("Failed to create invalid manifest bundle");

        // Try to load the bundle - should fail due to unsupported manifest version
        let result = Bundle::from_bytes(zip_data);
        assert!(
            result.is_err(),
            "Expected error for unsupported manifest version"
        );

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Unsupported manifest version: 2"));
    }

    #[test]
    fn test_read_root_files() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Test reading welcome.txt
        let welcome_data = bundle
            .get(&BundlePath::from("welcome.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(welcome_data).unwrap(),
            "Hello from the root directory!"
        );

        // Test reading readme.txt
        let readme_data = bundle
            .get(&BundlePath::from("readme.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(readme_data).unwrap(),
            "This is a sample collection of text files."
        );
    }

    #[test]
    fn test_seek_to_nested_files() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Test seeking to documents/report.txt
        let report_data = bundle
            .get(&BundlePath::from("documents/report.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(report_data).unwrap(),
            "Quarterly results look promising."
        );

        // Test seeking to notes/todo.txt
        let todo_data = bundle
            .get(&BundlePath::from("notes/todo.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(todo_data).unwrap(),
            "Remember to water the plants."
        );

        // Test seeking to deeply nested file
        let nested_data = bundle
            .get(&BundlePath::from("misc/subfolder/nested.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(nested_data).unwrap(),
            "Deep inside the folder structure!"
        );
    }

    #[test]
    fn test_random_access_seeking() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Read files in non-sequential order to test seeking
        let files_to_test = vec![
            (
                BundlePath::from("misc/subfolder/hidden_message.txt"),
                "You found the secret message!",
            ),
            (
                BundlePath::from("notes/ideas.txt"),
                "Build something amazing today!",
            ),
            (
                BundlePath::from("documents/summary.txt"),
                "Executive summary complete.",
            ),
            (BundlePath::from("misc/data.txt"), "Random data goes here."),
        ];

        for (key, expected_content) in files_to_test {
            let data = bundle
                .get(&key)
                .expect("Failed to read file")
                .unwrap_or_else(|| panic!("File not found: {key}"));
            assert_eq!(String::from_utf8(data).unwrap(), expected_content);
        }
    }

    #[test]
    fn test_prefix_queries() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Get all files under documents
        let docs = bundle
            .prefix(&BundlePath::from("documents"))
            .expect("Failed to get prefix");
        assert_eq!(docs.len(), 2);

        // Get all files under misc/subfolder
        let subfolder = bundle
            .prefix(&BundlePath::from("misc/subfolder"))
            .expect("Failed to get prefix");
        assert_eq!(subfolder.len(), 2);

        // Verify content of prefix results
        for (_key, data) in subfolder {
            let content = String::from_utf8(data).unwrap();
            assert!(
                content == "Deep inside the folder structure!"
                    || content == "You found the secret message!"
            );
        }
    }

    #[test]
    fn test_nonexistent_file() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        let result = bundle
            .get(&BundlePath::from("nonexistent.txt"))
            .expect("Failed to read file");
        assert!(result.is_none());
    }

    #[test]
    fn test_bundle_from_bytes() {
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle from bytes");

        // Test reading a file to ensure the bundle works correctly
        let data = bundle
            .get(&BundlePath::from("welcome.txt"))
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(data).unwrap(),
            "Hello from the root directory!"
        );
    }
}
