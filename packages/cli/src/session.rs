use crate::authority;
use crate::delegation::Delegation;
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use ucan::delegation::subject::DelegatedSubject;

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

/// Represents a space with its accessible commands
#[derive(Debug, Clone)]
pub struct SpaceAccess {
    pub space_did: String,
    pub commands: Vec<(String, i64)>, // (command, expiration)
    pub is_auth_space: bool, // True if this is the authority's own DID (authorization space)
    pub authorization_chains: HashMap<String, Vec<Delegation>>, // (command -> delegation chain to operator)
}

/// Get the active authority from the list based on state
pub fn get_active_authority_from_list(
    authorities: &[authority::Authority],
) -> Result<authority::Authority> {
    let active_did = crate::state::get_active_session()?;

    if let Some(active_did) = active_did {
        // Try to find the active authority
        if let Some(auth) = authorities.iter().find(|a| a.did == active_did) {
            return Ok(auth.clone());
        }
    }

    // Default to first authority if no active one is set or not found
    Ok(authorities[0].clone())
}

/// Show the current session DID
pub async fn show_current() -> Result<()> {
    let authorities = authority::get_authorities()?;

    if authorities.is_empty() {
        println!("🚏 No active sessions found");
        println!("👤 Run 'tonk login' to create an authorization session\n");
        return Ok(());
    }

    let active_authority = get_active_authority_from_list(&authorities)?;
    println!("{}", active_authority.did);

    Ok(())
}

/// Switch to a different session
pub async fn set(authority_did: String) -> Result<()> {
    let authorities = authority::get_authorities()?;

    // Find the authority in the list
    if !authorities.iter().any(|a| a.did == authority_did) {
        anyhow::bail!(
            "Authority not found: {}\n   Run 'tonk session' to see available sessions",
            authority_did
        );
    }

    // Set active session
    crate::state::set_active_session(&authority_did)?;

    println!("✅ Switched to session: {}", authority_did);

    // Restore active space for this session (if any)
    if let Some(active_space) = crate::state::get_active_space(&authority_did)? {
        println!("   Active space: {}", active_space);
    }

    Ok(())
}

