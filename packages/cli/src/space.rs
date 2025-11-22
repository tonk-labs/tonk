use crate::authority;
use crate::crypto::Keypair;
use crate::delegation::{Delegation, DelegationPayload};
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

/// Format time remaining until expiration
fn format_time_remaining(exp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let remaining = exp - now;

    if remaining < 0 {
        return "EXPIRED".to_string();
    }

    if remaining == i64::MAX - now {
        return "never".to_string();
    }

    let days = remaining / 86400;
    let hours = (remaining % 86400) / 3600;
    let mins = (remaining % 3600) / 60;

    if days > 365 {
        "never".to_string()
    } else if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

/// List all spaces accessible by the active authority
pub async fn list() -> Result<()> {
    // Get operator DID
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Get active authority
    let authorities = authority::get_authorities()?;
    if authorities.is_empty() {
        println!("⚠  No active session");
        println!("   Run 'tonk login' to authenticate\n");
        return Ok(());
    }

    let active_authority = crate::session::get_active_authority_from_list(&authorities)?;

    // Get active space from state
    let active_space_did = crate::state::get_active_space(&active_authority.did)?;

    // Collect spaces for active authority
    let spaces = crate::session::collect_spaces_for_authority(&operator_did, &active_authority.did)?;

    if spaces.is_empty() {
        println!("⚠  No accessible spaces\n");
        return Ok(());
    }

    // Sort spaces: auth space first, then alphabetically
    let mut sorted_spaces: Vec<_> = spaces.values().collect();
    sorted_spaces.sort_by(|a, b| match (a.is_auth_space, b.is_auth_space) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.space_did.cmp(&b.space_did),
    });

    // Display each space
    for space in sorted_spaces {
        let is_active = active_space_did.as_ref() == Some(&space.space_did);
        let dim_start = if is_active { "" } else { "\x1b[2m" };
        let dim_end = if is_active { "" } else { "\x1b[0m" };

        // Top separator
        if is_active {
            println!("════════════════════════════════════════════════════════════════");
        } else {
            println!("{}────────────────────────────────────────────────────────────────{}", dim_start, dim_end);
        }

        // Try to load space metadata to get the name
        let space_name = if let Ok(Some(meta)) = crate::metadata::SpaceMetadata::load(&space.space_did) {
            Some(meta.name)
        } else {
            None
        };

        let emoji = if space.is_auth_space { "🔐" } else { "🏠" };
        if space.is_auth_space {
            if let Some(name) = &space_name {
                if name != &space.space_did {
                    println!("{}{} {} ({}) (authorization space){}", dim_start, emoji, name, space.space_did, dim_end);
                } else {
                    println!("{}{} {} (authorization space){}", dim_start, emoji, space.space_did, dim_end);
                }
            } else {
                println!("{}{} {} (authorization space){}", dim_start, emoji, space.space_did, dim_end);
            }
        } else {
            if let Some(name) = &space_name {
                if name != &space.space_did {
                    println!("{}{} {} ({}){}", dim_start, emoji, name, space.space_did, dim_end);
                } else {
                    println!("{}{} {}{}", dim_start, emoji, space.space_did, dim_end);
                }
            } else {
                println!("{}{} {}{}", dim_start, emoji, space.space_did, dim_end);
            }
        }

        // Group and deduplicate commands
        let mut command_exps: HashMap<String, i64> = HashMap::new();
        for (cmd, exp) in &space.commands {
            command_exps
                .entry(cmd.clone())
                .and_modify(|e| *e = (*e).max(*exp))
                .or_insert(*exp);
        }

        let mut commands: Vec<_> = command_exps.into_iter().collect();
        commands.sort_by(|a, b| a.0.cmp(&b.0));

        for (j, (cmd, exp)) in commands.iter().enumerate() {
            let is_last = j == commands.len() - 1;
            let prefix = if is_last { "└─" } else { "├─" };
            let time_str = format_time_remaining(*exp);
            if time_str == "never" {
                println!("{}   {} ⚙️  {}{}", dim_start, prefix, cmd, dim_end);
            } else {
                println!("{}   {} ⚙️  {} (expires: {}){}", dim_start, prefix, cmd, time_str, dim_end);
            }
        }

        // Bottom separator
        if is_active {
            println!("════════════════════════════════════════════════════════════════\n");
        } else {
            println!("{}────────────────────────────────────────────────────────────────{}\n", dim_start, dim_end);
        }
    }

    Ok(())
}

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
        let home: PathBuf = crate::util::home_dir().context("Could not determine home directory")?;

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

    // Save space metadata
    let space_metadata = crate::metadata::SpaceMetadata::new(
        name.clone(),
        owner_dids.clone(),
    );
    space_metadata.save(&space_did)?;

    // Add space to the current session
    crate::state::add_space_to_session(&authority.did, &space_did)?;

    // Set this as the active space for the current session
    crate::state::set_active_space(&authority.did, &space_did)?;

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
    let home = crate::util::home_dir().context("Could not determine home directory")?;
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

