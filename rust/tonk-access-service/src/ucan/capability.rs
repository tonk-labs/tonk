//! Capability definitions for blob operations.
//!
//! These follow the w3-blob specification pattern.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ucan::promise::Promised;

/// Blob digest (multihash)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobDigest {
    /// The multihash bytes
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

/// Blob metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobMeta {
    /// SHA-256 digest as multihash
    pub digest: BlobDigest,
    /// Size in bytes
    pub size: u64,
}

/// Arguments for blob/allocate capability
#[derive(Debug, Clone)]
pub struct BlobAllocate {
    /// Space DID where the blob will be stored
    pub space: String,
    /// Blob metadata
    pub blob: BlobMeta,
}

/// Arguments for blob/get capability
#[derive(Debug, Clone)]
pub struct BlobGet {
    /// Space DID where the blob is stored
    pub space: String,
    /// Blob digest to retrieve
    pub digest: BlobDigest,
}

/// Parsed capability
#[derive(Debug, Clone)]
pub enum Capability {
    BlobAllocate(BlobAllocate),
    BlobGet(BlobGet),
}

impl Capability {
    /// Parse capability from invocation command and arguments.
    pub fn from_invocation(
        command: &[String],
        arguments: &BTreeMap<String, Promised>,
    ) -> Result<Self, String> {
        let cmd_str = command.join("/");

        match cmd_str.as_str() {
            "blob/allocate" => {
                let space = get_string_arg(arguments, "space")?;
                let blob = get_blob_arg(arguments)?;
                Ok(Capability::BlobAllocate(BlobAllocate { space, blob }))
            }
            "blob/get" => {
                let space = get_string_arg(arguments, "space")?;
                let digest = get_digest_arg(arguments)?;
                Ok(Capability::BlobGet(BlobGet { space, digest }))
            }
            _ => Err(format!("Unknown capability: {}", cmd_str)),
        }
    }
}

fn get_string_arg(args: &BTreeMap<String, Promised>, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Promised::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("Argument '{}' must be a string", key)),
        None => Err(format!("Missing required argument: {}", key)),
    }
}

fn get_blob_arg(args: &BTreeMap<String, Promised>) -> Result<BlobMeta, String> {
    let blob = args.get("blob").ok_or("Missing required argument: blob")?;

    match blob {
        Promised::Map(map) => {
            let digest = match map.get("digest") {
                Some(Promised::Bytes(b)) => BlobDigest { bytes: b.clone() },
                Some(Promised::Map(m)) => {
                    // Handle IPLD bytes representation
                    match m.get("/") {
                        Some(Promised::Map(inner)) => match inner.get("bytes") {
                            Some(Promised::Bytes(b)) => BlobDigest { bytes: b.clone() },
                            _ => return Err("Invalid digest format".into()),
                        },
                        _ => return Err("Invalid digest format".into()),
                    }
                }
                _ => return Err("blob.digest must be bytes".into()),
            };

            let size = match map.get("size") {
                Some(Promised::Integer(n)) => *n as u64,
                // Also accept string to work around i128 deserialization issues
                Some(Promised::String(s)) => s
                    .parse::<u64>()
                    .map_err(|_| "blob.size must be a valid integer string")?,
                _ => return Err("blob.size must be an integer or string".into()),
            };

            Ok(BlobMeta { digest, size })
        }
        _ => Err("blob must be a map".into()),
    }
}

fn get_digest_arg(args: &BTreeMap<String, Promised>) -> Result<BlobDigest, String> {
    match args.get("digest") {
        Some(Promised::Bytes(b)) => Ok(BlobDigest { bytes: b.clone() }),
        Some(_) => Err("digest must be bytes".into()),
        None => Err("Missing required argument: digest".into()),
    }
}
