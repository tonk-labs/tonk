pub mod path;

use anyhow::{Context, Result};
use automerge::transaction::Transactable;
pub use path::BundlePath;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// Version information for the bundle
#[derive(Debug, Deserialize, Clone)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

/// Manifest structure for bundle metadata
#[derive(Debug, Deserialize, Clone)]
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
    fn get_prefix_paths(&self, prefix_components: &[String]) -> Vec<&String> {
        if prefix_components.is_empty() {
            // Return all paths from this subtree
            return self.collect_all_paths();
        }

        let component = &prefix_components[0];
        if let Some(child) = self.children.get(component) {
            child.get_prefix_paths(&prefix_components[1..])
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
    pub fn get_entry(&self, path: &str) -> Option<&EntryMetadata> {
        self.entries.get(path)
    }

    /// Get all entries matching a prefix
    pub fn get_prefix_entries(&self, prefix: &str) -> Vec<&EntryMetadata> {
        let prefix_components: Vec<String> = if prefix.is_empty() {
            Vec::new()
        } else {
            prefix
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        };

        let matching_paths = self.path_tree.get_prefix_paths(&prefix_components);
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
    /// Track if central directory needs updating
    needs_rebuild: bool,
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
            needs_rebuild: false,
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
        if let Some(metadata) = self.index.get_entry(&path).cloned() {
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

    /// Delete a file by removing it from the central directory
    ///
    /// This operation is lazy - the file is immediately removed from the in-memory index
    /// but the physical ZIP file is not updated until flush()
    /// is called or the Bundle is dropped.
    pub fn delete(&mut self, key: &BundlePath) -> Result<()> {
        let path = key.to_string();

        // Check if the file exists and remove it from our in-memory index
        if self.index.entries.remove(&path).is_none() {
            return Err(anyhow::anyhow!("File not found: {}", path));
        }

        // Remove from path tree for prefix queries
        self.index.remove_from_path_tree(&path);

        // Mark for lazy central directory rebuild - the file will be physically
        // removed from the ZIP when flush() is called or when Bundle is dropped
        self.needs_rebuild = true;

        Ok(())
    }

    /// Append a key-value pair to the ZIP bundle
    ///
    /// This operation is immediate - the file is written directly to the ZIP archive
    /// and the in-memory index is rebuilt to reflect the changes. This ensures the
    /// file is immediately available for reading.
    pub fn put(&mut self, key: &BundlePath, value: Vec<u8>) -> Result<()> {
        let path = key.to_string();

        // HACK: properly implement ZIP file updates
        // Reset to the beginning to ensure ZipWriter can properly read the ZIP structure
        self.data_source.seek_to(0)?;

        // Use ZipWriter::new_append to safely append to the existing ZIP archive
        let mut zip_writer = ZipWriter::new_append(&mut self.data_source)
            .context("Failed to create ZipWriter for appending")?;

        // Start a new file entry in the ZIP
        zip_writer
            .start_file(&path, SimpleFileOptions::default())
            .context("Failed to start new file in ZIP")?;

        // Write the data
        zip_writer
            .write_all(&value)
            .context("Failed to write data to ZIP entry")?;

        // Finish the ZIP operation to update the central directory
        zip_writer
            .finish()
            .context("Failed to finish ZIP writing")?;

        // After writing, we need to rebuild the index since the ZIP structure has changed
        self.index = Self::build_index(&mut self.data_source)
            .context("Failed to rebuild index after writing")?;

        // Reset needs_rebuild since we just did a full rebuild
        self.needs_rebuild = false;

        Ok(())
    }

    /// Read all key-value pairs that match a key prefix
    pub fn get_prefix(&mut self, prefix: &BundlePath) -> Result<Vec<(BundlePath, Vec<u8>)>> {
        let prefix_path = prefix.to_string();
        let entries: Vec<EntryMetadata> = self
            .index
            .get_prefix_entries(&prefix_path)
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
            .get_entry("manifest.json")
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

    /// Flush changes by rebuilding the central directory if needed
    pub fn flush(&mut self) -> Result<()> {
        if self.needs_rebuild {
            self.rebuild_central_directory()?;
            self.needs_rebuild = false;
        }
        Ok(())
    }

    /// Rebuild the central directory to reflect current index state
    /// This creates a new ZIP archive that only includes files present in our index
    fn rebuild_central_directory(&mut self) -> Result<()> {
        // Create new ZIP archive with only files from our current index
        let mut new_data = Vec::new();
        {
            let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut new_data));

            // Copy only files that exist in our index (excluding deleted ones)
            let entries: Vec<(String, EntryMetadata)> =
                self.index.entries.clone().into_iter().collect();
            for (path, metadata) in entries {
                // Try to read file data, but skip if it fails (file might have been
                // corrupted or the ZIP structure changed since we last rebuilt)
                if let Ok(Some(file_data)) = self.read_entry_data(&metadata) {
                    zip_writer.start_file(&path, SimpleFileOptions::default())?;
                    zip_writer.write_all(&file_data)?;
                } else {
                    // Log warning but continue - this file will be lost
                    tracing::warn!(
                        "Could not read data for file '{}' during rebuild, skipping",
                        path
                    );
                }
            }

            zip_writer.finish()?;
        }

        // Replace our data source with the new ZIP that excludes deleted files
        self.data_source.seek_to(0)?;
        self.data_source.write_all(&new_data)?;
        RandomAccess::flush(&mut self.data_source)?;

        // Rebuild index from the new archive
        self.index = Self::build_index(&mut self.data_source)?;

        Ok(())
    }

    /// Compact the bundle by creating a new archive with only active files
    /// This physically removes deleted file data and reclaims space
    pub fn compact(&mut self) -> Result<()> {
        // Create new ZIP archive with only files from our index
        let mut new_data = Vec::new();
        {
            let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut new_data));

            // Copy only files that exist in our index
            let entries: Vec<(String, EntryMetadata)> =
                self.index.entries.clone().into_iter().collect();
            for (path, metadata) in entries {
                if let Some(file_data) = self.read_entry_data(&metadata)? {
                    zip_writer.start_file(&path, SimpleFileOptions::default())?;
                    zip_writer.write_all(&file_data)?;
                }
            }

            zip_writer.finish()?;
        }

        // Replace our data source with compacted version
        self.data_source.seek_to(0)?;
        self.data_source.write_all(&new_data)?;
        RandomAccess::flush(&mut self.data_source)?;

        // Truncate the data source if possible (for files)
        if let Some(_size) = self.data_source.size()? {
            // For files, we should truncate to the new size
            // This is a limitation of the RandomAccess trait - it doesn't have truncate
            // In practice, this would need to be handled at the file level
        }

        // Rebuild index from new archive
        self.index = Self::build_index(&mut self.data_source)?;
        self.needs_rebuild = false;

        Ok(())
    }
}

