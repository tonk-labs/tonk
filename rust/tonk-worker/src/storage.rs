//! Storage backend implementation for the service worker.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod inner {
    use async_trait::async_trait;
    use std::sync::Arc;
    use wasm_bindgen::prelude::*;

    use dialog_storage::{
        Blake3Hash, CompressedStorage, DialogStorageError, IndexedDbStorageBackend, StorageBackend,
        StorageCache, web::ObjectSafeStorageBackend,
    };
    use tokio::sync::Mutex;

    /// Storage backend for the service worker using IndexedDB with compression and caching.
    ///
    /// This wraps an IndexedDB backend with compression and caching layers to optimize
    /// storage performance in the browser environment.
    #[derive(Clone)]
    pub struct ServiceWorkerStorageBackend(Arc<Mutex<dyn ObjectSafeStorageBackend>>);

    impl ServiceWorkerStorageBackend {
        /// Creates a new storage backend instance.
        ///
        /// Initializes an IndexedDB backend wrapped with compression (level 3) and
        /// a 64K-large in-memory cache for blocks.
        pub async fn new() -> Self {
            let backend = IndexedDbStorageBackend::new("tonk-artifacts")
                .await
                .unwrap_throw();
            let backend = CompressedStorage::<3, _>::new(backend);
            #[allow(clippy::arc_with_non_send_sync)]
            let backend: Arc<Mutex<dyn ObjectSafeStorageBackend>> = Arc::new(Mutex::new(
                StorageCache::new(backend, 64_000).unwrap_throw(),
            ));

            Self(backend)
        }
    }

    // SAFETY: At the time of authorship, web browsers run Wasm in a single thread
    // only. If and when this changes, the interior storage is wrapped in a Send +
    // Sync locking mechanism, and `ConditionalSend + ConditionalSync` bounds
    // _should_ flip to requiring the appropriate bounds on the inner
    // `BackendStorage` implementation
    unsafe impl Send for ServiceWorkerStorageBackend {}
    unsafe impl Sync for ServiceWorkerStorageBackend {}

    #[async_trait(?Send)]
    impl StorageBackend for ServiceWorkerStorageBackend {
        type Key = Blake3Hash;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            StorageBackend::set(&mut self.0, key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            StorageBackend::get(&self.0, key).await
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use dialog_storage::{Blake3Hash, DialogStorageError, MemoryStorageBackend, StorageBackend};

    /// Storage backend for non-Wasm targets using in-memory storage.
    ///
    /// This is only a placeholder implementation for testing purposes. The worker
    /// has no use case for being used in non-wasm contexts at this time.
    #[derive(Clone)]
    pub struct ServiceWorkerStorageBackend(MemoryStorageBackend<Blake3Hash, Vec<u8>>);
    impl ServiceWorkerStorageBackend {
        /// Creates a new in-memory storage backend instance.
        pub async fn new() -> Self {
            Self(MemoryStorageBackend::default())
        }
    }
    #[async_trait::async_trait]
    impl StorageBackend for ServiceWorkerStorageBackend {
        type Key = Blake3Hash;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            StorageBackend::set(&mut self.0, key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            StorageBackend::get(&self.0, key).await
        }
    }
}

pub use inner::*;
