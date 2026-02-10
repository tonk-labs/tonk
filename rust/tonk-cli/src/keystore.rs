use crate::crypto::Operator;
use base64::{Engine, engine::general_purpose::STANDARD};
use keyring::Entry;
use thiserror::Error;

const SERVICE_NAME: &str = "tonk-cli";
const KEY_NAME: &str = "operator-keypair";

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("Failed to access keyring: {0}")]
    KeyringError(#[from] keyring::Error),

    #[error("Invalid key data: {0}")]
    InvalidKeyData(String),
}

pub struct Keystore {
    entry: Entry,
}

impl Keystore {
    /// Create a new keystore instance
    pub fn new() -> Result<Self, KeystoreError> {
        let entry = Entry::new(SERVICE_NAME, KEY_NAME)?;
        Ok(Self { entry })
    }

    /// Get or create an operator from the keystore
    /// If TONK_OPERATOR_KEY env var is set, uses that key (base58btc encoded)
    /// Otherwise uses OS keyring
    pub fn get_or_create_keypair(&self) -> Result<Operator, KeystoreError> {
        // Check for TONK_OPERATOR_KEY environment variable first
        if let Ok(operator_key) = std::env::var("TONK_OPERATOR_KEY") {
            return self.operator_from_env_key(&operator_key);
        }

        // Fall back to OS keyring
        match self.get_operator() {
            Ok(operator) => Ok(operator),
            Err(KeystoreError::KeyringError(keyring::Error::NoEntry)) => {
                let operator = Operator::generate();
                self.store_operator(&operator)?;
                Ok(operator)
            }
            Err(e) => Err(e),
        }
    }

    /// Load operator from base58btc-encoded key in TONK_OPERATOR_KEY
    fn operator_from_env_key(&self, key_b58: &str) -> Result<Operator, KeystoreError> {
        let key_bytes = bs58::decode(key_b58)
            .into_vec()
            .map_err(|e| KeystoreError::InvalidKeyData(format!("Invalid base58btc key: {}", e)))?;

        if key_bytes.len() != 32 {
            return Err(KeystoreError::InvalidKeyData(format!(
                "TONK_OPERATOR_KEY must be 32 bytes, got {}",
                key_bytes.len()
            )));
        }

        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&key_bytes);

        Ok(Operator::from_secret(bytes))
    }

    /// Get an existing operator from the keystore
    fn get_operator(&self) -> Result<Operator, KeystoreError> {
        let password = self.entry.get_password()?;
        let bytes = STANDARD
            .decode(&password)
            .map_err(|e| KeystoreError::InvalidKeyData(e.to_string()))?;

        if bytes.len() != 32 {
            return Err(KeystoreError::InvalidKeyData(format!(
                "Expected 32 bytes, got {}",
                bytes.len()
            )));
        }

        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(&bytes);

        Ok(Operator::from_secret(key_bytes))
    }

    /// Store an operator in the keystore
    fn store_operator(&self, operator: &Operator) -> Result<(), KeystoreError> {
        let bytes = operator.to_secret();
        let encoded = STANDARD.encode(bytes);
        self.entry.set_password(&encoded)?;
        Ok(())
    }

    /// Delete the stored operator (for logout/reset)
    #[allow(dead_code)]
    pub fn delete_keypair(&self) -> Result<(), KeystoreError> {
        self.entry.delete_credential()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires actual keyring access
    fn test_keystore_roundtrip() {
        let keystore = Keystore::new().unwrap();

        // Clean up any existing key
        let _ = keystore.delete_keypair();

        // First call should generate a new operator
        let op1 = keystore.get_or_create_keypair().unwrap();
        let did1 = op1.did().to_string();

        // Second call should retrieve the same operator
        let op2 = keystore.get_or_create_keypair().unwrap();
        let did2 = op2.did().to_string();

        assert_eq!(did1, did2);

        // Cleanup
        keystore.delete_keypair().unwrap();
    }
}