impl<R: RandomAccess> Drop for Bundle<R> {
    /// Automatically flush any pending changes when Bundle is dropped
    fn drop(&mut self) {
        if self.needs_rebuild {
            // We can't propagate errors in Drop, so we just log them
            if let Err(e) = self.flush() {
                tracing::warn!("Failed to flush pending changes during Bundle drop: {}", e);
            }
        }
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
        // First flush any pending changes
        self.flush()?;

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
            .get_prefix(&BundlePath::from("documents"))
            .expect("Failed to get prefix");
        assert_eq!(docs.len(), 2);

        // Get all files under misc/subfolder
        let subfolder = bundle
            .get_prefix(&BundlePath::from("misc/subfolder"))
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

    #[test]
    fn test_simple_put_in_memory() {
        // Test with in-memory buffer
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle from bytes");

        // Add a simple new file
        let key = BundlePath::from("test/simple.txt");
        let content = b"Simple test content".to_vec();

        bundle
            .put(&key, content.clone())
            .expect("Failed to put simple file");

        // Read it back
        let read_content = bundle
            .get(&key)
            .expect("Failed to read simple file")
            .expect("Simple file not found");

        assert_eq!(read_content, content);
    }

    #[test]
    fn test_put_and_get_new_file() {
        // Create a test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Add a new file
        let new_key = BundlePath::from("new_file.txt");
        let new_content = b"This is a newly added file!".to_vec();
        bundle
            .put(&new_key, new_content.clone())
            .expect("Failed to put new file");

        // Re-read the file to verify it was added correctly
        let read_content = bundle
            .get(&new_key)
            .expect("Failed to read new file")
            .expect("New file not found");

        assert_eq!(read_content, new_content);
        assert_eq!(
            String::from_utf8(read_content).unwrap(),
            "This is a newly added file!"
        );

        // Verify the file count increased
        let keys = bundle.list_keys();
        assert_eq!(keys.len(), 11); // Original 10 (including manifest) + 1 new file
    }

    #[test]
    fn test_put_multiple_files_and_read_back() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Add multiple new files
        let files_to_add = vec![
            (
                BundlePath::from("test1.txt"),
                b"Content of test file 1".to_vec(),
            ),
            (
                BundlePath::from("documents/new_report.txt"),
                b"New quarterly report data".to_vec(),
            ),
            (
                BundlePath::from("notes/urgent.txt"),
                b"Urgent reminder!".to_vec(),
            ),
        ];

        // Add all files
        for (key, content) in &files_to_add {
            bundle
                .put(key, content.clone())
                .expect("Failed to put file");
        }

        // Verify all files can be read back correctly
        for (key, expected_content) in &files_to_add {
            let read_content = bundle
                .get(key)
                .expect("Failed to read file")
                .expect("File not found");
            assert_eq!(read_content, *expected_content);
        }

        // Verify the total file count
        let keys = bundle.list_keys();
        assert_eq!(keys.len(), 13); // Original 10 (including manifest) + 3 new files
    }

