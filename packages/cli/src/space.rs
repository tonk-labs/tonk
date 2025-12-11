use crate::authority;
use crate::crypto::Keypair;
use crate::delegation::{Delegation, keypair_to_signer};
use crate::keystore::Keystore;
use crate::session::collect_spaces_for_authority;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use tonk_space::{DelegationClaim, Issuer, Space};
use ucan::did::{Did, Ed25519Did, Ed25519Signer};
use ucan::{Delegation as UcanDelegation, delegation::subject::DelegatedSubject};

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
    let spaces =
        crate::session::collect_spaces_for_authority(&operator_did, &active_authority.did)?;

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
            println!(
                "{}────────────────────────────────────────────────────────────────{}",
                dim_start, dim_end
            );
        }

        // Try to load space metadata to get the name
        let space_name =
            if let Ok(Some(meta)) = crate::metadata::SpaceMetadata::load(&space.space_did) {
                Some(meta.name)
            } else {
                None
            };

        let emoji = if space.is_auth_space { "🔐" } else { "🏠" };
        if space.is_auth_space {
            if let Some(name) = &space_name {
                if name != &space.space_did {
                    println!(
                        "{}{} {} ({}) (authorization space){}",
                        dim_start, emoji, name, space.space_did, dim_end
                    );
                } else {
                    println!(
                        "{}{} {} (authorization space){}",
                        dim_start, emoji, space.space_did, dim_end
                    );
                }
            } else {
                println!(
                    "{}{} {} (authorization space){}",
                    dim_start, emoji, space.space_did, dim_end
                );
            }
        } else if let Some(name) = &space_name {
            if name != &space.space_did {
                println!(
                    "{}{} {} ({}){}",
                    dim_start, emoji, name, space.space_did, dim_end
                );
            } else {
                println!("{}{} {}{}", dim_start, emoji, space.space_did, dim_end);
            }
        } else {
            println!("{}{} {}{}", dim_start, emoji, space.space_did, dim_end);
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
                println!(
                    "{}   {} ⚙️  {} (expires: {}){}",
                    dim_start, prefix, cmd, time_str, dim_end
                );
            }
        }

        // Bottom separator
        if is_active {
            println!("════════════════════════════════════════════════════════════════\n");
        } else {
            println!(
                "{}────────────────────────────────────────────────────────────────{}\n",
                dim_start, dim_end
            );
        }
    }

    Ok(())
}

/// Show the current space DID
pub async fn show_current() -> Result<()> {
    // Get active authority
    let authority = crate::authority::get_active_authority()?
        .context("No active session. Please run `tonk login` first.")?;

    // Get active space
    let active_space = crate::state::get_active_space(&authority.did)?;

    match active_space {
        Some(space_did) => {
            // Try to load space metadata to show name
            if let Ok(Some(metadata)) = crate::metadata::SpaceMetadata::load(&space_did) {
                println!("🏠 {} ({})", metadata.name, space_did);
            } else {
                println!("🏠 {}", space_did);
            }
        }
        None => {
            println!("No active space set for current session.");
            println!("Use `tonk space set <name-or-did>` to select a space.");
        }
    }

    Ok(())
}

