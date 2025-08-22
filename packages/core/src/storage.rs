use crate::bundle::{Bundle, RandomAccess};
use crate::util::CloneableFile;
use samod::storage::{Storage, StorageKey};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Storage adapter that uses Bundle for persistence instead of filesystem
#[derive(Debug)]
pub struct BundleStorage<R: RandomAccess> {
    bundle: Arc<Mutex<Bundle<R>>>,
}

impl<R: RandomAccess> BundleStorage<R> {
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
        let bundle = self.bundle.clone();
        async move {
            let mut bundle_guard = bundle.lock().await;

            // Convert StorageKey to Vec<String> for bundle
            let bundle_key: Vec<String> = key.into_iter().collect();

            match bundle_guard.get(&bundle_key) {
                Ok(data) => data,
                Err(_) => None,
            }
        }
    }

    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl std::future::Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        let bundle = self.bundle.clone();
        async move {
            let mut bundle_guard = bundle.lock().await;
            let mut result = HashMap::new();

            // Convert StorageKey to Vec<String> for bundle prefix
            let bundle_prefix: Vec<String> = prefix.into_iter().collect();

            match bundle_guard.get_prefix(&bundle_prefix) {
                Ok(entries) => {
                    for (key_components, data) in entries {
                        // Convert back to StorageKey
                        let storage_key: StorageKey =
                            key_components.into_iter().collect::<StorageKey>();
                        result.insert(storage_key, data);
                    }
                }
                Err(_) => {} // Return empty map on error
            }

            result
        }
    }

    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl std::future::Future<Output = ()> + Send {
        let bundle = self.bundle.clone();
        async move {
            let mut bundle_guard = bundle.lock().await;

            // Convert StorageKey to Vec<String> for bundle
            let bundle_key: Vec<String> = key.into_iter().collect();

            // Ignore errors for now - in a real implementation you might want to handle them
            let _ = bundle_guard.put(bundle_key, data);
        }
    }

    fn delete(&self, _key: StorageKey) -> impl std::future::Future<Output = ()> + Send {
        // Note: Bundle doesn't currently support delete operations
        // This would need to be implemented in the Bundle struct
        async move {
            // For now, this is a no-op
            // TODO: Implement delete functionality in Bundle
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samod::storage::Storage;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

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
        let key = StorageKey::from_iter(vec!["test".to_string(), "data1.bin".to_string()]);
        let result = storage.load(key).await;
        assert_eq!(result, Some(b"test data 1".to_vec()));

        // Test loading non-existent data
        let missing_key = StorageKey::from_iter(vec!["missing".to_string()]);
        let result = storage.load(missing_key).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_bundle_storage_load_range() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test loading with prefix
        let prefix = StorageKey::from_iter(vec!["test".to_string()]);
        let result = storage.load_range(prefix).await;

        assert_eq!(result.len(), 2);

        let key1 = StorageKey::from_iter(vec!["test".to_string(), "data1.bin".to_string()]);
        let key2 = StorageKey::from_iter(vec!["test".to_string(), "data2.bin".to_string()]);

        assert_eq!(result.get(&key1), Some(&b"test data 1".to_vec()));
        assert_eq!(result.get(&key2), Some(&b"test data 2".to_vec()));
    }

    #[tokio::test]
    async fn test_bundle_storage_put() {
        let zip_data = create_test_bundle().expect("Failed to create test bundle");
        let storage = BundleStorage::from_bytes(zip_data).expect("Failed to create storage");

        // Test putting new data
        let key = StorageKey::from_iter(vec!["new".to_string(), "file.bin".to_string()]);
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
        let prefix: StorageKey = empty_vec.into_iter().collect();
        let result = storage.load_range(prefix).await;

        // Should include manifest.json and the test files
        assert!(result.len() >= 3);

        let manifest_key = StorageKey::from_iter(vec!["manifest.json".to_string()]);
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
        let key = StorageKey::from_iter(vec!["test".to_string(), "data1.bin".to_string()]);
        let result = storage.load(key).await;
        assert_eq!(result, Some(b"test data 1".to_vec()));

        // Test putting new data (this will modify the file)
        let new_key = StorageKey::from_iter(vec!["file_test".to_string(), "new.bin".to_string()]);
        let new_data = b"data from file storage".to_vec();

        storage.put(new_key.clone(), new_data.clone()).await;

        // Verify it was stored
        let result = storage.load(new_key).await;
        assert_eq!(result, Some(new_data));

        // temp_file is automatically cleaned up when it goes out of scope
    }
}
