use crate::error::{RelayError, Result};
use samod::storage::StorageKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonk_core::{Bundle, BundlePath};

#[derive(Clone)]
pub struct BundleStorageAdapter {
    bundle: Arc<RwLock<Bundle<std::io::Cursor<Vec<u8>>>>>,
    memory_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl BundleStorageAdapter {
    pub async fn from_bundle(bundle_bytes: Vec<u8>) -> Result<Self> {
        let bundle = Bundle::from_bytes(bundle_bytes)
            .map_err(|e| RelayError::Bundle(format!("Failed to load bundle: {}", e)))?;

        let manifest = bundle.manifest();
        tracing::info!(
            "Loaded bundle with root ID: {}, {} entrypoints",
            manifest.root_id,
            manifest.entrypoints.len()
        );

        Ok(Self {
            bundle: Arc::new(RwLock::new(bundle)),
            memory_data: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn key_to_string(key: &StorageKey) -> String {
        key.into_iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn string_to_key(s: &str) -> Result<StorageKey> {
        if s.is_empty() || s == "/" {
            return StorageKey::from_parts(Vec::<&str>::new())
                .map_err(|e| RelayError::Storage(format!("Failed to create storage key: {}", e)));
        }

        let components: Vec<String> = s
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        StorageKey::from_parts(components)
            .map_err(|e| RelayError::Storage(format!("Failed to create storage key: {}", e)))
    }

    fn map_bundle_path_to_automerge_key(&self, bundle_path: &str) -> Option<StorageKey> {
        let re = regex::Regex::new(r"^storage/([^/]+)/([^/]+)/([^/]+)/bundle_export$").ok()?;
        let caps = re.captures(bundle_path)?;

        let prefix = caps.get(1)?.as_str();
        let rest_of_doc_id = caps.get(2)?.as_str();
        let type_name = caps.get(3)?.as_str();

        let full_doc_id = format!("{}{}", prefix, rest_of_doc_id);

        StorageKey::from_parts(vec![full_doc_id, type_name.to_string()]).ok()
    }

    async fn load_from_bundle(&self, path_str: &str) -> Result<Option<Vec<u8>>> {
        let mut bundle = self.bundle.write().await;
        let bundle_path = BundlePath::from(path_str);

        bundle
            .get(&bundle_path)
            .map_err(|e| RelayError::Bundle(format!("Failed to read from bundle: {}", e)))
    }

    pub async fn create_slim_bundle(&self) -> Result<Vec<u8>> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let (manifest_json, all_keys) = {
            let bundle = self.bundle.read().await;
            let manifest = bundle.manifest();
            let manifest_json = serde_json::to_string_pretty(manifest)?;
            let all_keys = bundle.list_keys();
            (manifest_json, all_keys)
        };

        let manifest: tonk_core::bundle::Manifest = serde_json::from_str(&manifest_json)?;
        let root_id_prefix = manifest.root_id.chars().take(2).collect::<String>();
        let storage_folder_prefix = format!("storage/{}", root_id_prefix);

        let mut zip_data = Vec::new();
        {
            let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

            zip_writer.start_file("manifest.json", SimpleFileOptions::default())?;
            zip_writer.write_all(manifest_json.as_bytes())?;

            for key in all_keys {
                let key_str = key.to_string();

                if !key_str.starts_with(&storage_folder_prefix) {
                    continue;
                }

                let memory_data = self.memory_data.read().await;
                let data = if let Some(updated_data) = memory_data.get(&key_str) {
                    updated_data.clone()
                } else {
                    drop(memory_data);
                    let mut bundle_write = self.bundle.write().await;
                    match bundle_write
                        .get(&key)
                        .map_err(|e| RelayError::Bundle(format!("Failed to read key: {}", e)))?
                    {
                        Some(data) => data,
                        None => continue,
                    }
                };

                zip_writer.start_file(&key_str, SimpleFileOptions::default())?;
                zip_writer.write_all(&data)?;
            }

            let memory_data = self.memory_data.read().await;
            for (key_str, data) in memory_data.iter() {
                if key_str.starts_with(&storage_folder_prefix) {
                    zip_writer.start_file(key_str, SimpleFileOptions::default())?;
                    zip_writer.write_all(data)?;
                }
            }

            zip_writer.finish()?;
        }

        Ok(zip_data)
    }
}

impl samod::storage::Storage for BundleStorageAdapter {
    fn load(&self, key: StorageKey) -> impl std::future::Future<Output = Option<Vec<u8>>> + Send {
        let key_str = Self::key_to_string(&key);
        let adapter = self.clone();

        async move {
            {
                let memory_data = adapter.memory_data.read().await;
                if let Some(data) = memory_data.get(&key_str) {
                    return Some(data.clone());
                }
            }

            adapter.load_from_bundle(&key_str).await.unwrap_or(None)
        }
    }

    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl std::future::Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        let adapter = self.clone();

        async move {
            let mut result = HashMap::new();
            let prefix_str = Self::key_to_string(&prefix);

            let mut bundle = adapter.bundle.write().await;
            let bundle_path = if prefix_str.is_empty() {
                BundlePath::from("")
            } else {
                BundlePath::from(prefix_str.as_str())
            };

            if let Ok(entries) = bundle.prefix(&bundle_path) {
                for (path, data) in entries {
                    let path_str = path.to_string();

                    if let Ok(key) = Self::string_to_key(&path_str) {
                        let mapped_key = adapter.map_bundle_path_to_automerge_key(&path_str);

                        let is_prefix_match = prefix.is_prefix_of(&key);
                        let is_mapped_match = mapped_key
                            .as_ref()
                            .map(|mk| prefix.is_prefix_of(mk))
                            .unwrap_or(false);

                        if is_prefix_match || is_mapped_match {
                            let memory_data = adapter.memory_data.read().await;
                            if !memory_data.contains_key(&path_str) {
                                drop(memory_data);
                                let result_key = if is_mapped_match {
                                    mapped_key.unwrap()
                                } else {
                                    key
                                };
                                result.insert(result_key, data);
                            }
                        }
                    }
                }
            }

            let memory_data = adapter.memory_data.read().await;
            for (key_str, data) in memory_data.iter() {
                if let Ok(key) = Self::string_to_key(key_str) {
                    if prefix.is_prefix_of(&key) {
                        result.insert(key, data.clone());
                    }
                }
            }

            result
        }
    }

    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl std::future::Future<Output = ()> + Send {
        let key_str = Self::key_to_string(&key);
        let adapter = self.clone();

        async move {
            let mut memory_data = adapter.memory_data.write().await;
            memory_data.insert(key_str, data);
        }
    }

    fn delete(&self, key: StorageKey) -> impl std::future::Future<Output = ()> + Send {
        let key_str = Self::key_to_string(&key);
        let adapter = self.clone();

        async move {
            let mut memory_data = adapter.memory_data.write().await;
            memory_data.remove(&key_str);
        }
    }
}