/// Switch to a different space (by name or DID)
pub async fn set(space_identifier: String) -> Result<()> {
    // Get operator and authority
    let keystore = crate::keystore::Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    let authority = crate::authority::get_active_authority()?
        .context("No active session. Please run `tonk login` first.")?;

    // Collect available spaces
    let spaces = collect_spaces_for_authority(&operator_did, &authority.did)?;

    // Find space by name or DID
    let space_did = if space_identifier.starts_with("did:key:") {
        // Direct DID lookup
        if spaces.contains_key(&space_identifier) {
            space_identifier
        } else {
            anyhow::bail!("Space {} not found or not accessible", space_identifier);
        }
    } else {
        // Name lookup
        let matching_spaces: Vec<String> = spaces
            .keys()
            .filter(|space_did| {
                if let Ok(Some(metadata)) = crate::metadata::SpaceMetadata::load(space_did) {
                    metadata.name == space_identifier
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        if matching_spaces.is_empty() {
            anyhow::bail!("No space found with name '{}'", space_identifier);
        } else if matching_spaces.len() > 1 {
            anyhow::bail!(
                "Multiple spaces found with name '{}'. Use the DID instead.",
                space_identifier
            );
        } else {
            matching_spaces[0].clone()
        }
    };

    // Set as active space
    crate::state::set_active_space(&authority.did, &space_did)?;

    // Show confirmation
    if let Ok(Some(metadata)) = crate::metadata::SpaceMetadata::load(&space_did) {
        println!("✅ Switched to space: {} ({})", metadata.name, space_did);
    } else {
        println!("✅ Switched to space: {}", space_did);
    }

    Ok(())
}

/// Create a new space
pub async fn create(
    name: String,
    owners: Option<Vec<String>>,
    _description: Option<String>,
) -> Result<()> {
    println!("🚀 Creating space: {}\n", name);

    // Get operator keypair
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Get active authority (required for space creation)
    let authority = authority::get_active_authority()?
        .context("No active authority. Please run 'tonk login' first")?;

    println!(
        "👤 Authority: {}\n",
        authority::format_authority_did(&authority.did)
    );

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

    // Generate space keypair (temporary - used only for signing owner delegations)
    let space_keypair = Keypair::generate();
    let space_did = space_keypair.to_did_key();

    println!("🏠 Space DID: {}\n", space_did);

    // Create delegations from space to each owner and collect for storage
    println!("📜 Creating owner delegations...");
    let mut delegation_claims: Vec<DelegationClaim> = Vec::new();
    for owner_did in &owner_dids {
        let delegation = create_owner_delegation(&space_keypair, &space_did, owner_did)?;

        // Convert to DelegationClaim for Space storage
        let claim: DelegationClaim = (&delegation)
            .try_into()
            .context("Failed to convert delegation to claim")?;
        delegation_claims.push(claim);

        // Also save to filesystem (existing behavior)
        delegation.save()?;
        println!("   ✓ {}", authority::format_authority_did(owner_did));
    }

    // Save space metadata
    let space_metadata = crate::metadata::SpaceMetadata::new(name.clone(), owner_dids.clone());
    space_metadata.save(&space_did)?;

    // Add space to the current session (creates the space directory)
    crate::state::add_space_to_session(&authority.did, &space_did)?;

    // Create Space with dialog-db storage
    println!("📦 Storing delegations in space database...");
    let storage_path = get_space_storage_path(&operator_did, &authority.did, &space_did)?;
    let issuer = Issuer::from_secret(&operator.to_bytes());

    Space::create(space_did.clone(), issuer, storage_path, delegation_claims)
        .await
        .context("Failed to create space database")?;
    println!("   ✓ Delegations stored in main branch");

    // Set this as the active space for the current session
    crate::state::set_active_space(&authority.did, &space_did)?;

    println!("✅ Space created and set as active!");
    println!("   Operators under these authorities now have access to the space.\n");

    Ok(())
}

/// Get the storage path for a space's facts database
fn get_space_storage_path(operator_did: &str, authority_did: &str, space_did: &str) -> Result<PathBuf> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let path = home
        .join(".tonk")
        .join("operator")
        .join(operator_did)
        .join("session")
        .join(authority_did)
        .join("space")
        .join(space_did)
        .join("facts");
    Ok(path)
}

/// Create a delegation from space to an owner
fn create_owner_delegation(
    space_keypair: &Keypair,
    space_did: &str,
    owner_did: &str,
) -> Result<Delegation> {
    // Parse owner DID
    let owner_did_parsed: Ed25519Did = owner_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse owner DID: {:?}", e))?;

    // Parse space DID for subject
    let space_did_parsed: Ed25519Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;

    // Create delegation using ucan builder: Space → Owner with full access
    let issuer_signer = keypair_to_signer(space_keypair);

    let ucan_delegation: UcanDelegation<Ed25519Did> = UcanDelegation::builder()
        .issuer(issuer_signer)
        .audience(owner_did_parsed)
        .subject(DelegatedSubject::Specific(space_did_parsed))
        .command(vec!["/".to_string()]) // Full access
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build delegation: {}", e))?;

    // Wrap in our Delegation type and return
    Ok(Delegation::from_ucan(ucan_delegation))
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
    let space_meta =
        crate::metadata::SpaceMetadata::load(&space_did)?.context("Space metadata not found")?;

    println!("📍 Space: {} ({})\n", space_meta.name, space_did);

    // Convert email to did:mailto
    let invitee_did = format!("did:mailto:{}", email);
    println!("📧 Invitee: {}\n", invitee_did);

    // Step 1: Create invitation delegation (operator → did:mailto)
    println!("1️⃣  Creating invitation delegation...");

    // Parse DIDs as UniversalDid
    use crate::did::UniversalDid;
    let operator_did_universal: UniversalDid = operator_did.parse()?;
    let invitee_did_universal: UniversalDid = invitee_did.parse()?;
    let space_did_universal: UniversalDid = space_did.parse()?;

    // Create a signer from the operator keypair
    // We need to create a custom signer that works with UniversalDid
    use ucan::did::DidSigner;

    #[derive(Serialize)]
    struct UniversalDidSigner {
        did: UniversalDid,
        signer: Ed25519Signer,
    }

    impl DidSigner for UniversalDidSigner {
        type Did = UniversalDid;

        fn did(&self) -> &Self::Did {
            &self.did
        }

        fn signer(&self) -> &<<Self::Did as Did>::VarsigConfig as varsig::signer::Sign>::Signer {
            self.signer.signer()
        }
    }

    let operator_signer_inner = keypair_to_signer(&operator);
    let operator_universal_signer = UniversalDidSigner {
        did: operator_did_universal.clone(),
        signer: operator_signer_inner,
    };

    // Build invitation delegation using ucan
    let invitation_ucan: UcanDelegation<UniversalDid> = UcanDelegation::builder()
        .issuer(operator_universal_signer)
        .audience(invitee_did_universal)
        .subject(DelegatedSubject::Specific(space_did_universal))
        .command(vec!["/".to_string()])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build invitation: {}", e))?;

    // Step 2: Serialize and store invitation
    println!("2️⃣  Storing invitation...");

    // Serialize to CBOR
    let invitation_cbor = serde_ipld_dagcbor::to_vec(&invitation_ucan)
        .map_err(|e| anyhow::anyhow!("Failed to serialize invitation: {}", e))?;

    // Compute hash
    let mut hasher = sha2::Sha256::new();
    hasher.update(&invitation_cbor);
    let invitation_hash_bytes = hasher.finalize();
    let invitation_hash = hex::encode(invitation_hash_bytes);

    // Save to storage (in the operator's access directory for the invitee)
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let access_dir = home
        .join(".tonk")
        .join("access")
        .join(&invitee_did)
        .join(&operator_did);
    fs::create_dir_all(&access_dir)?;

    let cbor_path = access_dir.join(format!("{}.cbor", invitation_hash));
    fs::write(&cbor_path, &invitation_cbor)?;

    println!("   ✓ Saved invitation");

    // Step 3: Generate invite code - sign hash with operator key, take last 5 chars in base58btc
    println!("3️⃣  Generating invite code...");
    let hash_signature = operator.sign(invitation_hash.as_bytes());
    let hash_sig_bytes = hash_signature.to_bytes();
    let hash_sig_b58 = bs58::encode(&hash_sig_bytes).into_string();
    let invite_code = hash_sig_b58
        .chars()
        .rev()
        .take(5)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>(); // Take last 5 chars
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

    // Parse DIDs
    let space_did_parsed: Ed25519Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;
    let membership_did_parsed: Ed25519Did = membership_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse membership DID: {:?}", e))?;

    // Build delegation using ucan
    let operator_signer2 = keypair_to_signer(&operator);
    let membership_ucan: UcanDelegation<Ed25519Did> = UcanDelegation::builder()
        .issuer(operator_signer2)
        .audience(membership_did_parsed)
        .subject(DelegatedSubject::Specific(space_did_parsed))
        .command(vec!["/".to_string()])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build membership delegation: {}", e))?;

    let operator_to_membership = Delegation::from_ucan(membership_ucan);

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

    // Step 7: Create invite file (use CBOR encoding)
    println!("7️⃣  Creating invite file...");
    let invite_file = InviteFile {
        secret: invitation_hash,
        code: invite_code,
        authorization: authorization_chain,
    };

    let invite_filename = format!("{}.invite", space_meta.name.replace(" ", "_"));
    let invite_cbor = serde_ipld_dagcbor::to_vec(&invite_file)?;
    fs::write(&invite_filename, invite_cbor)?;

    println!("\n✅ Invitation created!");
    println!("   File: {}", invite_filename);
    println!("   Share this file with {} to grant them access\n", email);

    Ok(())
}

/// Inspect an invite file and show its contents
pub fn inspect_invite(path: String) -> Result<()> {
    println!("🔍 Inspecting invite file...\n");

    // Read and parse invite file (CBOR format)
    println!("📂 Reading file: {}", path);
    let invite_data = fs::read(&path).context("Failed to read invite file")?;

    let invite: InviteFile = serde_ipld_dagcbor::from_slice(&invite_data)
        .context("Failed to parse invite file (expected CBOR format)")?;

    println!("   ✓ Loaded {} bytes\n", invite_data.len());

    // Show invite details
    println!("📋 Invite Details:");
    println!("   Secret (hash): {}", invite.secret);
    println!("   Invite code:   {}", invite.code);
    println!(
        "   Chain length:  {} delegations\n",
        invite.authorization.len()
    );

    // Show each delegation in the authorization chain
    println!("🔗 Authorization Chain:");
    for (i, delegation) in invite.authorization.iter().enumerate() {
        println!(
            "\n   {}. Delegation {} → {}",
            i + 1,
            delegation.issuer(),
            delegation.audience()
        );
        println!(
            "      Subject:  {}",
            match delegation.subject() {
                DelegatedSubject::Specific(did) => did.to_string(),
                DelegatedSubject::Any => "*".to_string(),
            }
        );
        println!("      Command:  {}", delegation.command().join(", "));
        println!("      Valid:    {}", delegation.is_valid());

        if let Some(exp) = delegation.expiration() {
            let exp_date = chrono::DateTime::from_timestamp(exp, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| format!("Invalid timestamp: {}", exp));
            println!("      Expires:  {}", exp_date);
        } else {
            println!("      Expires:  never");
        }
    }

    // Extract space DID from first delegation
    if let Some(first) = invite.authorization.first() {
        println!("\n📍 Space DID: {}", first.issuer());
    }

    // Extract membership DID from last delegation
    if let Some(last) = invite.authorization.last() {
        println!("🎟️  Membership DID: {}", last.audience());
    }

    println!("\n✅ Invite file is valid!\n");

    Ok(())
}

/// Find a delegation from issuer to audience for a specific subject
fn find_delegation(issuer: &str, audience: &str) -> Result<Option<Delegation>> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let access_dir = home
        .join(".tonk")
        .join("access")
        .join(audience)
        .join(issuer);

    if !access_dir.exists() {
        return Ok(None);
    }

    // Find the most recent valid delegation
    let mut delegations = Vec::new();
    for entry in fs::read_dir(&access_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("cbor") {
            continue;
        }

        if let Ok(cbor_bytes) = fs::read(&path) {
            if let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes) {
                if delegation.is_valid() {
                    delegations.push(delegation);
                }
            }
        }
    }

    // Return the one with the latest expiration
    delegations.sort_by(|a, b| {
        let exp_a = a.expiration().unwrap_or(0);
        let exp_b = b.expiration().unwrap_or(0);
        exp_b.cmp(&exp_a)
    });
    Ok(delegations.into_iter().next())
}

