//! Key storage for operator keys.
//!
//! This module provides `KeyStore`, which manages Ed25519 keys used for signing.
//!
//! # Storage Layout
//!
//! ```text
//! IndexedDB: "tonk-keys" (WASM) / In-memory HashMap (native)
//! └── Object Store: "keys"
//!     ├── "user"          → { secret: [u8; 32] }
//!     └── "space:{did}"   → { secret: [u8; 32] }
//! ```
//!
//! Note: Plan A uses extractable keys stored as secret bytes. Plan B will
//! re-introduce proper WebCrypto non-extractable keys for WASM.
//!
//! # Future Direction
//!
//! TODO: We should not be holding onto space keys as those are a liability.
//! Instead, we should store powerline delegations from space to owner (account/user).
//! That way accounts have complete authority over spaces without managing keys.
//! Unlike keys, those delegations can be kept public as they can't be exploited
//! without account keys. See: https://github.com/ucan-wg/delegation#powerline
//!
//! TODO: Consider using the existing `IndexedDbStorageBackend` instead of this
//! custom IDB implementation to reduce boilerplate.

#[cfg(target_arch = "wasm32")]
mod wasm {
    use idb::{Database, DatabaseEvent, Factory, KeyPath, ObjectStoreParams, TransactionMode};
    use js_sys::{Object, Reflect, Uint8Array};
    use std::sync::Arc;
    use thiserror::Error;
    use tonk_space::Operator;
    use wasm_bindgen::prelude::*;

    const DB_NAME: &str = "tonk-keys";
    const DB_VERSION: u32 = 2; // Bump version for schema change
    const STORE_NAME: &str = "keys";

    const USER_KEY: &str = "user";
    const SPACE_KEY_PREFIX: &str = "space:";

    /// Errors that can occur when working with the key store.
    #[derive(Debug, Error)]
    pub enum KeyStoreError {
        /// IndexedDB operation failed.
        #[error("IndexedDB error: {0}")]
        Idb(String),

        /// JavaScript error.
        #[error("JS error: {0}")]
        JsError(String),

        /// Invalid data in storage.
        #[error("Invalid data: {0}")]
        InvalidData(String),
    }

    impl From<idb::Error> for KeyStoreError {
        fn from(e: idb::Error) -> Self {
            KeyStoreError::Idb(format!("{:?}", e))
        }
    }

    /// Storage for operator keys in IndexedDB.
    ///
    /// Keys are stored as 32-byte secret arrays. This is Plan A's temporary
    /// approach using extractable keys; Plan B will use proper WebCrypto
    /// non-extractable keys.
    #[derive(Clone)]
    pub struct KeyStore {
        db: Arc<Database>,
    }

    // SAFETY: Web browsers run Wasm in a single thread.
    unsafe impl Send for KeyStore {}
    unsafe impl Sync for KeyStore {}

    impl KeyStore {
        /// Open the key store, creating the database if it doesn't exist.
        pub async fn open() -> Result<Self, KeyStoreError> {
            let factory = Factory::new().map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?;

            let mut open_request = factory
                .open(DB_NAME, Some(DB_VERSION))
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?;

            open_request.on_upgrade_needed(|event| {
                let db = event.database().expect("database should exist on upgrade");

                // Create object store if it doesn't exist
                if !db.store_names().into_iter().any(|name| name == STORE_NAME) {
                    let mut params = ObjectStoreParams::new();
                    params.key_path(Some(KeyPath::new_single("id")));
                    db.create_object_store(STORE_NAME, params)
                        .expect("failed to create object store");
                }
            });

            let db = open_request.await?;
            Ok(Self { db: Arc::new(db) })
        }

        /// Get the user's operator if one exists.
        pub async fn user_operator(&self) -> Result<Option<Operator>, KeyStoreError> {
            self.get_operator(USER_KEY).await
        }

        /// Create a new user operator.
        pub async fn create_user_operator(&self) -> Result<Operator, KeyStoreError> {
            let operator = Operator::generate();
            self.store_operator(USER_KEY, &operator).await?;
            Ok(operator)
        }

        /// Get a space's operator if one exists.
        pub async fn space_operator(
            &self,
            space_did: &str,
        ) -> Result<Option<Operator>, KeyStoreError> {
            let key = format!("{}{}", SPACE_KEY_PREFIX, space_did);
            self.get_operator(&key).await
        }

        /// Create a new space operator.
        ///
        /// Returns the new operator. The space DID is derived from the generated public key.
        pub async fn create_space_operator(&self) -> Result<Operator, KeyStoreError> {
            let operator = Operator::generate();
            let space_did = operator.did().to_string();
            let key = format!("{}{}", SPACE_KEY_PREFIX, space_did);
            self.store_operator(&key, &operator).await?;
            Ok(operator)
        }

