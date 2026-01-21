//! User data storage for identity and space secrets.
//!
//! This module provides a simple key-value store for user-scoped data that
//! persists across sessions. It uses a dedicated IndexedDB database separate
//! from space data.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod inner {
    use dialog_storage::IndexedDbStorageBackend;
    use thiserror::Error;

    /// Database name for user-scoped data.
    const USER_DB_NAME: &str = "tonk-user";

    /// Key for the user's identity secret.
    const IDENTITY_SECRET_KEY: &str = "identity:secret";

    /// Key prefix for space secrets.
    const SPACE_SECRET_PREFIX: &str = "spaces:secret:";

    /// Key for the default space DID.
    const DEFAULT_SPACE_KEY: &str = "spaces:default";

    /// Errors that can occur when working with the user store.
    #[derive(Debug, Error)]
    pub enum UserStoreError {
        /// Storage operation failed.
        #[error("Storage error: {0}")]
        Storage(String),

        /// Invalid data format.
        #[error("Invalid data: {0}")]
        InvalidData(String),
    }

    /// Storage for user-scoped data (identity, space secrets, preferences).
    ///
    /// Uses a dedicated IndexedDB database (`tonk-user`) separate from space data.
    /// This allows user identity to exist independently of any specific space.
    #[derive(Clone)]
    pub struct UserStore {
        db: IndexedDbStorageBackend<String, Vec<u8>>,
    }

    // SAFETY: Web browsers run Wasm in a single thread.
    unsafe impl Send for UserStore {}
    unsafe impl Sync for UserStore {}

    impl UserStore {
        /// Open the user store, creating the database if it doesn't exist.
        pub async fn open() -> Result<Self, UserStoreError> {
            let db = IndexedDbStorageBackend::new(USER_DB_NAME)
                .await
                .map_err(|e| UserStoreError::Storage(e.to_string()))?;
            Ok(Self { db })
        }

        /// Get the user's identity secret key bytes.
        ///
        /// Returns `None` if no identity has been created yet.
        pub async fn get_identity_secret(&self) -> Result<Option<[u8; 32]>, UserStoreError> {
            self.get_secret(IDENTITY_SECRET_KEY).await
        }

        /// Store the user's identity secret key bytes.
        pub async fn set_identity_secret(&mut self, secret: [u8; 32]) -> Result<(), UserStoreError> {
            self.set_secret(IDENTITY_SECRET_KEY, secret).await
        }

        /// Get a space's secret key bytes.
        ///
        /// Returns `None` if the space secret is not stored.
        pub async fn get_space_secret(
            &self,
            space_did: &str,
        ) -> Result<Option<[u8; 32]>, UserStoreError> {
            let key = format!("{}{}", SPACE_SECRET_PREFIX, space_did);
            self.get_secret(&key).await
        }

        /// Store a space's secret key bytes.
        pub async fn set_space_secret(
            &mut self,
            space_did: &str,
            secret: [u8; 32],
        ) -> Result<(), UserStoreError> {
            let key = format!("{}{}", SPACE_SECRET_PREFIX, space_did);
            self.set_secret(&key, secret).await
        }

        /// Get the default space DID.
        ///
        /// Returns `None` if no default space has been set.
        pub async fn get_default_space(&self) -> Result<Option<String>, UserStoreError> {
            use dialog_storage::StorageBackend;

            match self.db.get(&DEFAULT_SPACE_KEY.to_string()).await {
                Ok(Some(bytes)) => {
                    let did = String::from_utf8(bytes)
                        .map_err(|e| UserStoreError::InvalidData(e.to_string()))?;
                    Ok(Some(did))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(UserStoreError::Storage(e.to_string())),
            }
        }

        /// Set the default space DID.
        pub async fn set_default_space(&mut self, space_did: &str) -> Result<(), UserStoreError> {
            use dialog_storage::StorageBackend;

            self.db
                .set(DEFAULT_SPACE_KEY.to_string(), space_did.as_bytes().to_vec())
                .await
                .map_err(|e| UserStoreError::Storage(e.to_string()))
        }

        // Internal helper to get a 32-byte secret
        async fn get_secret(&self, key: &str) -> Result<Option<[u8; 32]>, UserStoreError> {
            use dialog_storage::StorageBackend;

            match self.db.get(&key.to_string()).await {
                Ok(Some(bytes)) => {
                    let secret: [u8; 32] = bytes.try_into().map_err(|_| {
                        UserStoreError::InvalidData(format!(
                            "Expected 32 bytes for secret, got different length"
                        ))
                    })?;
                    Ok(Some(secret))
                }
                Ok(None) => Ok(None),
                Err(e) => Err(UserStoreError::Storage(e.to_string())),
            }
        }

        // Internal helper to set a 32-byte secret
        async fn set_secret(&mut self, key: &str, secret: [u8; 32]) -> Result<(), UserStoreError> {
            use dialog_storage::StorageBackend;

            self.db
                .set(key.to_string(), secret.to_vec())
                .await
                .map_err(|e| UserStoreError::Storage(e.to_string()))
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod inner {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use thiserror::Error;

    /// Errors that can occur when working with the user store.
    #[derive(Debug, Error)]
    pub enum UserStoreError {
        /// Storage operation failed.
        #[error("Storage error: {0}")]
        Storage(String),

        /// Invalid data format.
        #[error("Invalid data: {0}")]
        InvalidData(String),
    }

    /// In-memory user store for testing (non-WASM targets).
    #[derive(Clone, Default)]
    pub struct UserStore {
        data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    }

    impl UserStore {
        /// Open the user store (in-memory for non-WASM).
        pub async fn open() -> Result<Self, UserStoreError> {
            Ok(Self::default())
        }

        /// Get the user's identity secret key bytes.
        pub async fn get_identity_secret(&self) -> Result<Option<[u8; 32]>, UserStoreError> {
            self.get_secret("identity:secret").await
        }

        /// Store the user's identity secret key bytes.
        pub async fn set_identity_secret(&mut self, secret: [u8; 32]) -> Result<(), UserStoreError> {
            self.set_secret("identity:secret", secret).await
        }

        /// Get a space's secret key bytes.
        pub async fn get_space_secret(
            &self,
            space_did: &str,
        ) -> Result<Option<[u8; 32]>, UserStoreError> {
            let key = format!("spaces:secret:{}", space_did);
            self.get_secret(&key).await
        }

        /// Store a space's secret key bytes.
        pub async fn set_space_secret(
            &mut self,
            space_did: &str,
            secret: [u8; 32],
        ) -> Result<(), UserStoreError> {
            let key = format!("spaces:secret:{}", space_did);
            self.set_secret(&key, secret).await
        }

        /// Get the default space DID.
        pub async fn get_default_space(&self) -> Result<Option<String>, UserStoreError> {
            let data = self.data.read().unwrap();
            match data.get("spaces:default") {
                Some(bytes) => {
                    let did = String::from_utf8(bytes.clone())
                        .map_err(|e| UserStoreError::InvalidData(e.to_string()))?;
                    Ok(Some(did))
                }
                None => Ok(None),
            }
        }

        /// Set the default space DID.
        pub async fn set_default_space(&mut self, space_did: &str) -> Result<(), UserStoreError> {
            let mut data = self.data.write().unwrap();
            data.insert("spaces:default".to_string(), space_did.as_bytes().to_vec());
            Ok(())
        }

        async fn get_secret(&self, key: &str) -> Result<Option<[u8; 32]>, UserStoreError> {
            let data = self.data.read().unwrap();
            match data.get(key) {
                Some(bytes) => {
                    let secret: [u8; 32] = bytes.clone().try_into().map_err(|_| {
                        UserStoreError::InvalidData("Expected 32 bytes for secret".to_string())
                    })?;
                    Ok(Some(secret))
                }
                None => Ok(None),
            }
        }

        async fn set_secret(&mut self, key: &str, secret: [u8; 32]) -> Result<(), UserStoreError> {
            let mut data = self.data.write().unwrap();
            data.insert(key.to_string(), secret.to_vec());
            Ok(())
        }
    }
}

pub use inner::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_stores_and_retrieves_identity_secret() {
        let mut store = UserStore::open().await.unwrap();
        let secret = [42u8; 32];

        assert!(store.get_identity_secret().await.unwrap().is_none());

        store.set_identity_secret(secret).await.unwrap();

        let retrieved = store.get_identity_secret().await.unwrap();
        assert_eq!(retrieved, Some(secret));
    }

    #[tokio::test]
    async fn it_stores_and_retrieves_space_secret() {
        let mut store = UserStore::open().await.unwrap();
        let space_did = "did:key:z6MkTestSpace";
        let secret = [123u8; 32];

        assert!(store.get_space_secret(space_did).await.unwrap().is_none());

        store.set_space_secret(space_did, secret).await.unwrap();

        let retrieved = store.get_space_secret(space_did).await.unwrap();
        assert_eq!(retrieved, Some(secret));
    }

    #[tokio::test]
    async fn it_stores_and_retrieves_default_space() {
        let mut store = UserStore::open().await.unwrap();
        let space_did = "did:key:z6MkTestSpace";

        assert!(store.get_default_space().await.unwrap().is_none());

        store.set_default_space(space_did).await.unwrap();

        let retrieved = store.get_default_space().await.unwrap();
        assert_eq!(retrieved, Some(space_did.to_string()));
    }
}