/// Invitation file format
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InviteFile {
    /// Secret - hash of the invitation delegation
    secret: String,

    /// Invite code - last 5 characters of signed hash (base58btc)
    code: String,

    /// Authorization chain: [space → authority, authority → operator, operator → membership]
    authorization: Vec<Delegation>,
}

/// Invite a collaborator to a space
pub async fn invite(email: String, space_name: Option<String>) -> Result<()> {
    println!("🎫 Inviting {} to space\n", email);

    // Get operator keypair
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore.get_or_create_keypair()?;
    let operator_did = operator.to_did_key();

    // Get active authority
    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;

    // Get the space to invite to
    let space_did = if let Some(name) = &space_name {
        // Look up space by name from active session's spaces
        let spaces = crate::state::list_spaces_for_session(&authority.did)?;

        // Try to find space by looking up metadata
        let mut found_did = None;
        for space_did in &spaces {
            if let Ok(Some(meta)) = crate::metadata::SpaceMetadata::load(space_did) {
                if &meta.name == name {
                    found_did = Some(space_did.clone());
                    break;
                }
            }
        }

        found_did.context(format!("Space '{}' not found", name))?
    } else if let Some(active_id) = crate::state::get_active_space(&authority.did)? {
        active_id
    } else {
        anyhow::bail!("No active space. Create one with 'tonk space create' or specify --space");
    };

    // Load space metadata to get the name
    let space_meta = crate::metadata::SpaceMetadata::load(&space_did)?
        .context("Space metadata not found")?;

    println!("📍 Space: {} ({})\n", space_meta.name, space_did);

    // Convert email to did:mailto
    let invitee_did = format!("did:mailto:{}", email);
    println!("📧 Invitee: {}\n", invitee_did);

    // Step 1: Create delegation from operator → did:mailto (invitation)
    println!("1️⃣  Creating invitation delegation...");
    let invitation_payload = DelegationPayload {
        iss: operator_did.clone(),
        aud: invitee_did.clone(),
        cmd: "/".to_string(),
        sub: Some(space_did.clone()),
        exp: i64::MAX, // Never expires
        pol: Vec::new(),
    };

    let invitation_json = serde_json::to_string(&invitation_payload)?;
    let invitation_signature = operator.sign(invitation_json.as_bytes());
    let invitation = Delegation {
        payload: invitation_payload,
        signature: STANDARD.encode(invitation_signature.to_bytes()),
    };

    // Step 2: Store invitation under ~/.tonk/access/did:mailto:.../did:key:space/...
    println!("2️⃣  Storing invitation...");
    let invitation_hash = invitation.hash();
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let invitation_dir = home
        .join(".tonk")
        .join("access")
        .join(&invitee_did)
        .join(&space_did);
    fs::create_dir_all(&invitation_dir)?;

    let invitation_path = invitation_dir.join(format!("{}-{}.json", invitation.payload.exp, invitation_hash));
    fs::write(&invitation_path, serde_json::to_string_pretty(&invitation)?)?;
    println!("   ✓ Saved invitation");

    // Step 3: Generate invite code - sign hash with operator key, take last 5 chars in base58btc
    println!("3️⃣  Generating invite code...");
    let hash_signature = operator.sign(invitation_hash.as_bytes());
    let hash_sig_bytes = hash_signature.to_bytes();
    let hash_sig_b58 = bs58::encode(&hash_sig_bytes).into_string();
    let invite_code = hash_sig_b58.chars().rev().take(5).collect::<String>()
        .chars().rev().collect::<String>(); // Take last 5 chars
    println!("   ✓ Code: {}", invite_code);

    // Step 4: Derive membership keypair using HKDF(hash + code)
    println!("4️⃣  Deriving membership principal...");
    use hkdf::Hkdf;
    use sha2::Sha256;

    let ikm = format!("{}{}", invitation_hash, invite_code);
    let hk = Hkdf::<Sha256>::new(None, ikm.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"tonk-membership-v1", &mut okm)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {:?}", e))?;

    let membership_keypair = Keypair::from_bytes(&okm);
    let membership_did = membership_keypair.to_did_key();
    println!("   ✓ Membership: {}", membership_did);

    // Step 5: Create delegation from operator → membership
    println!("5️⃣  Creating operator → membership delegation...");
    let membership_payload = DelegationPayload {
        iss: operator_did.clone(),
        aud: membership_did.clone(),
        cmd: "/".to_string(),
        sub: Some(space_did.clone()),
        exp: i64::MAX,
        pol: Vec::new(),
    };

    let membership_json = serde_json::to_string(&membership_payload)?;
    let membership_signature = operator.sign(membership_json.as_bytes());
    let operator_to_membership = Delegation {
        payload: membership_payload,
        signature: STANDARD.encode(membership_signature.to_bytes()),
    };

    // Step 6: Build delegation chain - we need space → authority and authority → operator
    println!("6️⃣  Building delegation chain...");

    // Find space → authority delegation
    let space_to_authority = find_delegation(&space_did, &authority.did)?
        .context("Space → authority delegation not found")?;

    // Find authority → operator delegation
    let authority_to_operator = find_delegation(&authority.did, &operator_did)?
        .context("Authority → operator delegation not found")?;

    let authorization_chain = vec![
        space_to_authority,
        authority_to_operator,
        operator_to_membership,
    ];
    println!("   ✓ Chain: space → authority → operator → membership");

    // Step 7: Create invite file
    println!("7️⃣  Creating invite file...");
    let invite_file = InviteFile {
        secret: invitation_hash,
        code: invite_code,
        authorization: authorization_chain,
    };

    let invite_filename = format!("{}.invite", space_meta.name.replace(" ", "_"));
    let invite_json = serde_json::to_string_pretty(&invite_file)?;
    fs::write(&invite_filename, invite_json)?;

    println!("\n✅ Invitation created!");
    println!("   File: {}", invite_filename);
    println!("   Share this file with {} to grant them access\n", email);

    Ok(())
}

