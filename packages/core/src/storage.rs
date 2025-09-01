use crate::bundle::{Bundle, BundlePath, RandomAccess};
use crate::util::CloneableFile;
use samod::storage::{Storage, StorageKey};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Storage adapter that uses Bundle for persistence instead of filesystem.
///
/// BundleStorage implements the samod Storage trait, allowing Bundle instances
/// to be used as the persistence layer for CRDT synchronization. This enables
/// storing document data in ZIP archives instead of traditional filesystems.
///
/// # Type Parameters
/// * `R` - The RandomAccess type used by the underlying Bundle
///
/// # Examples
///
/// ```no_run
/// # use tonk_core::storage::BundleStorage;
/// # use std::io::Cursor;
/// let data = vec![/* ZIP data */];
/// let storage = BundleStorage::from_bytes(data).unwrap();
/// ```
#[derive(Debug)]
pub struct BundleStorage<R: RandomAccess> {
    bundle: Arc<Mutex<Bundle<R>>>,
}

impl<R: RandomAccess> BundleStorage<R> {
    /// Create a new BundleStorage from an existing Bundle.
    ///
    /// # Arguments
    /// * `bundle` - The Bundle instance to wrap
    ///
    /// # Returns
    /// A new BundleStorage instance that can be used with samod
    pub fn new(bundle: Bundle<R>) -> Self {
        Self {
            bundle: Arc::new(Mutex::new(bundle)),
        }
    }
}

impl<R: RandomAccess> Clone for BundleStorage<R> {
    fn clone(&self) -> Self {
        Self {
            bundle: Arc::clone(&self.bundle),
        }
    }
}

// Convenience constructors for common bundle types
impl BundleStorage<std::io::Cursor<Vec<u8>>> {
    /// Create a new BundleStorage from a byte array (in-memory bundle)
    pub fn from_bytes(data: Vec<u8>) -> anyhow::Result<Self> {
        let bundle = Bundle::from_bytes(data)?;
        Ok(Self::new(bundle))
    }
}

impl BundleStorage<CloneableFile> {
    /// Create a new BundleStorage from a file path (for large files with seeking)
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let cloneable_file =
            CloneableFile::new(path).map_err(|e| anyhow::anyhow!("Failed to open file: {}", e))?;
        let bundle = Bundle::from_stream(cloneable_file)?;
        Ok(Self::new(bundle))
    }
}

