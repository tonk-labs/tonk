//! Storage abstraction for content-addressed delegation chain backups.
//!
//! A production implementation backs this with Cloudflare R2; tests and
//! local development use [`MemoryChainStore`]. Both are namespaced by
//! an account's root DID, so ceremony logic elsewhere in this crate is
//! written once, generically over [`ChainStore`].

/// Errors surfaced by a [`ChainStore`] implementation.
#[derive(Debug)]
pub enum ChainError {
    /// An unexpected storage failure.
    Internal(String),
}

/// Storage backend for content-addressed delegation chain bytes,
/// namespaced by account root DID.
///
/// Methods are plain `async fn`: callers are always generic over
/// `ChainStore`, never `dyn ChainStore`.
#[allow(async_fn_in_trait)]
pub trait ChainStore {
    /// Store `bytes` under `key` in `root_did`'s namespace.
    async fn put(&self, root_did: &str, key: &str, bytes: &[u8]) -> Result<(), ChainError>;

    /// List the keys stored in `root_did`'s namespace.
    async fn list(&self, root_did: &str) -> Result<Vec<String>, ChainError>;

    /// Look up the bytes stored under `key` in `root_did`'s namespace.
    async fn get(&self, root_did: &str, key: &str) -> Result<Option<Vec<u8>>, ChainError>;
}

#[cfg(any(test, feature = "helpers"))]
mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::{ChainError, ChainStore};

    /// An in-memory [`ChainStore`], keyed `"{root_did}/{key}"`.
    ///
    /// Intended for tests and local development; not durable.
    #[derive(Default)]
    pub struct MemoryChainStore(Mutex<HashMap<String, Vec<u8>>>);

    impl ChainStore for MemoryChainStore {
        async fn put(&self, root_did: &str, key: &str, bytes: &[u8]) -> Result<(), ChainError> {
            let mut store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            store.insert(format!("{root_did}/{key}"), bytes.to_vec());
            Ok(())
        }

        async fn list(&self, root_did: &str) -> Result<Vec<String>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            let prefix = format!("{root_did}/");
            Ok(store
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix).map(str::to_string))
                .collect())
        }

        async fn get(&self, root_did: &str, key: &str) -> Result<Option<Vec<u8>>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            Ok(store.get(&format!("{root_did}/{key}")).cloned())
        }
    }
}

#[cfg(any(test, feature = "helpers"))]
pub use memory::MemoryChainStore;
