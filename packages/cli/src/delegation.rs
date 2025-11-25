use crate::crypto::Keypair;
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::EncodeError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::{collections::TryReserveError, convert::Infallible};
use thiserror::Error;
use ucan_core::did::{Ed25519Did, Ed25519Signer};
use ucan_core::time::timestamp::Timestamp;
use ucan_core::{Delegation as UcanDelegation, delegation::subject::DelegatedSubject};

#[derive(Error, Debug)]
pub enum DelegationError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("CBOR serialization error: {0}")]
    CborError(#[from] serde_ipld_dagcbor::DecodeError<Infallible>),

    #[error("CBOR encoding error: {0}")]
    CborEncodeError(String),

    #[error("Delegation not found")]
    NotFound,

    #[error("Invalid delegation: {0}")]
    InvalidDelegation(String),

    #[error("Delegation expired")]
    Expired,

    #[error("Delegation not yet valid (notBefore in future)")]
    NotYetValid,

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),
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

/// Wrapper around ucan_core::Delegation with storage and validation methods
#[derive(Debug, Clone)]
pub struct Delegation(UcanDelegation<Ed25519Did>);

impl Delegation {
    /// Create from a UCAN delegation
    pub fn from_ucan(delegation: UcanDelegation<Ed25519Did>) -> Self {
        Self(delegation)
    }

    /// Get the inner UCAN delegation
    pub fn inner(&self) -> &UcanDelegation<Ed25519Did> {
        &self.0
    }

    /// Deserialize from DAG-CBOR bytes
    pub fn from_cbor_bytes(bytes: &[u8]) -> Result<Self, DelegationError> {
        let delegation: UcanDelegation<Ed25519Did> = serde_ipld_dagcbor::from_slice(bytes)?;
        Ok(Self(delegation))
    }

    /// Serialize to DAG-CBOR bytes
    pub fn to_cbor_bytes(&self) -> Result<Vec<u8>, DelegationError> {
        serde_ipld_dagcbor::to_vec(&self.0).map_err(|e: EncodeError<TryReserveError>| {
            DelegationError::CborEncodeError(e.to_string())
        })
    }

    /// Check if the delegation is still valid (not expired and notBefore passed)
    pub fn is_valid(&self) -> bool {
        let now = chrono::Utc::now().timestamp() as u64;

        // Check expiration
        if let Some(exp) = self.0.expiration() {
            let exp_secs = exp.to_unix();
            if exp_secs <= now {
                return false;
            }
        }

        // Check notBefore
        if let Some(nbf) = self.0.not_before() {
            let nbf_secs = nbf.to_unix();
            if nbf_secs > now {
                return false;
            }
        }

        true
    }

    /// Get the audience (operator DID)
    pub fn audience(&self) -> String {
        self.0.audience().to_string()
    }

    /// Get the issuer (authority DID)
    pub fn issuer(&self) -> String {
        self.0.issuer().to_string()
    }

    /// Get the subject
    pub fn subject(&self) -> &DelegatedSubject<Ed25519Did> {
        self.0.subject()
    }

    /// Check if this is a powerline delegation (subject is *)
    pub fn is_powerline(&self) -> bool {
        matches!(self.0.subject(), DelegatedSubject::Any)
    }

    /// Get the command as a slash-separated string
    pub fn command_str(&self) -> String {
        self.0.command().join("/")
    }

    /// Get the commands
    pub fn command(&self) -> &Vec<String> {
        self.0.command()
    }

    /// Get expiration timestamp (Unix epoch seconds)
    pub fn expiration(&self) -> Option<i64> {
        self.0.expiration().map(|ts: Timestamp| ts.to_unix() as i64)
    }

    /// Calculate hash of the delegation (for storage path)
    fn hash(&self) -> Result<String, DelegationError> {
        let cbor_bytes = self.to_cbor_bytes()?;
        let mut hasher = Sha256::new();
        hasher.update(&cbor_bytes);
        let result = hasher.finalize();
        Ok(hex::encode(result))
    }

    /// Get the delegation storage path based on delegation fields
    fn storage_path(&self) -> Result<PathBuf, DelegationError> {
        let home: PathBuf = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        // Build path: ~/.tonk/access/{aud}/{sub or iss}/{exp}-{hash}.cbor
        let audience = self.audience();
        let access_dir = home.join(".tonk").join("access").join(&audience);

        let sub_dir = match self.subject() {
            DelegatedSubject::Specific(did) => {
                // Specific subject
                access_dir.join(did.to_string())
            }
            DelegatedSubject::Any => {
                // Powerline delegation - use issuer as directory name
                access_dir.join(self.issuer())
            }
        };

        // Create directory structure
        fs::create_dir_all(&sub_dir)?;

        let hash = self.hash()?;
        let exp = self.expiration().unwrap_or(0);
        let filename = format!("{}-{}.cbor", exp, hash);

        Ok(sub_dir.join(filename))
    }

    /// Save delegation to local storage (both CBOR and JSON)
    pub fn save(&self) -> Result<(), DelegationError> {
        let cbor_path: PathBuf = self.storage_path()?;

        // Save as DAG-CBOR (primary format)
        let cbor_bytes = self.to_cbor_bytes()?;
        fs::write(&cbor_path, cbor_bytes)?;
        println!("   Saved to: {}", cbor_path.display());

        // Save as JSON (for human inspection)
        let json_path = cbor_path.with_extension("json");
        let json = serde_json::to_string_pretty(&self.0)?;
        fs::write(&json_path, json)?;
        println!("   JSON export: {}", json_path.display());

        Ok(())
    }

