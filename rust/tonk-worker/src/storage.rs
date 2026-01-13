//! Storage backend implementation for the service worker.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod inner {
    use async_trait::async_trait;
    use dialog_common::Blake3Hash;
    use wasm_bindgen::prelude::*;

    use dialog_storage::{
        DialogStorageError, IndexedDbStorageBackend, StorageBackend, TransactionalMemoryBackend,
    };

    /// Storage backend for the service worker using IndexedDB.
    ///
    /// This wraps an IndexedDB backend that provides both StorageBackend and
    /// TransactionalMemoryBackend implementations for use with Space.
    #[derive(Clone)]
    pub struct ServiceWorkerStorageBackend(IndexedDbStorageBackend<Vec<u8>, Vec<u8>>);

    impl ServiceWorkerStorageBackend {
        /// Creates a new storage backend instance.
        ///
        /// Initializes an IndexedDB backend for the service worker.
        ///
        /// # Arguments
        /// * `name` - The name for the IndexedDB database (typically the space/repository ID)
        pub async fn new(name: &str) -> Self {
            let backend = IndexedDbStorageBackend::new(name).await.unwrap_throw();
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
        type Key = Vec<u8>;
        type Value = Vec<u8>;
        type Error = DialogStorageError;

        async fn set(&mut self, key: Self::Key, value: Self::Value) -> Result<(), Self::Error> {
            StorageBackend::set(&mut self.0, key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            StorageBackend::get(&self.0, key).await
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
            TransactionalMemoryBackend::resolve(&self.0, address).await
        }

        async fn replace(
            &self,
            address: &Self::Address,
            edition: Option<&Self::Edition>,
            content: Option<Self::Value>,
        ) -> Result<Option<Self::Edition>, Self::Error> {
            TransactionalMemoryBackend::replace(&self.0, address, edition, content).await
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
        ///
        /// # Arguments
        /// * `_name` - The name for the storage (ignored for in-memory backend)
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
            StorageBackend::set(&mut self.0, key, value).await
        }

        async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, Self::Error> {
            StorageBackend::get(&self.0, key).await
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
            TransactionalMemoryBackend::resolve(&self.0, address).await
        }

        async fn replace(
            &self,
            address: &Self::Address,
            edition: Option<&Self::Edition>,
            content: Option<Self::Value>,
        ) -> Result<Option<Self::Edition>, Self::Error> {
            TransactionalMemoryBackend::replace(&self.0, address, edition, content).await
        }
    }
}

pub use inner::*;
