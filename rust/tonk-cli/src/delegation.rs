use crate::crypto::Operator;
use anyhow::Result;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use dialog_credentials::Ed25519Signer;
use dialog_ucan::Delegation as UcanDelegation;
use dialog_ucan::subject::Subject;
use dialog_ucan::time::Timestamp;
use dialog_varsig::Did;
use dialog_varsig::eddsa::Ed25519Signature;
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::EncodeError;
use sha2_0_10::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::{collections::TryReserveError, convert::Infallible};
use thiserror::Error;

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

/// Wrapper around ucan::Delegation with storage and validation methods
#[derive(Debug, Clone)]
pub struct Delegation(UcanDelegation<Ed25519Signature>);

impl Delegation {
    /// Create from a UCAN delegation
    pub fn from_ucan(delegation: UcanDelegation<Ed25519Signature>) -> Self {
        Self(delegation)
    }

    /// Get the inner UCAN delegation
    pub fn inner(&self) -> &UcanDelegation<Ed25519Signature> {
        &self.0
    }

    /// Deserialize from DAG-CBOR bytes
    pub fn from_cbor_bytes(bytes: &[u8]) -> Result<Self, DelegationError> {
        let delegation: UcanDelegation<Ed25519Signature> = serde_ipld_dagcbor::from_slice(bytes)?;
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
        let now = Timestamp::now();
        self.0.expiration().is_none_or(|exp| exp > now)
            && self.0.not_before().is_none_or(|nbf| nbf <= now)
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
    pub fn subject(&self) -> &Subject {
        self.0.subject()
    }

    /// Check if this is a powerline delegation (subject is *)
    pub fn is_powerline(&self) -> bool {
        matches!(self.0.subject(), Subject::Any)
    }

    /// Get the command as a slash-separated string
    pub fn command_str(&self) -> String {
        self.0.command().to_string()
    }

    /// Get expiration timestamp (Unix epoch seconds)
    pub fn expiration(&self) -> Option<i64> {
        self.0.expiration().map(|ts| ts.to_unix() as i64)
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
            Subject::Specific(did) => {
                // Specific subject
                access_dir.join(did.to_string())
            }
            Subject::Any => {
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

    /// Save delegation to local storage
    pub fn save(&self) -> Result<(), DelegationError> {
        let cbor_path: PathBuf = self.storage_path()?;

        // Save as DAG-CBOR
        let cbor_bytes = self.to_cbor_bytes()?;
        fs::write(&cbor_path, cbor_bytes)?;
        println!("   Saved to: {}", cbor_path.display());

        Ok(())
    }

    /// Save delegation to local storage using provided raw CBOR bytes
    /// This preserves the exact bytes received without re-serialization
    pub fn save_raw(&self, raw_cbor: &[u8]) -> Result<(), DelegationError> {
        let cbor_path: PathBuf = self.storage_path()?;

        // Save the original CBOR bytes without re-serialization
        fs::write(&cbor_path, raw_cbor)?;
        println!("   Saved to: {}", cbor_path.display());

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

    /// Save delegation with metadata using provided raw CBOR bytes
    /// This preserves the exact bytes received without re-serialization
    pub fn save_raw_with_metadata(
        &self,
        raw_cbor: &[u8],
        metadata: &DelegationMetadata,
    ) -> Result<(), DelegationError> {
        // Save the delegation using raw bytes
        self.save_raw(raw_cbor)?;

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

        if cbor_path.exists() {
            fs::remove_file(cbor_path)?;
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

/// Convert an Operator to an Ed25519Signer (for signing delegations)
pub fn keypair_to_signer(operator: &Operator) -> Ed25519Signer {
    Ed25519Signer::from(operator)
}

/// Convert an Operator to a Did (for audience/subject)
pub fn keypair_to_did(operator: &Operator) -> Did {
    operator.did()
}

/// Create an ownership delegation (full access to a space)
/// This is used when creating a space: Space DID -> Profile DID
pub async fn create_ownership_delegation(
    issuer: &Operator,
    audience_did: &Did,
    subject_did: Did,
) -> Result<UcanDelegation<Ed25519Signature>, DelegationError> {
    let signer = Ed25519Signer::from(issuer);

    UcanDelegation::builder()
        .issuer(signer)
        .audience(audience_did)
        .subject(Subject::Specific(subject_did))
        .command(vec!["read".to_string(), "write".to_string()])
        .try_build()
        .await
        .map_err(|e| DelegationError::InvalidDelegation(e.to_string()))
}

/// Create a capability delegation with specific permissions
/// Commands: "read", "write", or both
pub async fn create_capability_delegation(
    issuer: &Operator,
    audience_did: &Did,
    subject_did: Did,
    capabilities: &[&str],
) -> Result<UcanDelegation<Ed25519Signature>, DelegationError> {
    let signer = Ed25519Signer::from(issuer);
    let command: Vec<String> = capabilities.iter().map(|s| s.to_string()).collect();

    UcanDelegation::builder()
        .issuer(signer)
        .audience(audience_did)
        .subject(Subject::Specific(subject_did))
        .command(command)
        .try_build()
        .await
        .map_err(|e| DelegationError::InvalidDelegation(e.to_string()))
}

/// Create a read-only delegation
pub async fn create_read_delegation(
    issuer: &Operator,
    audience_did: &Did,
    subject_did: Did,
) -> Result<UcanDelegation<Ed25519Signature>, DelegationError> {
    create_capability_delegation(issuer, audience_did, subject_did, &["read"]).await
}

/// Create a read-write delegation
pub async fn create_read_write_delegation(
    issuer: &Operator,
    audience_did: &Did,
    subject_did: Did,
) -> Result<UcanDelegation<Ed25519Signature>, DelegationError> {
    create_capability_delegation(issuer, audience_did, subject_did, &["read", "write"]).await
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
        let delegation = UcanDelegation::<Ed25519Signature>::deserialize(deserializer)?;
        Ok(Self(delegation))
    }
}

impl From<&Delegation> for tonk_space::Delegation {
    fn from(delegation: &Delegation) -> Self {
        tonk_space::Delegation::from(delegation.inner().clone())
    }
}

/// Inspect a base64-encoded CBOR delegation or a delegation file and print detailed information
pub fn inspect(input: String) -> Result<()> {
    println!("🔍 Inspecting delegation...\n");

    // Determine if input is a file path or base64 string
    let decoded = if std::path::Path::new(&input).exists() {
        // Read from file
        println!("📂 Reading from file: {}", input);
        fs::read(&input).map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?
    } else {
        // Decode base64
        println!("📥 Decoding base64...");
        base64::engine::general_purpose::STANDARD
            .decode(&input)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))?
    };

    println!("   ✓ Loaded {} bytes", decoded.len());
    println!(
        "   Hex (first 200): {}\n",
        hex::encode(&decoded[..decoded.len().min(200)])
    );

    // Try to parse as UCAN delegation
    println!("🎫 Parsing as UCAN delegation...");
    match Delegation::from_cbor_bytes(&decoded) {
        Ok(delegation) => {
            println!("   ✓ Valid UCAN delegation!\n");

            println!("📋 Delegation Details:");
            println!("   Issuer:   {}", delegation.issuer());
            println!("   Audience: {}", delegation.audience());
            println!("   Command:  {}", delegation.command_str());

            match delegation.subject() {
                Subject::Any => {
                    println!("   Subject:  * (powerline - any subject)");
                }
                Subject::Specific(did) => {
                    println!("   Subject:  {}", did);
                }
            }

            if let Some(exp) = delegation.expiration() {
                let exp_time = chrono::DateTime::from_timestamp(exp, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "invalid timestamp".to_string());
                println!("   Expires:  {} ({})", exp_time, exp);
            } else {
                println!("   Expires:  never");
            }

            if let Some(nbf) = delegation.0.not_before() {
                let nbf_secs = nbf.to_unix() as i64;
                let nbf_time = chrono::DateTime::from_timestamp(nbf_secs, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "invalid timestamp".to_string());
                println!("   Not before: {} ({})", nbf_time, nbf_secs);
            }

            println!("\n✅ Delegation is valid: {}", delegation.is_valid());

            // Check serialization roundtrip
            println!("\n🔄 Serialization roundtrip test:");
            match delegation.to_cbor_bytes() {
                Ok(reserialized) => {
                    println!("   Original size:    {} bytes", decoded.len());
                    println!("   Reserialized:     {} bytes", reserialized.len());
                    if decoded == reserialized {
                        println!("   ✓ Perfect roundtrip!");
                    } else {
                        println!("   ✗ Bytes differ after re-serialization");
                        println!(
                            "   Original (first 200):     {}",
                            hex::encode(&decoded[..decoded.len().min(200)])
                        );
                        println!(
                            "   Reserialized (first 200): {}",
                            hex::encode(&reserialized[..reserialized.len().min(200)])
                        );

                        // Find first difference
                        for (i, (a, b)) in decoded.iter().zip(reserialized.iter()).enumerate() {
                            if a != b {
                                println!("   First diff at byte {}: {:02x} -> {:02x}", i, a, b);
                                break;
                            }
                        }

                        // Try to parse the reserialized bytes
                        println!("\n   🧪 Testing if reserialized bytes can be parsed:");
                        match Delegation::from_cbor_bytes(&reserialized) {
                            Ok(_) => {
                                println!("   ✓ Reserialized bytes CAN be parsed!");
                            }
                            Err(e) => {
                                println!("   ✗ Reserialized bytes CANNOT be parsed!");
                                println!("   Error: {}", e);
                            }
                        }

                        // Save both versions for external analysis
                        println!("\n   💾 Saving versions for comparison:");
                        let temp_orig = "/tmp/delegation_original.cbor";
                        let temp_reser = "/tmp/delegation_reserialized.cbor";
                        std::fs::write(temp_orig, &decoded).ok();
                        std::fs::write(temp_reser, &reserialized).ok();
                        println!("   Original:      {}", temp_orig);
                        println!("   Reserialized:  {}", temp_reser);
                        println!("\n   You can analyze with: cbor2diag.rb or similar tools");
                        println!("   Or compare: xxd {} > /tmp/orig.txt", temp_orig);
                        println!("              xxd {} > /tmp/reser.txt", temp_reser);
                        println!("              diff /tmp/orig.txt /tmp/reser.txt")
                    }
                }
                Err(e) => {
                    println!("   ✗ Re-serialization failed: {}", e);
                }
            }
        }
        Err(e) => {
            println!("   ✗ Failed to parse as UCAN delegation");
            println!("   Error: {}\n", e);

            println!("❌ This delegation cannot be parsed by ucan");
            println!("   This may indicate:");
            println!("   - Incompatible UCAN library versions");
            println!("   - Different serialization format");
            println!("   - Corrupted or invalid data");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_query::Entity;
    use std::str::FromStr;

    #[tokio::test]
    async fn test_delegation_claim_cid_is_valid_uri() {
        // Create test operators
        let issuer = crate::crypto::Operator::generate();
        let audience = crate::crypto::Operator::generate();

        let signer = Ed25519Signer::from(&issuer);
        let audience_did = audience.did();
        let subject_did = issuer.did();

        let delegation: UcanDelegation<Ed25519Signature> = UcanDelegation::builder()
            .issuer(signer)
            .audience(&audience_did)
            .subject(Subject::Specific(subject_did))
            .command(vec![]) // Empty = root access "/"
            .try_build()
            .await
            .expect("Failed to build delegation");

        let delegation = Delegation::from_ucan(delegation);

        // Convert to tonk_space::Delegation
        let space_delegation: tonk_space::Delegation = (&delegation).into();

        // The CID should be a valid URI that Entity::from_str can parse
        let entity_str = format!("ucan:{}", space_delegation.cid());
        assert!(
            entity_str.starts_with("ucan:"),
            "CID should be formatted as ucan: URI: {}",
            entity_str
        );

        // Verify Entity::from_str can parse it
        let entity = Entity::from_str(&entity_str);
        assert!(
            entity.is_ok(),
            "CID should be parseable as Entity: {}",
            entity_str
        );
    }

    #[test]
    fn test_plain_hash_is_not_valid_entity() {
        // This test documents why we need the ucan: prefix.
        // A plain hash is NOT a valid URI and Entity::from_str will reject it.
        let plain_hash = "abc123def456";
        let result = Entity::from_str(plain_hash);
        assert!(
            result.is_err(),
            "Plain hash should NOT be parseable as Entity"
        );

        // But with ucan: prefix it works
        let ucan_uri = format!("ucan:{}", plain_hash);
        let result = Entity::from_str(&ucan_uri);
        assert!(result.is_ok(), "ucan: URI should be parseable as Entity");
    }
}