/// Find a delegation from issuer to audience for a specific subject
fn find_delegation(issuer: &str, audience: &str) -> Result<Option<Delegation>> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let access_dir = home.join(".tonk").join("access").join(audience).join(issuer);

    if !access_dir.exists() {
        return Ok(None);
    }

    // Find the most recent valid delegation
    let mut delegations = Vec::new();
    for entry in fs::read_dir(&access_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        if let Ok(json) = fs::read_to_string(&path) {
            if let Ok(delegation) = serde_json::from_str::<Delegation>(&json) {
                if delegation.is_valid() {
                    delegations.push(delegation);
                }
            }
        }
    }

    // Return the one with the latest expiration
    delegations.sort_by(|a, b| b.payload.exp.cmp(&a.payload.exp));
    Ok(delegations.into_iter().next())
}

/// Join a space using an invitation file
pub async fn join(invite_path: String, _profile_name: Option<String>) -> Result<()> {
    println!("🔗 Joining space with invitation\n");

    // Step 1: Read and parse invite file
    println!("1️⃣  Reading invite file...");
    let invite_data = fs::read_to_string(&invite_path)
        .context("Failed to read invite file")?;
    let invite: InviteFile = serde_json::from_str(&invite_data)
        .context("Failed to parse invite file")?;
    println!("   ✓ Loaded invitation");

    // Extract space DID from authorization chain
    let space_did = invite.authorization.first()
        .context("Authorization chain is empty")?
        .payload.iss.clone();

    // Extract operator → membership delegation (last in chain)
    let operator_to_membership = invite.authorization.last()
        .context("Authorization chain is empty")?;

    println!("   Space DID: {}", space_did);

    // Step 2: Reconstruct membership keypair
    println!("2️⃣  Reconstructing membership keypair...");
    use hkdf::Hkdf;
    use sha2::Sha256;

    let ikm = format!("{}{}", invite.secret, invite.code);
    let hk = Hkdf::<Sha256>::new(None, ikm.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"tonk-membership-v1", &mut okm)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {:?}", e))?;

    let membership_keypair = Keypair::from_bytes(&okm);
    let reconstructed_membership_did = membership_keypair.to_did_key();
    println!("   ✓ Reconstructed: {}", reconstructed_membership_did);

    // Step 3: Verify membership DID matches authorization chain
    println!("3️⃣  Verifying membership principal...");
    let expected_membership_did = &operator_to_membership.payload.aud;

    if &reconstructed_membership_did != expected_membership_did {
        anyhow::bail!(
            "Membership verification failed!\n   Expected: {}\n   Got: {}",
            expected_membership_did,
            reconstructed_membership_did
        );
    }
    println!("   ✓ Membership verified!");

    // Step 4: Get active authority to delegate to
    println!("4️⃣  Getting active authority...");
    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;
    println!("   ✓ Authority: {}", authority.did);

    // Step 5: Create membership → authority delegation
    println!("5️⃣  Creating membership → authority delegation...");
    let membership_to_authority_payload = DelegationPayload {
        iss: reconstructed_membership_did.clone(),
        aud: authority.did.clone(),
        cmd: "/".to_string(),
        sub: Some(space_did.clone()),
        exp: i64::MAX,
        pol: Vec::new(),
    };

    let membership_json = serde_json::to_string(&membership_to_authority_payload)?;
    let membership_signature = membership_keypair.sign(membership_json.as_bytes());
    let membership_to_authority = Delegation {
        payload: membership_to_authority_payload,
        signature: STANDARD.encode(membership_signature.to_bytes()),
    };
    println!("   ✓ Delegation created");

    // Step 6: Import all delegations to local access directory
    println!("6️⃣  Importing delegation chain...");
    let home = crate::util::home_dir().context("Could not determine home directory")?;

    // Import each delegation in the authorization chain
    for (i, delegation) in invite.authorization.iter().enumerate() {
        let aud = &delegation.payload.aud;
        let iss = &delegation.payload.iss;

        let access_dir = home.join(".tonk").join("access").join(aud).join(iss);
        fs::create_dir_all(&access_dir)?;

        let hash = delegation.hash();
        let filename = format!("{}-{}.json", delegation.payload.exp, hash);
        let delegation_path = access_dir.join(filename);

        fs::write(&delegation_path, serde_json::to_string_pretty(delegation)?)?;
        println!("   ✓ Imported delegation {} of {}", i + 1, invite.authorization.len());
    }

    // Import the membership → authority delegation
    let membership_access_dir = home
        .join(".tonk")
        .join("access")
        .join(&authority.did)
        .join(&reconstructed_membership_did);
    fs::create_dir_all(&membership_access_dir)?;

    let membership_hash = membership_to_authority.hash();
    let membership_filename = format!("{}-{}.json", membership_to_authority.payload.exp, membership_hash);
    let membership_path = membership_access_dir.join(membership_filename);
    fs::write(&membership_path, serde_json::to_string_pretty(&membership_to_authority)?)?;
    println!("   ✓ Imported membership → authority delegation");

    // Step 7: Add space to active session
    println!("7️⃣  Adding space to session...");
    crate::state::add_space_to_session(&authority.did, &space_did)?;
    crate::state::set_active_space(&authority.did, &space_did)?;
    println!("   ✓ Space added to session");

    // Load or create space metadata
    if let Ok(Some(meta)) = crate::metadata::SpaceMetadata::load(&space_did) {
        println!("\n✅ Successfully joined space!");
        println!("   Space: {}", meta.name);
        println!("   DID: {}", space_did);
        println!("   Membership: {}", reconstructed_membership_did);
        println!("   Authority: {}\n", authority.did);
    } else {
        println!("\n✅ Successfully joined space!");
        println!("   DID: {}", space_did);
        println!("   Membership: {}", reconstructed_membership_did);
        println!("   Authority: {}\n", authority.did);
    }

    Ok(())
}
