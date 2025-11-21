use crate::authority;
use crate::config::GlobalConfig;
use crate::crypto::Keypair;
use crate::delegation::{Delegation, DelegationPayload};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

/// Configuration for a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceConfig {
    /// Human-readable name
    pub name: String,

    /// Space DID (did:key) - serves as unique identifier
    pub did: String,

    /// Owner DIDs - authorities that have full control over the space
    pub owners: Vec<String>,

    /// When the space was created
    pub created_at: DateTime<Utc>,

    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl SpaceConfig {
    /// Create a new space configuration
    pub fn new(name: String, did: String, owners: Vec<String>, description: Option<String>) -> Self {
        Self {
            name,
            did,
            owners,
            created_at: Utc::now(),
            description,
        }
    }

    /// Get the directory path for this space
    pub fn space_dir(&self) -> Result<PathBuf> {
        let home: PathBuf = dirs::home_dir().context("Could not determine home directory")?;

        // Use DID as the directory name (spaces are uniquely identified by their DID)
        Ok(home.join(".tonk").join("spaces").join(&self.did))
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
pub async fn create(name: String, owners: Option<Vec<String>>, description: Option<String>) -> Result<()> {
    println!("🚀 Creating space: {}\n", name);

    // Get active authority (required for space creation)
    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;

    println!("👤 Authority: {}\n", authority::format_authority_did(&authority.did));

    // Collect owner DIDs
    let mut owner_dids = Vec::new();

    // Always include the active authority as an owner
    owner_dids.push(authority.did.clone());

    // Handle additional owners
    if let Some(provided_owners) = owners {
        // Validate and add provided owners
        for owner in &provided_owners {
            if !owner.starts_with("did:key:") {
                anyhow::bail!("Invalid owner DID: {}. Must be a did:key identifier", owner);
            }
            if owner != &authority.did && !owner_dids.contains(owner) {
                owner_dids.push(owner.clone());
            }
        }
    } else {
        // Interactive mode: ask for additional owners
        println!("Additional owners (did:key identifiers, one per line, empty line to finish):");
        loop {
            print!("> ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();

            if input.is_empty() {
                break;
            }

            if !input.starts_with("did:key:") {
                println!("   ⚠  Invalid DID format. Must start with 'did:key:'");
                continue;
            }

            if input == authority.did {
                println!("   ℹ  Authority already included as owner");
                continue;
            }

            if owner_dids.contains(&input.to_string()) {
                println!("   ℹ  Already added");
                continue;
            }

            owner_dids.push(input.to_string());
            println!("   ✓ Added");
        }
        println!();
    }

    println!("👥 Owners ({}):", owner_dids.len());
    for owner in &owner_dids {
        println!("   • {}", authority::format_authority_did(owner));
    }
    println!();

    // Generate space keypair
    let space_keypair = Keypair::generate();
    let space_did = space_keypair.to_did_key();

    println!("🏠 Space DID: {}\n", space_did);

    // Create space config
    let space_config = SpaceConfig::new(
        name.clone(),
        space_did.clone(),
        owner_dids.clone(),
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
    println!("   Config saved to: {}\n", path.display());

    // Create delegations from space to each owner
    println!("📜 Creating owner delegations...");
    for owner_did in &owner_dids {
        create_owner_delegation(&space_keypair, &space_did, owner_did)?;
        println!("   ✓ {}", authority::format_authority_did(owner_did));
    }
    println!();

    // Update global config to set this as the active space
    let mut global_config: GlobalConfig =
        GlobalConfig::load().context("Failed to load global config")?;

    global_config.active_space = Some(space_did.clone());
    global_config
        .save()
        .context("Failed to save global config")?;

    println!("✅ Space created and set as active!");
    println!("   Operators under these authorities now have access to the space.\n");

    Ok(())
}

/// Create a delegation from space to an owner
fn create_owner_delegation(
    space_keypair: &Keypair,
    space_did: &str,
    owner_did: &str,
) -> Result<()> {
    // Create delegation payload: Space → Owner with full access
    let payload = DelegationPayload {
        iss: space_did.to_string(),
        aud: owner_did.to_string(),
        cmd: "/".to_string(), // Full access
        sub: Some(space_did.to_string()), // Subject is the space itself
        exp: i64::MAX, // Never expires (permanent ownership)
        pol: Vec::new(),
    };

    // Sign the payload
    let payload_json = serde_json::to_string(&payload)?;
    let signature_obj = space_keypair.sign(payload_json.as_bytes());
    let signature = STANDARD.encode(signature_obj.to_bytes());

    // Create delegation
    let delegation = Delegation {
        payload,
        signature,
    };

    // Verify the signature
    delegation.verify().context("Failed to verify delegation signature")?;

    // Save delegation to owner's access directory
    // This goes to: ~/.tonk/access/{owner_did}/{space_did}/{exp}-{hash}.json
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let owner_access_dir = home
        .join(".tonk")
        .join("access")
        .join(owner_did)
        .join(space_did);

    fs::create_dir_all(&owner_access_dir)?;

    let hash = delegation.hash();
    let filename = format!("{}-{}.json", delegation.payload.exp, hash);
    let delegation_path = owner_access_dir.join(filename);

    let delegation_json = serde_json::to_string_pretty(&delegation)?;
    fs::write(&delegation_path, delegation_json)?;

    Ok(())
}

/// Invite a collaborator to a space
pub async fn invite(email: String, space_name: Option<String>) -> Result<()> {
    println!("🎫 Inviting {} to space\n", email);

    // Load the space to invite to
    let global_config = GlobalConfig::load().context("Failed to load global config")?;

    let _space_id = if let Some(name) = space_name {
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

/// Join a space using an invitation file
pub async fn join(invite_path: String, profile_name: Option<String>) -> Result<()> {
    println!("🔗 Joining space with invitation\n");

    println!("   Invite file: {}", invite_path);

    let _profile = if let Some(name) = profile_name {
        println!("   Using profile: {}\n", name);
        name
    } else {
        println!("   Using active profile\n");
        "default".to_string()
    };

    println!("⚠️  This command is not yet fully implemented.\n");
    println!("    When UCAN library is available, this will:\n");
    println!("    1. Decode the invite file");
    println!("    2. Use time-limited invocation to read invitation from space");
    println!("    3. Extract invitation signature");
    println!("    4. Prompt for invite code from email");
    println!("    5. Derive membership key: HKDF(signature, code)");
    println!("    6. Create delegation: Membership → Profile");
    println!("    7. Store delegations in space");
    println!("    8. Mark space as joined\n");

    // TODO: When UCAN library is available, implement join flow:
    //
    // 1. Read and parse invite file:
    //    let invite_data = fs::read_to_string(&invite_path)?;
    //    let invite: InviteFile = serde_json::from_str(&invite_data)?;
    //
    //    Expected structure:
    //    {
    //      "invite_code": "ABCDE",
    //      "invitation_id": "bafy...",
    //      "space_did": "did:key:z...",
    //      "space_name": "My Space",
    //      "inviter": "alice@example.com",
    //      "invitee": "bob@example.com",
    //      "membership_delegation": {...},
    //      "access_invocation": {...}  // Time-limited UCAN for reading invitation
    //    }
    //
    // 2. Use the time-limited access_invocation to read the invitation:
    //    - The invocation allows reading /access/{did:mailto}/{invitation_id}
    //    - This is valid for 7 days after invitation creation
    //    - Read invitation delegation from space
    //
    // 3. Extract the invitation signature:
    //    invitation_signature = invitation_delegation.signature
    //
    // 4. Prompt user to enter the invite code from their email:
    //    print!("Enter invite code from email: ");
    //    let mut code = String::new();
    //    std::io::stdin().read_line(&mut code)?;
    //    let code = code.trim();
    //
    // 5. Derive membership keypair:
    //    membership_key = HKDF(invitation_signature, invite_code)
    //    membership_did = keypair_to_did_key(membership_key)
    //
    // 6. Verify the derived membership DID matches the one in membership_delegation:
    //    if membership_did != membership_delegation.aud {
    //        bail!("Invalid invite code - membership DID doesn't match");
    //    }
    //
    // 7. Get or create the profile to join with:
    //    - Load profile keypair (or derive from authority if needed)
    //    - profile_did = profile.to_did_key()
    //
    // 8. Create delegation from Membership → Profile:
    //    - iss: membership_did
    //    - aud: profile_did
    //    - cmd: same capabilities as membership has (from membership_delegation)
    //    - sub: space_did
    //    - exp: null (permanent)
    //    - Sign with membership_key
    //
    // 9. Store the membership delegation in space VFS:
    //    ~/.tonk/spaces/{space_id}/access/{membership_did}/{hash}.json
    //    (This delegation was created by inviter: Space → Membership)
    //
    // 10. Store the profile delegation in space VFS:
    //     ~/.tonk/spaces/{space_id}/access/{profile_did}/{hash}.json
    //     (This delegation we just created: Membership → Profile)
    //
    // 11. Also store delegations locally for this profile to access:
    //     ~/.tonk/access/{profile_did}/spaces/{space_id}-membership.json
    //     ~/.tonk/access/{profile_did}/spaces/{space_id}-profile.json
    //
    // 12. Create local space config if it doesn't exist:
    //     - id: derive from space_did or use invitation_id
    //     - name: from invite file
    //     - did: space_did
    //     - Save to ~/.tonk/spaces/{space_id}/config.json
    //     - Note: We don't have the space private key (only owners do)
    //
    // 13. Update global config:
    //     - Add space to list of joined spaces
    //     - Optionally set as active space
    //
    // 14. Success message:
    //     println!("✅ Successfully joined space: {}", space_name);
    //     println!("   Your DID: {}", profile_did);
    //     println!("   Membership: {}", membership_did);
    //     println!("   Space: {}", space_did);

    Ok(())
}
