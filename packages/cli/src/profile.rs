use crate::crypto::Keypair;
use crate::delegation::Delegation;
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Configuration for a profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Unique identifier for the profile
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Profile DID (did:key)
    pub did: String,

    /// Issuer DID of the authority from which this profile is derived
    pub authority_issuer: String,

    /// When the profile was created
    pub created_at: DateTime<Utc>,
}

impl Profile {
    /// Create a new profile configuration
    pub fn new(id: String, name: String, did: String, authority_issuer: String) -> Self {
        Self {
            id,
            name,
            did,
            authority_issuer,
            created_at: Utc::now(),
        }
    }

    /// Get the directory path for this profile
    pub fn profile_dir(&self) -> Result<PathBuf> {
        let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

        // Hash the authority issuer to create a stable directory name
        let authority_hash = {
            use sha2::Digest;
            let mut hasher = Sha256::new();
            hasher.update(self.authority_issuer.as_bytes());
            hex::encode(&hasher.finalize()[..8]) // Use first 8 bytes for readability
        };

        Ok(home
            .join(".tonk")
            .join("profiles")
            .join(&authority_hash)
            .join(&self.id))
    }

    /// Save the profile configuration
    pub fn save(&self) -> Result<()> {
        let profile_dir: PathBuf = self.profile_dir()?;

        // Create profile directory
        fs::create_dir_all(&profile_dir).context("Failed to create profile directory")?;

        // Save config.json
        let config_path = profile_dir.join("config.json");
        let json =
            serde_json::to_string_pretty(self).context("Failed to serialize profile config")?;

        fs::write(&config_path, json).context("Failed to write profile config")?;

        Ok(())
    }

    /// Save the profile keypair
    pub fn save_keypair(&self, keypair: &Keypair) -> Result<()> {
        let profile_dir: PathBuf = self.profile_dir()?;

        // Ensure directory exists
        fs::create_dir_all(&profile_dir).context("Failed to create profile directory")?;

        // Save keypair as JSON with the secret key bytes
        let key_data = serde_json::json!({
            "secret_key": hex::encode(keypair.to_bytes()),
            "public_key": hex::encode(keypair.verifying_key().as_bytes()),
            "did": keypair.to_did_key(),
        });

        let key_path = profile_dir.join("key.json");
        let json =
            serde_json::to_string_pretty(&key_data).context("Failed to serialize keypair")?;

        fs::write(&key_path, json).context("Failed to write keypair")?;

        Ok(())
    }

    /// Load the profile keypair
    pub fn load_keypair(&self) -> Result<Keypair> {
        let profile_dir: PathBuf = self.profile_dir()?;
        let key_path = profile_dir.join("key.json");

        let key_json = fs::read_to_string(&key_path).context("Failed to read profile keypair")?;
        let key_data: serde_json::Value =
            serde_json::from_str(&key_json).context("Failed to parse key data")?;

        let secret_key_hex = key_data["secret_key"]
            .as_str()
            .context("Missing secret_key field")?;

        let secret_key_bytes =
            hex::decode(secret_key_hex).context("Failed to decode secret key")?;

        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&secret_key_bytes);

        Ok(Keypair::from_bytes(&key_array))
    }

    /// Load a profile by authority and profile ID
    pub fn load(authority_issuer: &str, profile_id: &str) -> Result<Self> {
        let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

        // Hash the authority issuer
        let authority_hash = {
            use sha2::Digest;
            let mut hasher = Sha256::new();
            hasher.update(authority_issuer.as_bytes());
            hex::encode(&hasher.finalize()[..8])
        };

        let profile_dir = home
            .join(".tonk")
            .join("profiles")
            .join(&authority_hash)
            .join(profile_id);

        let config_path = profile_dir.join("config.json");
        let json = fs::read_to_string(&config_path).context("Failed to read profile config")?;

        serde_json::from_str(&json).context("Failed to parse profile config")
    }
}

