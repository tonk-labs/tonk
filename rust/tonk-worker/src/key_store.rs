//! Key storage for operator keys.
//!
//! This module provides `KeyStore`, which manages Ed25519 keys used for signing.
//!
//! # Storage Layout
//!
//! ```text
//! IndexedDB: "tonk-keys" (WASM) / In-memory HashMap (native)
//! └── Object Store: "keys"
//!     ├── "user"          → { id, cryptoKey: CryptoKey, did: string } (WebCrypto)
//!     │                   → { id, secret: Uint8Array }                (Fallback)
//!     └── "space:{did}"   → { id, secret: Uint8Array }                (always extractable)
//! ```
//!
//! User identity keys use WebCrypto non-extractable keys when available.
//! Space keys remain extractable (they'll be replaced by UCAN delegations).
//!
//! # Future Direction
//!
//! TODO: We should not be holding onto space keys as those are a liability.
//! Instead, we should store powerline delegations from space to owner (account/user).
//! That way accounts have complete authority over spaces without managing keys.
//! Unlike keys, those delegations can be kept public as they can't be exploited
//! without account keys. See: https://github.com/ucan-wg/delegation#powerline

#[cfg(target_arch = "wasm32")]
mod wasm {
    use dialog_artifacts::replica::{CryptoKey, SigningAuthority, WebCryptoEd25519Signer};
    use idb::{Database, DatabaseEvent, Factory, KeyPath, ObjectStoreParams, TransactionMode};
    use js_sys::{Object, Reflect, Uint8Array};
    use std::sync::Arc;
    use thiserror::Error;
    use tonk_space::Operator;
    use ucan::did::Did as UcanDid;
    use wasm_bindgen::prelude::*;

    const DB_NAME: &str = "tonk-keys";
    const DB_VERSION: u32 = 3; // Bump version for WebCrypto schema
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

