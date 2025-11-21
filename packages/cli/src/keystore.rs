use crate::crypto::Keypair;
use base64::{engine::general_purpose::STANDARD, Engine};
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

    /// Get or create a keypair from the keystore
    /// If a keypair already exists, it will be loaded. Otherwise, a new one is generated and stored.
    pub fn get_or_create_keypair(&self) -> Result<Keypair, KeystoreError> {
        match self.get_keypair() {
            Ok(keypair) => Ok(keypair),
            Err(KeystoreError::KeyringError(keyring::Error::NoEntry)) => {
                let keypair = Keypair::generate();
                self.store_keypair(&keypair)?;
                Ok(keypair)
            }
            Err(e) => Err(e),
        }
    }

    /// Get an existing keypair from the keystore
    fn get_keypair(&self) -> Result<Keypair, KeystoreError> {
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

        Ok(Keypair::from_bytes(&key_bytes))
    }

    /// Store a keypair in the keystore
    fn store_keypair(&self, keypair: &Keypair) -> Result<(), KeystoreError> {
        let bytes = keypair.to_bytes();
        let encoded = STANDARD.encode(&bytes);
        self.entry.set_password(&encoded)?;
        Ok(())
    }

    /// Delete the stored keypair (for logout/reset)
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

        // First call should generate a new keypair
        let keypair1 = keystore.get_or_create_keypair().unwrap();
        let did1 = keypair1.to_did_key();

        // Second call should retrieve the same keypair
        let keypair2 = keystore.get_or_create_keypair().unwrap();
        let did2 = keypair2.to_did_key();

        assert_eq!(did1, did2);

        // Cleanup
        keystore.delete_keypair().unwrap();
    }
}
