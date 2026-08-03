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

/// Independent mutable head slots for named and legacy unnamed spot artifacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotHeadSlot {
    /// Preferred head written by clients carrying repository-name metadata.
    Named,
    /// Compatibility head written by legacy or retrying unnamed clients.
    Unnamed,
}

#[cfg(any(test, feature = "helpers", target_arch = "wasm32"))]
impl SpotHeadSlot {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Unnamed => "unnamed",
        }
    }
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

    /// Point one hashed spot subject at its current immutable blob.
    async fn put_spot_head(
        &self,
        root_did: &str,
        subject_key: &str,
        slot: SpotHeadSlot,
        blob_key: &str,
    ) -> Result<(), ChainError>;

    /// Read one immutable blob key from a subject's independent head slot.
    async fn spot_head(
        &self,
        root_did: &str,
        subject_key: &str,
        slot: SpotHeadSlot,
    ) -> Result<Option<String>, ChainError>;

    /// List every `(hashed subject, blob key)` head in one slot for an account.
    async fn list_spot_heads(
        &self,
        root_did: &str,
        slot: SpotHeadSlot,
    ) -> Result<Vec<(String, String)>, ChainError>;
}

#[cfg(any(test, feature = "helpers"))]
mod memory {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::{ChainError, ChainStore, SpotHeadSlot};

    /// In-memory immutable blobs and mutable subject heads.
    #[derive(Default)]
    struct MemoryState {
        blobs: HashMap<String, Vec<u8>>,
        heads: HashMap<String, String>,
    }

    /// An in-memory [`ChainStore`], keyed `"{root_did}/{key}"`.
    ///
    /// Intended for tests and local development; not durable.
    #[derive(Default)]
    pub struct MemoryChainStore(Mutex<MemoryState>);

    impl ChainStore for MemoryChainStore {
        async fn put(&self, root_did: &str, key: &str, bytes: &[u8]) -> Result<(), ChainError> {
            let mut store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            store
                .blobs
                .insert(format!("{root_did}/{key}"), bytes.to_vec());
            Ok(())
        }

        async fn list(&self, root_did: &str) -> Result<Vec<String>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            let prefix = format!("{root_did}/");
            let mut keys: Vec<_> = store
                .blobs
                .keys()
                .filter_map(|key| key.strip_prefix(&prefix).map(str::to_string))
                .collect();
            keys.sort();
            Ok(keys)
        }

        async fn get(&self, root_did: &str, key: &str) -> Result<Option<Vec<u8>>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            Ok(store.blobs.get(&format!("{root_did}/{key}")).cloned())
        }

        async fn put_spot_head(
            &self,
            root_did: &str,
            subject_key: &str,
            slot: SpotHeadSlot,
            blob_key: &str,
        ) -> Result<(), ChainError> {
            let mut store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            store.heads.insert(
                format!("{}/{root_did}/{subject_key}", slot.as_str()),
                blob_key.to_string(),
            );
            Ok(())
        }

        async fn spot_head(
            &self,
            root_did: &str,
            subject_key: &str,
            slot: SpotHeadSlot,
        ) -> Result<Option<String>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            Ok(store
                .heads
                .get(&format!("{}/{root_did}/{subject_key}", slot.as_str()))
                .cloned())
        }

        async fn list_spot_heads(
            &self,
            root_did: &str,
            slot: SpotHeadSlot,
        ) -> Result<Vec<(String, String)>, ChainError> {
            let store = self
                .0
                .lock()
                .map_err(|_| ChainError::Internal("chain store lock poisoned".to_string()))?;
            let prefix = format!("{}/{root_did}/", slot.as_str());
            let mut heads: Vec<_> = store
                .heads
                .iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix(&prefix)
                        .map(|subject| (subject.to_string(), value.clone()))
                })
                .collect();
            heads.sort();
            Ok(heads)
        }
    }
}

#[cfg(any(test, feature = "helpers"))]
pub use memory::MemoryChainStore;

#[cfg(target_arch = "wasm32")]
pub mod r2;