impl<R: RandomAccess + 'static> Storage for BundleStorage<R> {
    fn load(&self, key: StorageKey) -> impl std::future::Future<Output = Option<Vec<u8>>> + Send {
        let bundle = Arc::clone(&self.bundle);
        async move {
            let mut bundle_guard = bundle.lock().await;

            // Convert StorageKey to BundlePath for bundle
            let bundle_key = BundlePath::from(key.into_iter().collect::<Vec<String>>());

            bundle_guard.get(&bundle_key).unwrap_or_default()
        }
    }

    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl std::future::Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        let bundle = Arc::clone(&self.bundle);
        async move {
            let mut bundle_guard = bundle.lock().await;
            let mut result = HashMap::new();

            // Convert StorageKey to BundlePath for bundle prefix
            let bundle_prefix = BundlePath::from(prefix.into_iter().collect::<Vec<String>>());

            if let Ok(entries) = bundle_guard.get_prefix(&bundle_prefix) {
                for (key_path, data) in entries {
                    // Convert back to StorageKey
                    let storage_key: StorageKey = StorageKey::from_parts(key_path.components())
                        .expect("Failed to create StorageKey from BundlePath");
                    result.insert(storage_key, data);
                }
            }

            result
        }
    }

    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl std::future::Future<Output = ()> + Send {
        let bundle = Arc::clone(&self.bundle);
        async move {
            let mut bundle_guard = bundle.lock().await;

            // Convert StorageKey to BundlePath for bundle
            let bundle_key = BundlePath::from(key.into_iter().collect::<Vec<String>>());

            // Ignore errors for now - in a real implementation you might want to handle them
            let _ = bundle_guard.put(&bundle_key, data);
        }
    }

    async fn delete(&self, key: StorageKey) {
        let bundle = Arc::clone(&self.bundle);
        let bundle_key = BundlePath::from(key.into_iter().collect::<Vec<String>>());

        let mut bundle_guard = bundle.lock().await;

        // Call the delete method on the bundle
        if let Err(e) = bundle_guard.delete(&bundle_key) {
            // Log the error but don't propagate it since the trait method doesn't return Result
            tracing::warn!("Failed to delete key: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samod::storage::Storage;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    /// Create a test bundle with manifest and some test data
    fn create_test_bundle() -> anyhow::Result<Vec<u8>> {
        let mut zip_data = Vec::new();
        let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

        // Create manifest.json
        let manifest_content = r#"{
            "manifestVersion": 1,
            "version": { "major": 1, "minor": 0 },
            "root": "main",
            "entrypoints": ["bin/app"],
            "networkUris": []
        }"#;

        zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
        zip_writer.write_all(manifest_content.as_bytes())?;

        // Add some test data
        zip_writer.start_file("test/data1.bin", SimpleFileOptions::default())?;
        zip_writer.write_all(b"test data 1")?;

        zip_writer.start_file("test/data2.bin", SimpleFileOptions::default())?;
        zip_writer.write_all(b"test data 2")?;

        zip_writer.finish()?;
        Ok(zip_data)
    }

    #[tokio::test]
    async fn test_bundle_storage_load() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test loading existing data
        let key =
            StorageKey::from_parts(vec!["test".to_string(), "data1.bin".to_string()]).unwrap();
        let result = storage.load(key).await;
        assert_eq!(result, Some(b"test data 1".to_vec()));

        // Test loading non-existent data
        let missing_key = StorageKey::from_parts(vec!["missing".to_string()]).unwrap();
        let result = storage.load(missing_key).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_bundle_storage_load_range() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test loading with prefix
        let prefix = StorageKey::from_parts(vec!["test".to_string()]).unwrap();
        let result = storage.load_range(prefix).await;

        assert_eq!(result.len(), 2);

        let key1 =
            StorageKey::from_parts(vec!["test".to_string(), "data1.bin".to_string()]).unwrap();
        let key2 =
            StorageKey::from_parts(vec!["test".to_string(), "data2.bin".to_string()]).unwrap();

        assert_eq!(result.get(&key1), Some(&b"test data 1".to_vec()));
        assert_eq!(result.get(&key2), Some(&b"test data 2".to_vec()));
    }

    #[tokio::test]
    async fn test_bundle_storage_put() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test putting new data
        let key = StorageKey::from_parts(vec!["new".to_string(), "file.bin".to_string()]).unwrap();
        let data = b"new test data".to_vec();

        storage.put(key.clone(), data.clone()).await;

        // Verify it was stored
        let result = storage.load(key).await;
        assert_eq!(result, Some(data));
    }

    #[tokio::test]
    async fn test_bundle_storage_empty_prefix() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test loading with empty prefix (should get all files)
        let empty_vec: Vec<String> = vec![];
        let prefix = StorageKey::from_parts(empty_vec).unwrap();
        let result = storage.load_range(prefix).await;

        // Should include manifest.json and the test files
        assert!(result.len() >= 3);

        let manifest_key = StorageKey::from_parts(vec!["manifest.json".to_string()]).unwrap();
        assert!(result.contains_key(&manifest_key));
    }

    #[tokio::test]
    async fn test_bundle_storage_from_file() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        // Create test bundle data and write it to a temporary file
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        temp_file
            .write_all(&zip_data)
            .expect("Failed to write test data to temp file");
        std::io::Write::flush(&mut temp_file).expect("Failed to flush temp file");

        // Create storage from file
        let storage = BundleStorage::<CloneableFile>::from_file(temp_file.path())
            .expect("Failed to create storage from file");

        // Test loading existing data
        let key =
            StorageKey::from_parts(vec!["test".to_string(), "data1.bin".to_string()]).unwrap();
        let result = storage.load(key).await;
        assert_eq!(result, Some(b"test data 1".to_vec()));

        // Test putting new data (this will modify the file)
        let new_key =
            StorageKey::from_parts(vec!["file_test".to_string(), "new.bin".to_string()]).unwrap();
        let new_data = b"data from file storage".to_vec();

        storage.put(new_key.clone(), new_data.clone()).await;

        // Verify it was stored
        let result = storage.load(new_key).await;
        assert_eq!(result, Some(new_data));

        // temp_file is automatically cleaned up when it goes out of scope
    }

    #[tokio::test]
    async fn test_bundle_storage_delete() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Add a file
        let key = StorageKey::from_parts(vec!["delete_test".to_string(), "file.txt".to_string()])
            .unwrap();
        let data = b"This file will be deleted".to_vec();

        storage.put(key.clone(), data.clone()).await;

        // Verify it exists
        let result = storage.load(key.clone()).await;
        assert_eq!(result, Some(data));

        // Delete the file
        storage.delete(key.clone()).await;

        // Verify it's gone
        let after_delete = storage.load(key).await;
        assert_eq!(after_delete, None);
    }

    #[tokio::test]
    async fn test_bundle_storage_delete_and_recover() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Add a file
        let key = StorageKey::from_parts(vec!["recover_test".to_string(), "file.txt".to_string()])
            .unwrap();
        let original_data = b"Original content".to_vec();

        storage.put(key.clone(), original_data.clone()).await;

        // Delete it
        storage.delete(key.clone()).await;

        // Verify it's gone
        let after_delete = storage.load(key.clone()).await;
        assert_eq!(after_delete, None);
    }
}