/// Join a space using an invitation file
pub async fn join(invite_path: String, _profile_name: Option<String>) -> Result<()> {
    println!("🔗 Joining space with invitation\n");

    // Step 1: Read and parse invite file (CBOR format)
    println!("1️⃣  Reading invite file...");
    let invite_data = fs::read(&invite_path).context("Failed to read invite file")?;
    let invite: InviteFile = serde_ipld_dagcbor::from_slice(&invite_data)
        .context("Failed to parse invite file (expected CBOR format)")?;
    println!("   ✓ Loaded invitation");

    // Extract space DID from authorization chain
    let space_did = invite
        .authorization
        .first()
        .context("Authorization chain is empty")?
        .issuer();

    // Extract operator → membership delegation (last in chain)
    let operator_to_membership = invite
        .authorization
        .last()
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
    let expected_membership_did = operator_to_membership.audience();

    if reconstructed_membership_did != expected_membership_did {
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

    // Parse DIDs
    let authority_did_parsed: Ed25519Did = authority
        .did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse authority DID: {:?}", e))?;
    let space_did_parsed: Ed25519Did = space_did
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse space DID: {:?}", e))?;

    // Build delegation using ucan
    let membership_signer = keypair_to_signer(&membership_keypair);
    let membership_to_authority_ucan: UcanDelegation<Ed25519Did> = UcanDelegation::builder()
        .issuer(membership_signer)
        .audience(authority_did_parsed)
        .subject(DelegatedSubject::Specific(space_did_parsed))
        .command(vec!["/".to_string()])
        .try_build()
        .map_err(|e| anyhow::anyhow!("Failed to build membership delegation: {}", e))?;

    let membership_to_authority = Delegation::from_ucan(membership_to_authority_ucan);
    println!("   ✓ Delegation created");

    // Note: We don't create authority → operator here because:
    // 1. That delegation already exists from the login flow
    // 2. We don't have the authority's keypair (it's derived from passkey in browser)
    // The membership → authority delegation connects to the existing auth chain

    // Get the local operator DID for verification
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Step 6: Import all delegations to local access directory
    println!("6️⃣  Importing delegation chain...");

    // Import each delegation in the authorization chain
    for (i, delegation) in invite.authorization.iter().enumerate() {
        delegation.save()?;
        println!(
            "   ✓ Imported delegation {} of {}",
            i + 1,
            invite.authorization.len()
        );
    }

    // Import the membership → authority delegation
    membership_to_authority.save()?;
    println!("   ✓ Imported membership → authority delegation");

    // Step 7: Verify the authority → operator delegation exists
    println!("7️⃣  Verifying authority → operator delegation...");
    let authority_to_operator_exists = find_delegation(&authority.did, &operator_did)?;

    if authority_to_operator_exists.is_none() {
        anyhow::bail!(
            "Missing authority → operator delegation!\n   Authority: {}\n   Operator: {}\n   This should have been created during login.",
            authority.did,
            operator_did
        );
    }
    println!("   ✓ Authority → operator delegation exists");

    // Step 8: Add space to active session
    println!("8️⃣  Adding space to session...");
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

/// Delete a space
pub async fn delete(space_identifier: String, force: bool) -> Result<()> {
    // Get operator and authority
    let keystore = crate::keystore::Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    let authority = crate::authority::get_active_authority()?
        .context("No active session. Please run `tonk login` first.")?;

    // Collect available spaces
    let spaces = collect_spaces_for_authority(&operator_did, &authority.did)?;

    // Find space by name or DID
    let space_did = if space_identifier.starts_with("did:key:") {
        // Direct DID lookup
        if spaces.contains_key(&space_identifier) {
            space_identifier.clone()
        } else {
            anyhow::bail!("Space {} not found or not accessible", space_identifier);
        }
    } else {
        // Name lookup
        let matching_spaces: Vec<String> = spaces
            .keys()
            .filter(|space_did| {
                if let Ok(Some(metadata)) = crate::metadata::SpaceMetadata::load(space_did) {
                    metadata.name == space_identifier
                } else {
                    false
                }
            })
            .cloned()
            .collect();

        if matching_spaces.is_empty() {
            anyhow::bail!("No space found with name '{}'", space_identifier);
        } else if matching_spaces.len() > 1 {
            anyhow::bail!(
                "Multiple spaces found with name '{}'. Use the DID instead.",
                space_identifier
            );
        } else {
            matching_spaces[0].clone()
        }
    };

    // Get space name for display
    let space_name = if let Ok(Some(metadata)) = crate::metadata::SpaceMetadata::load(&space_did) {
        metadata.name
    } else {
        space_did.clone()
    };

    // Confirm deletion unless --force
    if !force {
        println!("⚠️  About to delete space: {} ({})", space_name, space_did);
        println!("   This will remove the space and all its data from this session.\n");

        use dialoguer::Confirm;
        let confirmed = Confirm::new()
            .with_prompt("Are you sure you want to delete this space?")
            .default(false)
            .interact()?;

        if !confirmed {
            println!("Cancelled.");
            return Ok(());
        }
    }

    println!("🗑️  Deleting space: {}\n", space_name);

    // Remove space from session (this also clears active space if needed)
    crate::state::remove_space_from_session(&authority.did, &space_did)?;
    println!("   ✓ Removed from session");

    // Delete space metadata
    if crate::metadata::SpaceMetadata::delete(&space_did).is_ok() {
        println!("   ✓ Deleted metadata");
    }

    // Delete delegations from this space (in access directory)
    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let access_dir = home.join(".tonk").join("access");

    if access_dir.exists() {
        let mut deleted_delegations = 0;
        // Look through all audience directories
        for audience_entry in fs::read_dir(&access_dir)? {
            let audience_entry = audience_entry?;
            let audience_path = audience_entry.path();

            if !audience_path.is_dir() {
                continue;
            }

            // Look for directories named with the space_did (delegations from the space)
            let space_delegation_dir = audience_path.join(&space_did);
            if space_delegation_dir.exists() && space_delegation_dir.is_dir() {
                fs::remove_dir_all(&space_delegation_dir)?;
                deleted_delegations += 1;
            }
        }

        if deleted_delegations > 0 {
            println!("   ✓ Deleted {} delegation chain(s)", deleted_delegations);
        }
    }

    println!("\n✅ Space deleted: {}", space_name);

    Ok(())
}
