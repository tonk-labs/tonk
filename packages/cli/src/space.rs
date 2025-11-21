use crate::config::GlobalConfig;
use crate::crypto::Keypair;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Configuration for a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceConfig {
    /// Unique identifier for the space
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Space DID (did:key)
    pub did: String,

    /// When the space was created
    pub created_at: DateTime<Utc>,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SpaceConfig {
    /// Create a new space configuration
    pub fn new(id: String, name: String, did: String, description: Option<String>) -> Self {
        Self {
            id,
            name,
            did,
            created_at: Utc::now(),
            description,
        }
    }

    /// Get the directory path for this space
    pub fn space_dir(&self) -> Result<PathBuf> {
        let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

        Ok(home.join(".tonk").join("spaces").join(&self.id))
    }

    /// Save the space configuration
    pub fn save(&self) -> Result<()> {
        let space_dir: PathBuf = self.space_dir()?;

        // Create space directory
        fs::create_dir_all(&space_dir).context("Failed to create space directory")?;

        // Save config.json
        let config_path = space_dir.join("config.json");
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize space config")?;

        fs::write(&config_path, json).context("Failed to write space config")?;

        Ok(())
    }

    /// Save the space keypair
    pub fn save_keypair(&self, keypair: &Keypair) -> Result<()> {
        let space_dir: PathBuf = self.space_dir()?;

        // Ensure directory exists
        fs::create_dir_all(&space_dir).context("Failed to create space directory")?;

        // Save keypair as JSON with the secret key bytes
        let key_data = serde_json::json!({
            "secret_key": hex::encode(keypair.to_bytes()),
            "public_key": hex::encode(keypair.verifying_key().as_bytes()),
            "did": keypair.to_did_key(),
        });

        let key_path = space_dir.join("key.json");
        let json =
            serde_json::to_string_pretty(&key_data).context("Failed to serialize keypair")?;

        fs::write(&key_path, json).context("Failed to write keypair")?;

        Ok(())
    }
}

/// Create a new space
pub async fn create(name: String, description: Option<String>) -> Result<()> {
    println!("🚀 Creating space: {}\n", name);

    // Generate space keypair
    let space_keypair = Keypair::generate();
    let space_did = space_keypair.to_did_key();

    // Generate unique ID for the space
    let space_id = Uuid::new_v4().to_string();

    println!("🏠 Space DID: {}", space_did);
    println!("   Space ID:  {}\n", space_id);

    // Create space config
    let space_config = SpaceConfig::new(
        space_id.clone(),
        name.clone(),
        space_did.clone(),
        description,
    );

    // Save space configuration
    space_config
        .save()
        .context("Failed to save space configuration")?;

    // Save space keypair
    space_config
        .save_keypair(&space_keypair)
        .context("Failed to save space keypair")?;

    let path: PathBuf = space_config.space_dir()?;
    println!("   Saved to:  {}\n", path.display());

    // Update global config to set this as the active space
    let mut global_config: GlobalConfig =
        GlobalConfig::load().context("Failed to load global config")?;

    global_config.active_space = Some(space_id);
    global_config
        .save()
        .context("Failed to save global config")?;

    println!("✅ Space created and set as active!\n");

    // TODO: When UCAN library is available, implement ownership delegation:
    //
    // 1. Get or create the active profile (derive from authority if needed)
    // 2. Create UCAN delegation: Space DID → Profile DID with full capabilities
    //    - iss: space_did
    //    - aud: profile_did
    //    - cmd: "/" (powerline - full access)
    //    - sub: space_did (the space is the subject)
    //    - exp: far future or null (permanent access)
    // 3. Sign the delegation with the space keypair
    // 4. Store delegation in space's authorization space:
    //    ~/.tonk/spaces/{space_id}/access/{profile_did}/{hash}.json
    // 5. Add profile_did to owners array in config (as cache/index)
    // 6. Discard the space private key after delegation

    Ok(())
}