    /// Save delegation with metadata
    pub fn save_with_metadata(&self, metadata: &DelegationMetadata) -> Result<(), DelegationError> {
        // Save the delegation first
        self.save()?;

        // Save metadata
        let home: PathBuf = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let hash: String = self.hash()?;
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
        let home: PathBuf = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let hash = self.hash()?;
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

    /// Load delegation from CBOR file (for future use)
    #[allow(dead_code)]
    pub fn load(
        aud: &str,
        sub: Option<&str>,
        exp: i64,
        hash: &str,
    ) -> Result<Self, DelegationError> {
        let home: PathBuf = crate::util::home_dir().ok_or_else(|| {
            DelegationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Could not determine home directory",
            ))
        })?;

        let access_dir = home.join(".tonk").join("access").join(aud);
        let sub_dir = if let Some(sub) = sub {
            access_dir.join(sub)
        } else {
            access_dir.join("*")
        };

        let filename = format!("{}-{}.cbor", exp, hash);
        let path = sub_dir.join(filename);

        if !path.exists() {
            return Err(DelegationError::NotFound);
        }

        let cbor_bytes = fs::read(path)?;
        Self::from_cbor_bytes(&cbor_bytes)
    }

    /// Delete the stored delegation (for future use)
    #[allow(dead_code)]
    pub fn delete(&self) -> Result<(), DelegationError> {
        let cbor_path: PathBuf = self.storage_path()?;
        let json_path = cbor_path.with_extension("json");

        if cbor_path.exists() {
            fs::remove_file(cbor_path)?;
        }
        if json_path.exists() {
            fs::remove_file(json_path)?;
        }
        Ok(())
    }

    /// Get the delegation hash as bytes
    /// This is used as a stable, deterministic identifier for the delegation
    ///
    /// Used for deriving membership keys from invitations
    pub fn hash_bytes(&self) -> Result<Vec<u8>, DelegationError> {
        let hash = self.hash()?;
        Ok(hex::decode(&hash).expect("hash is valid hex"))
    }
}

/// Convert a Keypair to an Ed25519Signer (for signing delegations)
pub fn keypair_to_signer(keypair: &Keypair) -> Ed25519Signer {
    let signing_key = SigningKey::from_bytes(&keypair.to_bytes());
    Ed25519Signer::new(signing_key)
}

/// Convert a Keypair to an Ed25519Did (for audience/subject)
pub fn keypair_to_did(keypair: &Keypair) -> Ed25519Did {
    let verifying_key = keypair.verifying_key();
    verifying_key.into()
}

/// Create an ownership delegation (full access to a space)
/// This is used when creating a space: Space DID → Profile DID
pub fn create_ownership_delegation(
    issuer_keypair: &Keypair,
    audience_keypair: &Keypair,
    subject_keypair: &Keypair,
) -> Result<UcanDelegation<Ed25519Did>, DelegationError> {
    let issuer_signer = keypair_to_signer(issuer_keypair);
    let audience_did = keypair_to_did(audience_keypair);
    let subject_did = keypair_to_did(subject_keypair);

    UcanDelegation::builder()
        .issuer(issuer_signer)
        .audience(audience_did)
        .subject(DelegatedSubject::Specific(subject_did))
        .command(vec!["read".to_string(), "write".to_string()])
        .try_build()
        .map_err(|e| DelegationError::InvalidDelegation(e.to_string()))
}

/// Create a capability delegation with specific permissions
/// Commands: "read", "write", or both
pub fn create_capability_delegation(
    issuer_keypair: &Keypair,
    audience_did: Ed25519Did,
    subject_did: Ed25519Did,
    capabilities: &[&str],
) -> Result<UcanDelegation<Ed25519Did>, DelegationError> {
    let issuer_signer = keypair_to_signer(issuer_keypair);
    let command: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();

    UcanDelegation::builder()
        .issuer(issuer_signer)
        .audience(audience_did)
        .subject(DelegatedSubject::Specific(subject_did))
        .command(command)
        .try_build()
        .map_err(|e| DelegationError::InvalidDelegation(e.to_string()))
}

/// Create a read-only delegation
pub fn create_read_delegation(
    issuer_keypair: &Keypair,
    audience_did: Ed25519Did,
    subject_did: Ed25519Did,
) -> Result<UcanDelegation<Ed25519Did>, DelegationError> {
    create_capability_delegation(issuer_keypair, audience_did, subject_did, &["read"])
}

/// Create a read-write delegation
pub fn create_read_write_delegation(
    issuer_keypair: &Keypair,
    audience_did: Ed25519Did,
    subject_did: Ed25519Did,
) -> Result<UcanDelegation<Ed25519Did>, DelegationError> {
    create_capability_delegation(
        issuer_keypair,
        audience_did,
        subject_did,
        &["read", "write"],
    )
}

// Implement Serialize/Deserialize by delegating to inner type
impl Serialize for Delegation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Delegation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let delegation = UcanDelegation::<Ed25519Did>::deserialize(deserializer)?;
        Ok(Self(delegation))
    }
}
