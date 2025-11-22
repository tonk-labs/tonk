use crate::crypto::Keypair;
use base64::{engine::general_purpose::STANDARD, Engine};
use keyring::Entry;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

const SERVICE_NAME: &str = "tonk-cli";
const KEY_NAME: &str = "operator-keypair";

#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("Failed to access keyring: {0}")]
    KeyringError(#[from] keyring::Error),

    #[error("Invalid key data: {0}")]
    InvalidKeyData(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

enum KeystoreBackend {
    OsKeyring(Entry),
    File(PathBuf),
}

pub struct Keystore {
    backend: KeystoreBackend,
}

impl Keystore {
    /// Create a new keystore instance
    /// When TONK_HOME is set, uses file-based storage instead of OS keyring
    pub fn new() -> Result<Self, KeystoreError> {
        let backend = if let Some(home) = crate::util::home_dir() {
            // Check if TONK_HOME is set
            if std::env::var("TONK_HOME").is_ok() {
                // Use file-based storage
                let key_path = home.join(".tonk").join("operator-key");
                KeystoreBackend::File(key_path)
            } else {
                // Use OS keyring
                let entry = Entry::new(SERVICE_NAME, KEY_NAME)?;
                KeystoreBackend::OsKeyring(entry)
            }
        } else {
            // Fallback to OS keyring if no home directory
            let entry = Entry::new(SERVICE_NAME, KEY_NAME)?;
            KeystoreBackend::OsKeyring(entry)
        };

        Ok(Self { backend })
    }

    /// Get or create a keypair from the keystore
    /// If a keypair already exists, it will be loaded. Otherwise, a new one is generated and stored.
    pub fn get_or_create_keypair(&self) -> Result<Keypair, KeystoreError> {
        match self.get_keypair() {
            Ok(keypair) => Ok(keypair),
            Err(KeystoreError::KeyringError(keyring::Error::NoEntry)) | Err(KeystoreError::IoError(_)) => {
                let keypair = Keypair::generate();
                self.store_keypair(&keypair)?;
                Ok(keypair)
            }
            Err(e) => Err(e),
        }
    }

    /// Get an existing keypair from the keystore
    fn get_keypair(&self) -> Result<Keypair, KeystoreError> {
        let encoded = match &self.backend {
            KeystoreBackend::OsKeyring(entry) => entry.get_password()?,
            KeystoreBackend::File(path) => {
                fs::read_to_string(path)?
            }
        };

        let bytes = STANDARD
            .decode(&encoded.trim())
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

        match &self.backend {
            KeystoreBackend::OsKeyring(entry) => {
                entry.set_password(&encoded)?;
            }
            KeystoreBackend::File(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, encoded)?;
            }
        }

        Ok(())
    }

    /// Delete the stored keypair (for logout/reset)
    #[allow(dead_code)]
    pub fn delete_keypair(&self) -> Result<(), KeystoreError> {
        match &self.backend {
            KeystoreBackend::OsKeyring(entry) => {
                entry.delete_credential()?;
            }
            KeystoreBackend::File(path) => {
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
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
