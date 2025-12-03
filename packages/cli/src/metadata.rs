use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Get the .tonk directory path
fn tonk_dir() -> Result<PathBuf> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".tonk"))
}

/// Session metadata stored in ~/.tonk/meta/{authority-did}/session.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Display name (defaults to DID)
    pub name: String,

    /// Authentication service used (e.g., "https://auth.tonk.xyz" or "local")
    pub via: String,

    /// ISO 8601 timestamp when session was created
    pub created_at: String,
}

impl SessionMetadata {
    /// Create new session metadata
    pub fn new(name: String, via: String) -> Self {
        let created_at = chrono::Utc::now().to_rfc3339();
        Self {
            name,
            via,
            created_at,
        }
    }

    /// Load session metadata from ~/.tonk/meta/{authority-did}/session.json
    pub fn load(authority_did: &str) -> Result<Option<Self>> {
        let meta_file = tonk_dir()?
            .join("meta")
            .join(authority_did)
            .join("session.json");

        if !meta_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&meta_file)
            .context("Failed to read session metadata")?;

        let metadata: SessionMetadata = serde_json::from_str(&content)
            .context("Failed to parse session metadata")?;

        Ok(Some(metadata))
    }

    /// Save session metadata to ~/.tonk/meta/{authority-did}/session.json
    pub fn save(&self, authority_did: &str) -> Result<()> {
        let meta_dir = tonk_dir()?.join("meta").join(authority_did);
        fs::create_dir_all(&meta_dir)?;

        let meta_file = meta_dir.join("session.json");
        let json = serde_json::to_string_pretty(self)?;

        fs::write(&meta_file, json)
            .context("Failed to write session metadata")?;

        Ok(())
    }
}

/// Space metadata stored in ~/.tonk/meta/{space-did}/space.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceMetadata {
    /// Display name for the space
    pub name: String,

    /// ISO 8601 timestamp when space was created
    pub created_at: String,

    /// DIDs of space owners
    pub owners: Vec<String>,
}

impl SpaceMetadata {
    /// Create new space metadata
    pub fn new(name: String, owners: Vec<String>) -> Self {
        let created_at = chrono::Utc::now().to_rfc3339();
        Self {
            name,
            created_at,
            owners,
        }
    }

    /// Load space metadata from ~/.tonk/meta/{space-did}/space.json
    pub fn load(space_did: &str) -> Result<Option<Self>> {
        let meta_file = tonk_dir()?
            .join("meta")
            .join(space_did)
            .join("space.json");

        if !meta_file.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&meta_file)
            .context("Failed to read space metadata")?;

        let metadata: SpaceMetadata = serde_json::from_str(&content)
            .context("Failed to parse space metadata")?;

        Ok(Some(metadata))
    }

    /// Save space metadata to ~/.tonk/meta/{space-did}/space.json
    pub fn save(&self, space_did: &str) -> Result<()> {
        let meta_dir = tonk_dir()?.join("meta").join(space_did);
        fs::create_dir_all(&meta_dir)?;

        let meta_file = meta_dir.join("space.json");
        let json = serde_json::to_string_pretty(self)?;

        fs::write(&meta_file, json)
            .context("Failed to write space metadata")?;

        Ok(())
    }
}
