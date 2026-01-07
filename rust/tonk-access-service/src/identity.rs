//! Service identity management
//!
//! The service DID is generated on first request and stored in KV.
//! This ensures the same identity is used across all worker instances.

use ed25519_dalek::SigningKey;
use ucan::did::{Ed25519Did, Ed25519Signer};
use worker::kv::KvStore;

const SERVICE_KEY_KV_KEY: &str = "service_identity";

/// Errors that can occur during identity operations
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("KV error: {0}")]
    Kv(String),

    #[error("Invalid key bytes")]
    InvalidKey,

    #[error("Failed to generate key")]
    Generation,
}

/// Service identity (Ed25519 keypair)
pub struct ServiceIdentity {
    signer: Ed25519Signer,
}

impl ServiceIdentity {
    /// Get or create the service identity from KV storage.
    ///
    /// On first call, generates a new keypair and stores it.
    /// On subsequent calls, loads the existing keypair.
    pub async fn get_or_create(kv: &KvStore) -> Result<Self, IdentityError> {
        // Try to load existing key
        if let Some(key_bytes) = kv
            .get(SERVICE_KEY_KV_KEY)
            .bytes()
            .await
            .map_err(|e| IdentityError::Kv(e.to_string()))?
        {
            let key_array: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| IdentityError::InvalidKey)?;

            let signing_key = SigningKey::from_bytes(&key_array);
            let signer = Ed25519Signer::new(signing_key);

            return Ok(Self { signer });
        }

        // Generate new key
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|_| IdentityError::Generation)?;

        let signing_key = SigningKey::from_bytes(&seed);

        // Store in KV
        kv.put_bytes(SERVICE_KEY_KV_KEY, &seed)
            .map_err(|e| IdentityError::Kv(e.to_string()))?
            .execute()
            .await
            .map_err(|e| IdentityError::Kv(e.to_string()))?;

        let signer = Ed25519Signer::new(signing_key);
        Ok(Self { signer })
    }

    /// Get the service DID
    pub fn did(&self) -> &Ed25519Did {
        self.signer.did()
    }

    /// Get the service DID as a string
    pub fn did_string(&self) -> String {
        self.signer.did().to_string()
    }

    /// Get the signer for creating signatures
    pub fn signer(&self) -> &Ed25519Signer {
        &self.signer
    }
}