        /// WebCrypto error.
        #[error("WebCrypto error: {0}")]
        WebCrypto(String),
    }

    impl From<idb::Error> for KeyStoreError {
        fn from(e: idb::Error) -> Self {
            KeyStoreError::Idb(format!("{:?}", e))
        }
    }

    /// Storage for operator keys in IndexedDB.
    ///
    /// User identity keys use WebCrypto non-extractable keys when available.
    /// Space keys are stored as extractable secret bytes.
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
        ///
        /// Returns a `SigningAuthority` which may be either WebCrypto or Fallback variant.
        pub async fn user_operator(&self) -> Result<Option<SigningAuthority>, KeyStoreError> {
            self.get_user_operator(USER_KEY).await
        }

        /// Create a new user operator using WebCrypto when available.
        ///
        /// This will attempt to use WebCrypto non-extractable keys. If WebCrypto
        /// Ed25519 is not available, it falls back to extractable keys.
        pub async fn create_user_operator(&self) -> Result<SigningAuthority, KeyStoreError> {
            // Use SigningAuthority::generate() which handles WebCrypto fallback
            let operator = SigningAuthority::generate()
                .await
                .map_err(|e| KeyStoreError::WebCrypto(e.to_string()))?;
            self.store_user_operator(USER_KEY, &operator).await?;
            Ok(operator)
        }

        /// Get a space's operator if one exists.
        ///
        /// Space operators are always extractable (stored as secret bytes).
        pub async fn space_operator(
            &self,
            space_did: &str,
        ) -> Result<Option<Operator>, KeyStoreError> {
            let key = format!("{}{}", SPACE_KEY_PREFIX, space_did);
            self.get_space_operator(&key).await
        }

        /// Store a space operator (for when you have an existing operator to store).
        ///
        /// Space operators are always stored as extractable secret bytes.
        pub async fn store_space_operator(&self, operator: &Operator) -> Result<(), KeyStoreError> {
            let space_did = operator.did().to_string();
            let key = format!("{}{}", SPACE_KEY_PREFIX, space_did);
            self.store_extractable_operator(&key, operator).await
        }

        /// Get a user operator (may be WebCrypto or Fallback).
        async fn get_user_operator(
            &self,
            key: &str,
        ) -> Result<Option<SigningAuthority>, KeyStoreError> {
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
                    // Check if this is a WebCrypto key (has cryptoKey field)
                    let crypto_key_js = Reflect::get(&value, &"cryptoKey".into())
                        .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

                    if !crypto_key_js.is_undefined() {
                        // WebCrypto key: reconstruct from CryptoKey + DID
                        let crypto_key: CryptoKey = crypto_key_js.unchecked_into();

                        let did_js = Reflect::get(&value, &"did".into())
                            .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

                        let did_str = did_js.as_string().ok_or_else(|| {
                            KeyStoreError::InvalidData("did is not a string".into())
                        })?;

                        // Parse the DID to get public key bytes
                        let ed25519_did: ucan::did::Ed25519Did = did_str.parse().map_err(|e| {
                            KeyStoreError::InvalidData(format!("invalid DID: {:?}", e))
                        })?;

                        let public_key_bytes =
                            <ucan::did::Ed25519Did as UcanDid>::verifier(&ed25519_did).to_bytes();

                        // Reconstruct WebCryptoEd25519Signer
                        let signer = WebCryptoEd25519Signer::from_key(crypto_key, public_key_bytes)
                            .map_err(|e| KeyStoreError::WebCrypto(format!("{:?}", e)))?;

                        Ok(Some(SigningAuthority::from_webcrypto_signer(signer)))
                    } else {
                        // Fallback key: reconstruct from secret bytes
                        let secret_js = Reflect::get(&value, &"secret".into())
                            .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

                        if secret_js.is_undefined() {
                            return Err(KeyStoreError::InvalidData(
                                "stored key has neither cryptoKey nor secret".into(),
                            ));
                        }

                        let secret_array = Uint8Array::new(&secret_js);
                        let mut secret_bytes = [0u8; 32];

                        if secret_array.length() != 32 {
                            return Err(KeyStoreError::InvalidData(format!(
                                "expected 32-byte secret, got {}",
                                secret_array.length()
                            )));
                        }

                        secret_array.copy_to(&mut secret_bytes);

                        Ok(Some(SigningAuthority::from_secret(&secret_bytes)))
                    }
                }
            }
        }

        /// Store a user operator (handles both WebCrypto and Fallback).
        async fn store_user_operator(
            &self,
            key: &str,
            operator: &SigningAuthority,
        ) -> Result<(), KeyStoreError> {
            let transaction = self
                .db
                .transaction(&[STORE_NAME], TransactionMode::ReadWrite)?;
            let store = transaction
                .object_store(STORE_NAME)
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?;

            let obj = Object::new();

            Reflect::set(&obj, &"id".into(), &JsValue::from_str(key))
                .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

            // Check if this is a WebCrypto operator
            if let Some(signer) = operator.webcrypto_signer() {
                // Store CryptoKey + DID
                Reflect::set(&obj, &"cryptoKey".into(), signer.crypto_key())
                    .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

                Reflect::set(
                    &obj,
                    &"did".into(),
                    &JsValue::from_str(&signer.did().to_string()),
                )
                .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;
            } else {
                // Store secret bytes (Fallback or Native)
                let secret_bytes = operator.secret_key_bytes().ok_or_else(|| {
                    KeyStoreError::InvalidData("operator has no secret bytes".into())
                })?;

                let secret_array = Uint8Array::from(&secret_bytes[..]);

                Reflect::set(&obj, &"secret".into(), &secret_array)
                    .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;
            }

            store
                .put(&obj, None)
                .map_err(|e| KeyStoreError::Idb(format!("{:?}", e)))?
                .await?;

            transaction.commit()?.await?;

            Ok(())
        }

        /// Get a space operator (always extractable).
        async fn get_space_operator(&self, key: &str) -> Result<Option<Operator>, KeyStoreError> {
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

                    Ok(Some(Operator::from_secret(secret_bytes)))
                }
            }
        }

        /// Store an extractable operator (for space keys).
        async fn store_extractable_operator(
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

            let obj = Object::new();

            Reflect::set(&obj, &"id".into(), &JsValue::from_str(key))
                .map_err(|e| KeyStoreError::JsError(format!("{:?}", e)))?;

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
    use dialog_artifacts::replica::SigningAuthority;
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

    /// Storage for user operator (as SigningAuthority) and space operators (as tonk_space::Operator).
    #[derive(Clone, Default)]
    struct OperatorStorage {
        user: Option<SigningAuthority>,
        spaces: HashMap<String, Operator>,
    }

    /// In-memory key store for testing (non-WASM targets).
    ///
    /// User operators are stored as `SigningAuthority` (matching WASM behavior).
    /// Space operators are stored as `tonk_space::Operator` (extractable).
    #[derive(Clone, Default)]
    pub struct KeyStore {
        storage: Arc<RwLock<OperatorStorage>>,
    }

    impl KeyStore {
        /// Open the key store (in-memory for non-WASM).
        pub async fn open() -> Result<Self, KeyStoreError> {
            Ok(Self::default())
        }

        /// Get the user's operator if one exists.
        pub async fn user_operator(&self) -> Result<Option<SigningAuthority>, KeyStoreError> {
            let storage = self.storage.read().unwrap();
            Ok(storage.user.clone())
        }

        /// Create a new user operator.
        pub async fn create_user_operator(&self) -> Result<SigningAuthority, KeyStoreError> {
            let operator = SigningAuthority::generate()
                .await
                .map_err(|e| KeyStoreError::Storage(e.to_string()))?;
            let mut storage = self.storage.write().unwrap();
            storage.user = Some(operator.clone());
            Ok(operator)
        }

        /// Get a space's operator if one exists.
        pub async fn space_operator(
            &self,
            space_did: &str,
        ) -> Result<Option<Operator>, KeyStoreError> {
            let storage = self.storage.read().unwrap();
            Ok(storage.spaces.get(&format!("space:{}", space_did)).cloned())
        }

        /// Create a new space operator.
        pub async fn create_space_operator(&self) -> Result<Operator, KeyStoreError> {
            let operator = Operator::generate().await;
            let space_did = operator.did().to_string();
            let mut storage = self.storage.write().unwrap();
            storage
                .spaces
                .insert(format!("space:{}", space_did), operator.clone());
            Ok(operator)
        }

        /// Store a space operator.
        pub async fn store_space_operator(&self, operator: &Operator) -> Result<(), KeyStoreError> {
            let space_did = operator.did();
            let mut storage = self.storage.write().unwrap();
            storage
                .spaces
                .insert(format!("space:{}", space_did), operator.clone());
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