/// Get the active authority delegation (powerline delegation)
fn get_active_authority_delegation() -> Result<Delegation> {
    let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

    // Get operator DID
    let keystore: Keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator: Keypair = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Find delegations in operator's access directory
    let access_dir = home.join(".tonk").join("access").join(&operator_did);

    if !access_dir.exists() {
        anyhow::bail!("No access directory found. Run 'tonk login' to authenticate.");
    }

    // Look for powerline delegations
    for entry in fs::read_dir(&access_dir).context("Failed to read access directory")? {
        let entry = entry?;
        let dir_path = entry.path();

        if !dir_path.is_dir() {
            continue;
        }

        // Read delegation files in this directory
        for delegation_entry in fs::read_dir(&dir_path)? {
            let delegation_entry = delegation_entry?;
            let delegation_path = delegation_entry.path();

            if !delegation_path.is_file()
                || delegation_path.extension().and_then(|e| e.to_str()) != Some("cbor")
            {
                continue;
            }

            // Parse delegation
            if let Ok(cbor_bytes) = fs::read(&delegation_path)
                && let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes)
            {
                // Check if valid and is powerline
                if delegation.is_valid() && delegation.is_powerline() {
                    return Ok(delegation);
                }
            }
        }
    }

    anyhow::bail!("No valid authority delegation found. Run 'tonk login' to authenticate.")
}

/// Derive a profile keypair from the active authority delegation
pub fn derive_from_authority(name: Option<String>) -> Result<(Profile, Keypair)> {
    println!("🔑 Deriving profile from authority delegation...\n");

    // Get active authority delegation
    let authority_delegation: Delegation =
        get_active_authority_delegation().context("Failed to get authority delegation")?;

    let authority_issuer = authority_delegation.issuer();

    // Get operator keypair (used as additional entropy)
    let keystore: Keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator: Keypair = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;

    // Derive profile keypair using HKDF
    // Input Key Material: authority delegation hash
    // Salt: operator's public key (for binding to this operator)
    // Info: "tonk-profile-v1" + profile_name
    let hash_bytes = authority_delegation
        .hash_bytes()
        .context("Failed to get delegation hash")?;

    let verifying_key = operator.verifying_key();
    let salt = verifying_key.as_bytes();
    let profile_name = name.unwrap_or_else(|| "default".to_string());
    let info = format!("tonk-profile-v1:{}", profile_name);

    let hkdf = Hkdf::<Sha256>::new(Some(salt), &hash_bytes);
    let mut profile_seed = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut profile_seed)
        .map_err(|_| anyhow::anyhow!("HKDF expansion failed"))?;

    // Create profile keypair from derived seed
    let profile_keypair = Keypair::from_bytes(&profile_seed);
    let profile_did = profile_keypair.to_did_key();

    // Generate unique ID for the profile
    let profile_id = Uuid::new_v4().to_string();

    println!("👤 Profile DID: {}", profile_did);
    println!("   Profile ID:  {}", profile_id);
    println!("   Authority:   {}\n", authority_issuer);

    // Create profile config
    let profile = Profile::new(profile_id, profile_name, profile_did, authority_issuer);

    Ok((profile, profile_keypair))
}

/// Get or create the default profile for the active authority
pub fn get_or_create_default() -> Result<(Profile, Keypair)> {
    // Get active authority delegation to determine which authority we're under
    let authority_delegation: Delegation =
        get_active_authority_delegation().context("Failed to get authority delegation")?;

    let authority_issuer = authority_delegation.issuer();

    // Check if a default profile already exists for this authority
    let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

    // Hash the authority issuer
    let authority_hash = {
        use sha2::Digest;
        let mut hasher = Sha256::new();
        hasher.update(authority_issuer.as_bytes());
        hex::encode(&hasher.finalize()[..8])
    };

    let profiles_dir = home.join(".tonk").join("profiles").join(&authority_hash);

    // Look for existing default profile
    if profiles_dir.exists() {
        for entry in fs::read_dir(&profiles_dir).context("Failed to read profiles directory")? {
            let entry = entry?;
            let profile_dir = entry.path();

            if !profile_dir.is_dir() {
                continue;
            }

            let config_path = profile_dir.join("config.json");
            if config_path.exists()
                && let Ok(json) = fs::read_to_string(&config_path)
                && let Ok(profile) = serde_json::from_str::<Profile>(&json)
                && profile.name == "default"
                && profile.authority_issuer == authority_issuer
            {
                println!("📂 Using existing default profile: {}\n", profile.did);
                let keypair = profile.load_keypair()?;
                return Ok((profile, keypair));
            }
        }
    }

    // No existing default profile found, create one
    println!("🆕 Creating default profile...\n");

    let (profile, keypair) = derive_from_authority(Some("default".to_string()))?;

    // Save profile and keypair
    profile.save().context("Failed to save profile")?;
    profile
        .save_keypair(&keypair)
        .context("Failed to save profile keypair")?;

    println!("   Saved to:  {}\n", profile.profile_dir()?.display());

    Ok((profile, keypair))
}
