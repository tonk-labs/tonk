//! Storage backend implementation for the service worker.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod inner {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;
    use wasm_bindgen::prelude::*;

    use dialog_common::Blake3Hash;
    use dialog_storage::{
        CompressedStorage, DialogStorageError, IndexedDbStorageBackend, StorageBackend,
        StorageCache, TransactionalMemoryBackend,
    };

    /// Type alias for compressed and cached storage backend.
    type CachedStorage =
        StorageCache<CompressedStorage<3, IndexedDbStorageBackend<[u8; 32], Vec<u8>>>>;

    /// Storage backend for the service worker using IndexedDB with compression and caching.
    ///
    /// This wraps an IndexedDB backend that provides both StorageBackend and
    /// TransactionalMemoryBackend implementations for use with Space.
    #[derive(Clone)]
    pub struct ServiceWorkerStorageBackend {
        /// Compressed and cached storage for blobs (StorageBackend).
        /// Uses [u8; 32] keys internally for content-addressed storage.
        storage: Arc<Mutex<CachedStorage>>,
        /// Raw IndexedDB backend for transactional memory (branches, remotes).
        memory: IndexedDbStorageBackend<Vec<u8>, Vec<u8>>,
    }

    impl ServiceWorkerStorageBackend {
        /// Creates a new storage backend instance.
        ///
        /// Initializes an IndexedDB backend wrapped with compression (level 3) and
        /// a 64K-large in-memory cache for blocks.
        pub async fn new(name: &str) -> Self {
            let backend: IndexedDbStorageBackend<[u8; 32], Vec<u8>> =
                IndexedDbStorageBackend::new(name).await.unwrap_throw();
            let compressed = CompressedStorage::<3, _>::new(backend);
            #[allow(clippy::arc_with_non_send_sync)]
            let storage = Arc::new(Mutex::new(
                StorageCache::new(compressed, 64_000).unwrap_throw(),
            ));

            let memory: IndexedDbStorageBackend<Vec<u8>, Vec<u8>> =
                IndexedDbStorageBackend::new(name).await.unwrap_throw();

            Self { storage, memory }
        }
    }

    // SAFETY: At the time of authorship, web browsers run Wasm in a single thread
    // only. If and when this changes, the interior storage is wrapped in a Send +
    // Sync locking mechanism, and `ConditionalSend + ConditionalSync` bounds
    // _should_ flip to requiring the appropriate bounds on the inner
    // `BackendStorage` implementation
    unsafe impl Send for ServiceWorkerStorageBackend {}
    unsafe impl Sync for ServiceWorkerStorageBackend {}

    /// Convert Vec<u8> to [u8; 32], assuming the vec contains exactly 32 bytes.
    fn vec_to_hash(key: &[u8]) -> Result<[u8; 32], DialogStorageError> {
        key.try_into().map_err(|_| {
            DialogStorageError::StorageBackend(format!("Key must be 32 bytes, got {}", key.len()))
        })
    }

    #[async_trait(?Send)]
    impl StorageBackend for ServiceWorkerStorageBackend {
        type Key = Vec<u8>;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            let hash = vec_to_hash(&key)?;
            self.storage.lock().await.set(hash, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            let hash = vec_to_hash(key)?;
            self.storage.lock().await.get(&hash).await
        }
    }

    #[async_trait(?Send)]
    impl TransactionalMemoryBackend for ServiceWorkerStorageBackend {
        type Address = Vec<u8>;
        type Value = Vec<u8>;
        type Error = DialogStorageError;
        type Edition = Blake3Hash;

        async fn resolve(
            &self,
            address: &Self::Address,
        ) -> Result<Option<(Self::Value, Self::Edition)>, Self::Error> {
            self.memory.resolve(address).await
        }

        async fn replace(
            &self,
            address: &Self::Address,
            edition: Option<&Self::Edition>,
            content: Option<Self::Value>,
        ) -> Result<Option<Self::Edition>, Self::Error> {
            self.memory.replace(address, edition, content).await
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use dialog_common::Blake3Hash;
    use dialog_storage::{
        DialogStorageError, MemoryStorageBackend, StorageBackend, TransactionalMemoryBackend,
    };

    /// Storage backend for non-Wasm targets using in-memory storage.
    ///
    /// This is only a placeholder implementation for testing purposes. The worker
    /// has no use case for being used in non-wasm contexts at this time.
    #[derive(Clone)]
    pub struct ServiceWorkerStorageBackend(MemoryStorageBackend<Vec<u8>, Vec<u8>>);

    impl ServiceWorkerStorageBackend {
        /// Creates a new in-memory storage backend instance.
        pub async fn new(_name: &str) -> Self {
            Self(MemoryStorageBackend::default())
        }
    }

    #[async_trait::async_trait]
    impl StorageBackend for ServiceWorkerStorageBackend {
        type Key = Vec<u8>;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            self.0.set(key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            self.0.get(key).await
        }
    }

    #[async_trait::async_trait]
    impl TransactionalMemoryBackend for ServiceWorkerStorageBackend {
        type Address = Vec<u8>;
        type Value = Vec<u8>;
        type Error = DialogStorageError;
        type Edition = Blake3Hash;

        async fn resolve(
            &self,
            address: &Self::Address,
        ) -> Result<Option<(Self::Value, Self::Edition)>, Self::Error> {
            self.0.resolve(address).await
        }

        async fn replace(
            &self,
            address: &Self::Address,
            edition: Option<&Self::Edition>,
            content: Option<Self::Value>,
        ) -> Result<Option<Self::Edition>, Self::Error> {
            self.0.replace(address, edition, content).await
        }
    }
}

pub use inner::*;
