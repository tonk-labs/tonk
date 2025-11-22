use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DelegationError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Delegation not found")]
    NotFound,

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Invalid issuer DID: {0}")]
    InvalidIssuerDid(String),

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),
}

/// Represents a capability delegation payload (simplified UCAN)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationPayload {
    /// Issuer DID (authority)
    pub iss: String,

    /// Audience DID (operator)
    pub aud: String,

    /// Command/capability
    pub cmd: String,

    /// Subject (null for account-level)
    pub sub: Option<String>,

    /// Expiration timestamp (Unix epoch seconds)
    pub exp: i64,

    /// Policy (empty for now)
    #[serde(default)]
    pub pol: Vec<serde_json::Value>,
}

/// Represents a signed delegation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delegation {
    /// The delegation payload
    pub payload: DelegationPayload,

    /// Signature over the payload
    pub signature: String,
}

/// Metadata about how a delegation was obtained
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationMetadata {
    /// URL of the site/service that issued the delegation
    pub site: String,

    /// Timestamp when the delegation was received
    pub received_at: DateTime<Utc>,

    /// Whether the site was local (served by CLI) or remote
    pub is_local: bool,

    /// Additional metadata
    #[serde(default)]
    pub extra: serde_json::Value,
}

impl Delegation {
    /// Check if the delegation is still valid (not expired)
    pub fn is_valid(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.payload.exp > now
    }

    /// Get the audience (operator DID)
    pub fn audience(&self) -> &str {
        &self.payload.aud
    }

    /// Get the issuer (authority DID)
    pub fn issuer(&self) -> &str {
        &self.payload.iss
    }

    /// Verify the signature using the issuer's public key
    pub fn verify(&self) -> Result<(), DelegationError> {
        // Extract public key from issuer DID (did:key:z...)
        let issuer_pubkey = Self::extract_pubkey_from_did(&self.payload.iss)?;

        // Serialize payload for verification
        let payload_bytes = serde_json::to_string(&self.payload)
            .map_err(|e| DelegationError::SerializationError(e))?;

        // Decode signature
        let signature_bytes = STANDARD.decode(&self.signature)?;

        // Verify signature
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let verifying_key = VerifyingKey::from_bytes(&issuer_pubkey)
            .map_err(|_| DelegationError::InvalidIssuerDid("Invalid public key".to_string()))?;

        let signature = Signature::from_bytes(
            &signature_bytes
                .try_into()
                .map_err(|_| DelegationError::InvalidSignature)?,
        );

        verifying_key
            .verify(payload_bytes.as_bytes(), &signature)
            .map_err(|_| DelegationError::InvalidSignature)?;

        Ok(())
    }

    /// Extract Ed25519 public key from did:key
    fn extract_pubkey_from_did(did: &str) -> Result<[u8; 32], DelegationError> {
        // did:key:z{base58btc-multicodec-pubkey}
        if !did.starts_with("did:key:z") {
            return Err(DelegationError::InvalidIssuerDid(
                "DID must start with 'did:key:z'".to_string(),
            ));
        }

        let encoded = &did[9..]; // Remove "did:key:z" prefix

        // Base58 decode
        let decoded = bs58::decode(encoded).into_vec().map_err(|e| {
            DelegationError::InvalidIssuerDid(format!("Base58 decode failed: {}", e))
        })?;

        // Check multicodec prefix (0xed01 for Ed25519)
        if decoded.len() < 34 || decoded[0] != 0xed || decoded[1] != 0x01 {
            return Err(DelegationError::InvalidIssuerDid(
                "Invalid multicodec prefix (expected 0xed01 for Ed25519)".to_string(),
            ));
        }

        // Extract 32-byte public key
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&decoded[2..34]);

        Ok(pubkey)
    }

    /// Calculate hash of the delegation
    pub fn hash(&self) -> String {
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }

    /// Get the delegation storage path based on delegation fields
    fn storage_path(&self) -> Result<PathBuf, DelegationError> {
        let home = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        // Build path: ~/.tonk/access/{aud}/{sub or iss}/{exp}-{hash}.json
        // When sub is null (powerline), use issuer DID as directory name
        let access_dir = home.join(".tonk").join("access").join(&self.payload.aud);

        let sub_dir = if let Some(ref sub) = self.payload.sub {
            access_dir.join(sub)
        } else {
            // Powerline delegation - use issuer as directory name
            access_dir.join(&self.payload.iss)
        };

        // Create directory structure
        fs::create_dir_all(&sub_dir)?;

        let hash = self.hash();
        let filename = format!("{}-{}.json", self.payload.exp, hash);

        Ok(sub_dir.join(filename))
    }

    /// Save delegation to local storage
    pub fn save(&self) -> Result<(), DelegationError> {
        let path = self.storage_path()?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&path, json)?;
        println!("   Saved to: {}", path.display());
        Ok(())
    }

    /// Save delegation with metadata
    pub fn save_with_metadata(&self, metadata: &DelegationMetadata) -> Result<(), DelegationError> {
        // Save the delegation first
        self.save()?;

        // Save metadata
        let home = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let hash = self.hash();
        let meta_dir = home.join(".tonk").join("meta").join(&hash);
        fs::create_dir_all(&meta_dir)?;

        let meta_path = meta_dir.join("site.json");
        let meta_json = serde_json::to_string_pretty(metadata)?;
        fs::write(&meta_path, meta_json)?;

        println!("   Metadata saved to: {}", meta_path.display());
        Ok(())
    }

    /// Load metadata for this delegation
    pub fn load_metadata(&self) -> Result<Option<DelegationMetadata>, DelegationError> {
        let home = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let hash = self.hash();
        let meta_path = home
            .join(".tonk")
            .join("meta")
            .join(&hash)
            .join("site.json");

        if !meta_path.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(meta_path)?;
        let metadata: DelegationMetadata = serde_json::from_str(&json)?;
        Ok(Some(metadata))
    }

    /// Load delegation from local storage (for future use)
    #[allow(dead_code)]
    pub fn load(
        aud: &str,
        sub: Option<&str>,
        exp: i64,
        hash: &str,
    ) -> Result<Self, DelegationError> {
        let home = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let access_dir = home.join(".tonk").join("access").join(aud);
        let sub_dir = if let Some(sub) = sub {
            access_dir.join(sub)
        } else {
            access_dir.join("null")
        };

        let filename = format!("{}-{}.json", exp, hash);
        let path = sub_dir.join(filename);

        if !path.exists() {
            return Err(DelegationError::NotFound);
        }

        let json = fs::read_to_string(path)?;
        let delegation: Delegation = serde_json::from_str(&json)?;
        Ok(delegation)
    }

    /// Delete the stored delegation (for future use)
    #[allow(dead_code)]
    pub fn delete(&self) -> Result<(), DelegationError> {
        let path = self.storage_path()?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