        /// Store a space operator (for when you have an existing operator to store).
        pub async fn store_space_operator(
            &self,
            space_did: &str,
            operator: &Operator,
        ) -> Result<(), KeyStoreError> {
            let key = format!("{}{}", SPACE_KEY_PREFIX, space_did);
            self.store_operator(&key, operator).await
        }

        // Internal: Get an operator by key name
        async fn get_operator(&self, key: &str) -> Result<Option<Operator>, KeyStoreError> {
            let transaction = self
                .db
                .transaction(&[STORE_NAME], TransactionMode::ReadOnly)?;
            let store = transaction
                .object_store(STORE_NAME)
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?;

            let result: Option<JsValue> = store
                .get(JsValue::from_str(key))
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?
                .await?;

            transaction.await?;

            match result {
                None => Ok(None),
                Some(value) => {
                    // Extract secret from the stored object
                    let secret_js = Reflect::get(&value, &"secret".into())
                        .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

                    let secret_array = Uint8Array::new(&secret_js);
                    let mut secret_bytes = [0u8; 32];

                    if secret_array.length() != 32 {
                        return Err(KeyStoreError::InvalidData(format!(
                            "expected 32-byte secret, got {}",
                            secret_array.length()
                        )));
                    }

                    secret_array.copy_to(&mut secret_bytes);

                    let operator = Operator::from_secret(secret_bytes);
                    Ok(Some(operator))
                }
            }
        }

        // Internal: Store an operator by key name
        async fn store_operator(
            &self,
            key: &str,
            operator: &Operator,
        ) -> Result<(), KeyStoreError> {
            let transaction = self
                .db
                .transaction(&[STORE_NAME], TransactionMode::ReadWrite)?;
            let store = transaction
                .object_store(STORE_NAME)
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?;

            // Create an object with id and secret
            let obj = Object::new();

            Reflect::set(&obj, &"id".into(), &JsValue::from_str(key))
                .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

            // Store secret as Uint8Array
            let secret_bytes = operator.to_secret();
            let secret_array = Uint8Array::from(&secret_bytes[..]);

            Reflect::set(&obj, &"secret".into(), &secret_array)
                .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

            store
                .put(&obj, None)
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?
                .await?;

            transaction.commit()?.await?;

            Ok(())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use thiserror::Error;
    use tonk_space::Operator;

    /// Errors that can occur when working with the key store.
    #[derive(Debug, Error)]
    pub enum KeyStoreError {
        /// Storage operation failed.
        #[error("Storage error: {0}")]
        Storage(String),

        /// Invalid data in storage.
        #[error("Invalid data: {0}")]
        InvalidData(String),
    }

    /// In-memory key store for testing (non-WASM targets).
    ///
    /// This stores `Operator` instances for native testing purposes.
    #[derive(Clone, Default)]
    pub struct KeyStore {
        operators: Arc<RwLock<HashMap<String, Operator>>>,
    }

    impl KeyStore {
        /// Open the key store (in-memory for non-WASM).
        pub async fn open() -> Result<Self, KeyStoreError> {
            Ok(Self::default())
        }

        /// Get the user's operator if one exists.
        pub async fn user_operator(&self) -> Result<Option<Operator>, KeyStoreError> {
            let ops = self.operators.read().unwrap();
            Ok(ops.get("user").cloned())
        }

        /// Create a new user operator.
        pub async fn create_user_operator(&self) -> Result<Operator, KeyStoreError> {
            let operator = Operator::generate();
            let mut ops = self.operators.write().unwrap();
            ops.insert("user".to_string(), operator.clone());
            Ok(operator)
        }

        /// Get a space's operator if one exists.
        pub async fn space_operator(
            &self,
            space_did: &str,
        ) -> Result<Option<Operator>, KeyStoreError> {
            let ops = self.operators.read().unwrap();
            Ok(ops.get(&format!("space:{}", space_did)).cloned())
        }

        /// Create a new space operator.
        pub async fn create_space_operator(&self) -> Result<Operator, KeyStoreError> {
            let operator = Operator::generate();
            let space_did = operator.did().to_string();
            let mut ops = self.operators.write().unwrap();
            ops.insert(format!("space:{}", space_did), operator.clone());
            Ok(operator)
        }

        /// Store a space operator.
        pub async fn store_space_operator(
            &self,
            space_did: &str,
            operator: &Operator,
        ) -> Result<(), KeyStoreError> {
            let mut ops = self.operators.write().unwrap();
            ops.insert(format!("space:{}", space_did), operator.clone());
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