/// List all available sessions
pub async fn list(verbose: bool) -> Result<()> {
    // Get operator DID
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Get all available authorities/sessions
    let authorities = authority::get_authorities()?;

    if authorities.is_empty() {
        println!("🫆 Operator: {}\n", operator_did);
        println!("🚏 No active sessions found");
        println!("👤 Run 'tonk login' to create an authorization session\n");
        return Ok(());
    }

    // Get active authority
    let active_authority = get_active_authority_from_list(&authorities)?;

    // Get home directory for chain building
    let home = crate::util::home_dir().context("Could not determine home directory")?;

    // Sort authorities to show active one first
    let mut sorted_authorities = authorities.clone();
    sorted_authorities.sort_by(|a, b| {
        if a.did == active_authority.did {
            std::cmp::Ordering::Less
        } else if b.did == active_authority.did {
            std::cmp::Ordering::Greater
        } else {
            a.did.cmp(&b.did)
        }
    });

    // Display each session as a separate block
    for auth in &sorted_authorities {
        let is_active = auth.did == active_authority.did;
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

        // Authority and Operator
        println!("{}👤 Authority: {}{}", dim_start, auth.did, dim_end);
        println!("{}🫆 Operator:  {}{}", dim_start, operator_did, dim_end);

        // Collect spaces for this authority
        let spaces = collect_spaces_for_authority(&operator_did, &auth.did)?;

        if spaces.is_empty() {
            println!("{}🪪  Access: none{}", dim_start, dim_end);
        } else {
            println!("{}🪪  Access:{}", dim_start, dim_end);

            // Sort spaces: auth space first, then alphabetically
            let mut sorted_spaces: Vec<_> = spaces.values().collect();
            sorted_spaces.sort_by(|a, b| match (a.is_auth_space, b.is_auth_space) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.space_did.cmp(&b.space_did),
            });

            for space in sorted_spaces {
                // Try to load space metadata to get the name
                let space_name = if let Ok(Some(meta)) =
                    crate::metadata::SpaceMetadata::load(&space.space_did)
                {
                    Some(meta.name)
                } else {
                    None
                };

                let emoji = if space.is_auth_space { "🔐" } else { "🏠" };
                if space.is_auth_space {
                    if let Some(name) = &space_name {
                        if name != &space.space_did {
                            println!(
                                "{}   {} {} ({}) (authorization space){}",
                                dim_start, emoji, name, space.space_did, dim_end
                            );
                        } else {
                            println!(
                                "{}   {} {} (authorization space){}",
                                dim_start, emoji, space.space_did, dim_end
                            );
                        }
                    } else {
                        println!(
                            "{}   {} {} (authorization space){}",
                            dim_start, emoji, space.space_did, dim_end
                        );
                    }
                } else if let Some(name) = &space_name {
                    if name != &space.space_did {
                        println!(
                            "{}   {} {} ({}){}",
                            dim_start, emoji, name, space.space_did, dim_end
                        );
                    } else {
                        println!("{}   {} {}{}", dim_start, emoji, space.space_did, dim_end);
                    }
                } else {
                    println!("{}   {} {}{}", dim_start, emoji, space.space_did, dim_end);
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
                        println!("{}      {} ⚙️  {}{}", dim_start, prefix, cmd, dim_end);
                    } else {
                        println!(
                            "{}      {} ⚙️  {} (expires: {}){}",
                            dim_start, prefix, cmd, time_str, dim_end
                        );
                    }

                    // Show delegation chain in verbose mode
                    if verbose {
                        // Build the complete authorization chain for this command
                        if let Ok(chain) = build_authorization_chain(
                            &space.space_did,
                            cmd,
                            &operator_did,
                            &auth.did,
                            &home,
                        ) {
                            // Chain is built: [authority→operator, ..., space→authority]
                            // Display from operator (top) to space (bottom/root)
                            let mut shown_dids = HashSet::new();
                            let mut chain_dids = Vec::new();

                            // Collect all DIDs in the chain in order: operator → ... → space
                            // Start with operator (audience of first delegation)
                            if !chain.is_empty() {
                                let first_del = &chain[0];
                                let aud = first_del.audience();
                                if !shown_dids.contains(&aud) {
                                    chain_dids.push((aud.clone(), first_del.is_powerline()));
                                    shown_dids.insert(aud);
                                }
                            }

                            // Add each issuer in the chain
                            for delegation in chain.iter() {
                                let iss = delegation.issuer();
                                if !shown_dids.contains(&iss) {
                                    chain_dids.push((iss.clone(), delegation.is_powerline()));
                                    shown_dids.insert(iss);
                                }
                            }

                            // Add the final subject (space) if different
                            if let Some(last_del) = chain.last() {
                                if let DelegatedSubject::Specific(sub_did) = last_del.subject() {
                                    let sub = sub_did.to_string();
                                    if !shown_dids.contains(&sub) {
                                        chain_dids.push((sub, false));
                                    }
                                }
                            }

                            // Display the chain with tree nesting
                            for (idx, (did, is_powerline)) in chain_dids.iter().enumerate() {
                                let emoji = if *is_powerline { "🎫" } else { "🎟️ " };
                                let indent = "         ".to_string() + &"   ".repeat(idx);
                                println!("{}{}└─{} {}{}", dim_start, indent, emoji, did, dim_end);
                            }
                        }
                    }
                }
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

/// Build the complete authorization chain from space to operator
/// Returns the chain in order: [delegation closest to operator, ..., delegation from space]
fn build_authorization_chain(
    space_did: &str,
    cmd: &str,
    operator_did: &str,
    authority_did: &str,
    home: &std::path::Path,
) -> Result<Vec<Delegation>> {
    let mut chain = Vec::new();
    let mut visited = HashSet::new();

    // Start by finding the delegation from authority to operator (powerline)
    if let Some(auth_to_op) = find_delegation_for_chain(authority_did, operator_did, home)? {
        chain.push(auth_to_op);
    }

    // Now trace backwards from authority to space
    trace_to_space(
        space_did,
        cmd,
        authority_did,
        home,
        &mut chain,
        &mut visited,
        0,
    )?;

    Ok(chain)
}

/// Trace backwards from current DID to the space
fn trace_to_space(
    space_did: &str,
    cmd: &str,
    current_did: &str,
    home: &std::path::Path,
    chain: &mut Vec<Delegation>,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<()> {
    if depth > 10 || visited.contains(current_did) {
        return Ok(());
    }
    visited.insert(current_did.to_string());

    // Look for delegations TO current_did
    let access_dir = home.join(".tonk").join("access").join(current_did);
    if !access_dir.exists() {
        return Ok(());
    }

    // Find delegations that grant access to this space/command
    for entry in fs::read_dir(&access_dir)? {
        let entry = entry?;
        let issuer_dir = entry.path();

        if !issuer_dir.is_dir() {
            continue;
        }

        // Note: directory name is either issuer (for powerline) or subject (for specific subject)
        // We need to read the delegation to get the actual issuer

        // Look for delegation files
        for del_entry in fs::read_dir(&issuer_dir)? {
            let del_entry = del_entry?;
            let del_path = del_entry.path();

            if !del_path.is_file() || del_path.extension().and_then(|e| e.to_str()) != Some("cbor")
            {
                continue;
            }

            if let Ok(cbor_bytes) = fs::read(&del_path) {
                if let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes) {
                    if !delegation.is_valid() {
                        continue;
                    }

                    let cmd_str = delegation.command_str();
                    let actual_issuer = delegation.issuer(); // Get the real issuer from the delegation

                    // Check if this delegation is relevant
                    let is_relevant = match delegation.subject() {
                        DelegatedSubject::Specific(sub_did) => {
                            // Regular delegation - must be for our space and command
                            let sub = sub_did.to_string();
                            sub == space_did
                                && (cmd_str == cmd || cmd_str == "/" || cmd.starts_with(&cmd_str))
                        }
                        DelegatedSubject::Any => {
                            // Powerline - must be for the right command
                            cmd_str == cmd || cmd_str == "/" || cmd.starts_with(&cmd_str)
                        }
                    };

                    if is_relevant {
                        chain.push(delegation.clone());

                        // If issuer is not the space itself, keep tracing
                        if actual_issuer != space_did {
                            trace_to_space(
                                space_did,
                                cmd,
                                &actual_issuer,
                                home,
                                chain,
                                visited,
                                depth + 1,
                            )?;
                        }

                        return Ok(());
                    }
                }
            }
        }
    }

    Ok(())
}

/// Find a delegation from issuer to audience
fn find_delegation_for_chain(
    issuer_did: &str,
    audience_did: &str,
    home: &std::path::Path,
) -> Result<Option<Delegation>> {
    let access_dir = home
        .join(".tonk")
        .join("access")
        .join(audience_did)
        .join(issuer_did);

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

/// Collect all spaces accessible under a specific authority
pub fn collect_spaces_for_authority(
    _operator_did: &str,
    authority_did: &str,
) -> Result<HashMap<String, SpaceAccess>> {
    let home = crate::util::home_dir().context("Could not determine home directory")?;

    // First, collect what the authority has direct access to
    let mut spaces: HashMap<String, SpaceAccess> = HashMap::new();

    // The authority itself is always an authorization space (self-access)
    spaces.insert(
        authority_did.to_string(),
        SpaceAccess {
            space_did: authority_did.to_string(),
            commands: vec![("/".to_string(), i64::MAX)], // Full self-access, never expires
            is_auth_space: true,
            authorization_chains: HashMap::new(), // Self-access has no chain
        },
    );

    // Check authority's own access (this is what they've been delegated)
    let authority_access_dir = home.join(".tonk").join("access").join(authority_did);

    if authority_access_dir.exists() {
        for entry in fs::read_dir(&authority_access_dir)? {
            let entry = entry?;
            let subject_path = entry.path();

            if !subject_path.is_dir() {
                continue;
            }

            let issuer_did = subject_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Skip if not a did:key
            if !issuer_did.starts_with("did:key:") {
                continue;
            }

            // Read delegations from this issuer
            for delegation_entry in fs::read_dir(&subject_path)? {
                let delegation_entry = delegation_entry?;
                let delegation_path = delegation_entry.path();

                if !delegation_path.is_file()
                    || delegation_path.extension().and_then(|e| e.to_str()) != Some("cbor")
                {
                    continue;
                }

                if let Ok(cbor_bytes) = fs::read(&delegation_path) {
                    if let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes) {
                        // Only consider valid delegations
                        if !delegation.is_valid() {
                            continue;
                        }

                        let exp = delegation.expiration().unwrap_or(i64::MAX);
                        let cmd = delegation.command_str();

                        // Check if this is a powerline delegation
                        match delegation.subject() {
                            DelegatedSubject::Any => {
                                // Powerline: issuer becomes an authorization space
                                spaces
                                    .entry(issuer_did.clone())
                                    .or_insert_with(|| SpaceAccess {
                                        space_did: issuer_did.clone(),
                                        commands: Vec::new(),
                                        is_auth_space: true, // Issuer of powerline is always auth space
                                        authorization_chains: HashMap::new(),
                                    })
                                    .commands
                                    .push((cmd, exp));
                            }
                            DelegatedSubject::Specific(subject_did_ref) => {
                                // Regular delegation to a space
                                let subject_did = subject_did_ref.to_string();
                                let is_auth_space = subject_did == authority_did;
                                spaces
                                    .entry(subject_did.clone())
                                    .or_insert_with(|| SpaceAccess {
                                        space_did: subject_did.clone(),
                                        commands: Vec::new(),
                                        is_auth_space,
                                        authorization_chains: HashMap::new(),
                                    })
                                    .commands
                                    .push((cmd, exp));
                            }
                        }
                    }
                }
            }
        }
    }

    // Also recursively check for spaces the authority can access via powerline delegations
    let mut visited = HashSet::new();
    collect_spaces_recursive(
        authority_did,
        authority_did,
        &home,
        &mut visited,
        &mut spaces,
        0,
    )?;

    Ok(spaces)
}

/// Recursively collect spaces through powerline delegation chains
fn collect_spaces_recursive(
    authority_did: &str,
    did: &str,
    home: &std::path::PathBuf,
    visited: &mut HashSet<String>,
    spaces: &mut HashMap<String, SpaceAccess>,
    depth: usize,
) -> Result<()> {
    if depth > 10 || visited.contains(did) {
        return Ok(());
    }
    visited.insert(did.to_string());

    let access_dir = home.join(".tonk").join("access").join(did);
    if !access_dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&access_dir)? {
        let entry = entry?;
        let subject_path = entry.path();

        if !subject_path.is_dir() {
            continue;
        }

        for delegation_entry in fs::read_dir(&subject_path)? {
            let delegation_entry = delegation_entry?;
            let delegation_path = delegation_entry.path();

            if !delegation_path.is_file()
                || delegation_path.extension().and_then(|e| e.to_str()) != Some("cbor")
            {
                continue;
            }

            if let Ok(cbor_bytes) = fs::read(&delegation_path) {
                if let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes) {
                    if !delegation.is_valid() {
                        continue;
                    }

                    let is_powerline = delegation.is_powerline();

                    if is_powerline {
                        // Recurse into issuer's capabilities
                        let issuer = delegation.issuer();
                        collect_spaces_recursive(
                            authority_did,
                            &issuer,
                            home,
                            visited,
                            spaces,
                            depth + 1,
                        )?;
                    } else if let DelegatedSubject::Specific(subject_did) = delegation.subject() {
                        // Regular delegation to a space
                        let subject = subject_did.to_string();
                        if subject.starts_with("did:key:") {
                            let exp = delegation.expiration().unwrap_or(i64::MAX);
                            let cmd = delegation.command_str();
                            spaces
                                .entry(subject.clone())
                                .or_insert_with(|| SpaceAccess {
                                    space_did: subject.clone(),
                                    commands: Vec::new(),
                                    is_auth_space: subject == authority_did,
                                    authorization_chains: HashMap::new(),
                                })
                                .commands
                                .push((cmd, exp));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
