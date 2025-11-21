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

/// Invite a collaborator to a space
pub async fn invite(email: String, space_name: Option<String>) -> Result<()> {
    println!("🎫 Inviting {} to space\n", email);

    // Load the space to invite to
    let global_config = GlobalConfig::load().context("Failed to load global config")?;

    let space_id = if let Some(name) = space_name {
        // TODO: Look up space by name
        println!("   Using space: {}", name);
        name
    } else if let Some(active_id) = global_config.active_space {
        println!("   Using active space");
        active_id
    } else {
        anyhow::bail!("No active space. Create one with 'tonk space create' or specify --space");
    };

    // Convert email to did:mailto
    let invitee_did = format!("did:mailto:{}", email);
    println!("   Invitee DID: {}\n", invitee_did);

    println!("⚠️  This command is not yet fully implemented.\n");
    println!("    When UCAN library is available, this will:\n");
    println!(
        "    1. Create UCAN delegation: Space → did:mailto:{}",
        email
    );
    println!(
        "    2. Store invitation in space VFS at /access/{}/{{}}",
        invitee_did
    );
    println!("    3. Generate membership keypair");
    println!("    4. Create delegation: Space → Membership DID");
    println!("    5. Derive invite code from invitation signature");
    println!("    6. Output invite file containing:");
    println!("       - Invite code (short secret)");
    println!("       - Invitation CID");
    println!("       - Space DID");
    println!("       - Membership delegation");
    println!("       - Time-limited invocation for reading invitation\n");

    // TODO: When UCAN library is available, implement invitation flow:
    //
    // 1. Load space config and keypair from ~/.tonk/spaces/{space_id}/
    //
    // 2. Create invitation delegation:
    //    - iss: space_did (or inviter's membership/profile DID)
    //    - aud: did:mailto:{email}
    //    - cmd: desired capabilities (e.g., "/read", "/write")
    //    - sub: space_did
    //    - exp: null (permanent) or future timestamp
    //
    // 3. Sign delegation with inviter's key
    //
    // 4. Compute invitation ID:
    //    invitation_id = hash(delegation)
    //
    // 5. Store delegation in space VFS:
    //    ~/.tonk/spaces/{space_id}/access/{did:mailto}/invitations/{invitation_id}.json
    //
    // 6. Derive invite code:
    //    full_secret = inviter.sign(invitation_id)
    //    invite_code = full_secret.slice(-5)  // Last 5 chars for user-friendly code
    //
    // 7. Generate membership keypair:
    //    membership_key = HKDF(delegation.signature, invite_code)
    //    membership_did = keypair_to_did_key(membership_key)
    //
    // 8. Create membership delegation:
    //    - iss: space_did
    //    - aud: membership_did
    //    - cmd: same capabilities as invitation
    //    - sub: space_did
    //    - exp: null (permanent)
    //
    // 9. Store membership delegation:
    //    ~/.tonk/spaces/{space_id}/access/{membership_did}/{hash}.json
    //
    // 10. Create time-limited UCAN invocation for reading invitation (expires in 7 days):
    //     - Allows reading /access/{did:mailto}/{invitation_id}
    //
    // 11. Create invite file containing:
    //     {
    //       "invite_code": "ABCDE",
    //       "invitation_id": "bafy...",
    //       "space_did": "did:key:z...",
    //       "space_name": "My Space",
    //       "inviter": "alice@example.com",
    //       "invitee": "bob@example.com",
    //       "membership_delegation": {...},
    //       "access_invocation": {...}  // Time-limited UCAN for reading invitation
    //     }
    //
    // 12. Save invite file to: ./space-{space_name}-invite-{timestamp}.json
    //
    // 13. Print instructions for sharing:
    //     - Send invite file to invitee
    //     - Invitee uses: tonk space join --invite <file>

    Ok(())
}