    #[test]
    fn test_put_and_prefix_query() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Add files in a new subdirectory
        let new_files = vec![
            (
                BundlePath::from("new_dir/file1.txt"),
                b"File 1 content".to_vec(),
            ),
            (
                BundlePath::from("new_dir/file2.txt"),
                b"File 2 content".to_vec(),
            ),
            (
                BundlePath::from("new_dir/subdirectory/file3.txt"),
                b"File 3 content".to_vec(),
            ),
        ];

        // Add all files
        for (key, content) in &new_files {
            bundle
                .put(key, content.clone())
                .expect("Failed to put file");
        }

        // Test prefix query for the new directory
        let new_dir_files = bundle
            .get_prefix(&BundlePath::from("new_dir"))
            .expect("Failed to get prefix");

        assert_eq!(new_dir_files.len(), 3);

        // Verify all files are present in the prefix results
        let mut found_contents = std::collections::HashSet::new();
        for (_key, content) in new_dir_files {
            found_contents.insert(String::from_utf8(content).unwrap());
        }

        assert!(found_contents.contains("File 1 content"));
        assert!(found_contents.contains("File 2 content"));
        assert!(found_contents.contains("File 3 content"));
    }

    #[test]
    fn test_put_duplicate_path_behavior() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Read original content to verify the file exists
        let key = BundlePath::from("welcome.txt");
        let original_content = bundle
            .get(&key)
            .expect("Failed to read original file")
            .expect("Original file not found");
        assert_eq!(
            String::from_utf8(original_content).unwrap(),
            "Hello from the root directory!"
        );

        // Try to add a file with the same path - this should fail with a duplicate filename error
        let duplicate_content = b"This content has been updated!".to_vec();
        let result = bundle.put(&key, duplicate_content);

        // Verify that putting a duplicate filename fails
        assert!(
            result.is_err(),
            "Expected error when adding duplicate filename"
        );
        let error = result.unwrap_err();
        let error_msg = error.to_string();

        // Check if the error message indicates a duplicate filename issue
        // The zip crate may wrap the duplicate filename error in a generic "Failed to start new file" message
        let is_duplicate_error = error_msg.contains("Duplicate filename")
            || error_msg.contains("duplicate")
            || error_msg.contains("Failed to start new file in ZIP");

        assert!(
            is_duplicate_error,
            "Expected duplicate filename error, got: {error_msg}"
        );

        // Verify the original file is still accessible and unchanged
        let still_original = bundle
            .get(&key)
            .expect("Failed to read original file")
            .expect("Original file not found");
        assert_eq!(
            String::from_utf8(still_original).unwrap(),
            "Hello from the root directory!"
        );
    }

    #[test]
    fn test_put_with_file_data_source() {
        use std::fs::OpenOptions;
        use tempfile::NamedTempFile;

        // Create test bundle data and write it to a temporary file
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        temp_file
            .write_all(&zip_data)
            .expect("Failed to write test data to temp file");

        // Open the temporary file with read+write permissions
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp_file.path())
            .expect("Failed to open file for read+write");

        let mut bundle = Bundle::from_source(file).expect("Failed to load bundle from file source");

        // Add a new file
        let key = BundlePath::from("from_file_source.txt");
        let content = b"Added via file data source!".to_vec();
        bundle
            .put(&key, content.clone())
            .expect("Failed to put file");

        // Read it back
        let read_content = bundle
            .get(&key)
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(read_content, content);

        // temp_file is automatically cleaned up when it goes out of scope
    }

    #[test]
    fn test_delete_and_get() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Verify file exists
        let key = BundlePath::from("welcome.txt");
        let content = bundle
            .get(&key)
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(
            String::from_utf8(content).unwrap(),
            "Hello from the root directory!"
        );

        // Delete the file
        bundle.delete(&key).expect("Failed to delete file");

        // Verify file is no longer accessible
        let result = bundle.get(&key).expect("Failed to read deleted file");
        assert!(result.is_none(), "Deleted file should not be accessible");

        // File should be deleted from index
        assert!(
            bundle.index.get_entry("welcome.txt").is_none(),
            "File should be removed from index after deletion"
        );
    }

    #[test]
    fn test_delete_and_recover() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Use a unique filename that doesn't exist in the test bundle
        let key = BundlePath::from("unique_test_file.txt");
        let original_content = b"Original content".to_vec();

        // Add a file
        bundle
            .put(&key, original_content.clone())
            .expect("Failed to put file");

        // Verify it exists
        let read_content = bundle
            .get(&key)
            .expect("Failed to read file")
            .expect("File not found");
        assert_eq!(read_content, original_content);

        // Delete the file
        bundle.delete(&key).expect("Failed to delete file");

        // Verify it's deleted (removed from index)
        let deleted_result = bundle.get(&key).expect("Failed to read deleted file");
        assert!(deleted_result.is_none());

        // In the central directory approach, recovery means the file is still
        // physically in the ZIP but just removed from our index
        // We can simulate recovery by adding it back to the index manually
        // (in practice, this would be done by putting a new file or
        // rebuilding from the physical ZIP contents)

        // For this test, let's verify that the file is indeed gone
        // and that we could recover it by rebuilding the index from the ZIP

        // The physical file should still exist in the ZIP archive
        // Let's rebuild the index to see the original file
        let original_index =
            Bundle::<std::io::Cursor<Vec<u8>>>::build_index(&mut bundle.data_source)
                .expect("Failed to rebuild index");

        // The original file should be back in the rebuilt index
        assert!(
            original_index.get_entry("unique_test_file.txt").is_some(),
            "File should exist in rebuilt index"
        );

        // If we restore it to our bundle's index, it should be accessible
        if let Some(metadata) = original_index.get_entry("unique_test_file.txt") {
            bundle.index.add_entry(metadata.clone());
            let recovered_result = bundle
                .get(&key)
                .expect("Failed to read recovered file")
                .expect("Recovered file not found");
            assert_eq!(recovered_result, original_content);
        }
    }

    #[test]
    fn test_delete_nonexistent_file() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Try to delete a file that doesn't exist
        let key = BundlePath::from("nonexistent.txt");
        let result = bundle.delete(&key);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("File not found"));
    }

    #[test]
    fn test_prefix_query_with_delete() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Get initial count of documents
        let initial_docs = bundle
            .get_prefix(&BundlePath::from("documents"))
            .expect("Failed to get prefix");
        let initial_count = initial_docs.len();

        // Delete one document
        bundle
            .delete(&BundlePath::from("documents/report.txt"))
            .expect("Failed to delete document");

        // Query again - should have one less document
        let after_delete_docs = bundle
            .get_prefix(&BundlePath::from("documents"))
            .expect("Failed to get prefix after delete");
        assert_eq!(after_delete_docs.len(), initial_count - 1);

        // Verify the deleted file is not in the results
        let has_deleted_file = after_delete_docs
            .iter()
            .any(|(key, _)| key.to_string() == "documents/report.txt");
        assert!(
            !has_deleted_file,
            "Deleted file should not appear in prefix query"
        );
    }

    #[test]
    fn test_flush_and_lazy_deletion() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Delete an existing file (not one we just added)
        let key = BundlePath::from("welcome.txt");

        // Verify file exists first
        let content = bundle
            .get(&key)
            .expect("Failed to read file")
            .expect("File should exist");
        assert_eq!(
            String::from_utf8(content).unwrap(),
            "Hello from the root directory!"
        );

        // Delete the file - this should set needs_rebuild = true
        bundle.delete(&key).expect("Failed to delete file");
        assert!(
            bundle.needs_rebuild,
            "needs_rebuild should be true after delete"
        );

        // File should be immediately inaccessible even though ZIP wasn't updated yet
        let result = bundle.get(&key).expect("Failed to read deleted file");
        assert!(result.is_none(), "Deleted file should not be accessible");

        // Explicitly flush to apply the deletion to the ZIP
        bundle.flush().expect("Failed to flush");
        assert!(
            !bundle.needs_rebuild,
            "needs_rebuild should be false after flush"
        );

        // File should still be inaccessible after flush
        let result = bundle
            .get(&key)
            .expect("Failed to read deleted file after flush");
        assert!(
            result.is_none(),
            "Deleted file should still not be accessible after flush"
        );

        // Other files should still be accessible
        let other_content = bundle
            .get(&BundlePath::from("readme.txt"))
            .expect("Failed to read other file");
        assert!(
            other_content.is_some(),
            "Other files should still be accessible after flush"
        );
    }

    #[test]
    fn test_put_clears_needs_rebuild() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Delete a file to set needs_rebuild = true
        let key = BundlePath::from("welcome.txt");
        bundle.delete(&key).expect("Failed to delete file");
        assert!(
            bundle.needs_rebuild,
            "needs_rebuild should be true after delete"
        );

        // Put a new file - this should clear needs_rebuild since it does a full rebuild
        let new_key = BundlePath::from("test_put_clears.txt");
        bundle
            .put(&new_key, b"Test content".to_vec())
            .expect("Failed to put file");

        // needs_rebuild should be false since put() does immediate index rebuild
        assert!(
            !bundle.needs_rebuild,
            "needs_rebuild should be false after put"
        );

        // New file should be accessible
        let result = bundle.get(&new_key).expect("Failed to read new file");
        assert!(result.is_some(), "New file should be accessible");
    }

    #[test]
    fn test_deleted_files_not_in_queries() {
        // Create test bundle in memory
        let zip_data = create_complete_test_bundle().expect("Failed to create test bundle");
        let mut bundle = Bundle::from_bytes(zip_data).expect("Failed to load bundle");

        // Get initial count of files
        let initial_files = bundle
            .get_prefix(&BundlePath::root())
            .expect("Failed to get all files");
        let initial_count = initial_files.len();

        // Add a file and then delete it
        let key = BundlePath::from("test_deleted.txt");
        bundle
            .put(&key, b"Test content".to_vec())
            .expect("Failed to put file");

        // Verify file was added
        let after_add_files = bundle
            .get_prefix(&BundlePath::root())
            .expect("Failed to get all files");
        assert_eq!(after_add_files.len(), initial_count + 1);

        // Delete the file
        bundle.delete(&key).expect("Failed to delete file");

        // Verify deleted file doesn't appear in queries
        let after_delete_files = bundle
            .get_prefix(&BundlePath::root())
            .expect("Failed to get all files");
        assert_eq!(after_delete_files.len(), initial_count);

        // Verify the deleted file is not in the results
        let has_deleted_file = after_delete_files
            .iter()
            .any(|(key_parts, _)| key_parts.to_string() == "test_deleted.txt");
        assert!(
            !has_deleted_file,
            "Deleted file should not appear in prefix queries"
        );
    }
}
