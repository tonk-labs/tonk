use crate::authority;
use crate::delegation::Delegation;
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;

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
    pub is_auth_space: bool,          // True if this is the authority's own DID (authorization space)
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
        println!("⚠  No sessions");
        println!("   Run 'tonk login' to authenticate\n");
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
pub async fn list() -> Result<()> {
    // Get operator DID
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    // Get all available authorities/sessions
    let authorities = authority::get_authorities()?;

    if authorities.is_empty() {
        println!("⚠  No sessions");
        println!("   Run 'tonk login' to authenticate\n");
        return Ok(());
    }

    // Get active authority
    let active_authority = get_active_authority_from_list(&authorities)?;

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
        println!(
            "{}👤 Authority: {}{}",
            dim_start,
            auth.did,
            dim_end
        );
        println!(
            "{}🤖 Operator:  {}{}",
            dim_start,
            operator_did,
            dim_end
        );

        // Collect spaces for this authority
        let spaces = collect_spaces_for_authority(&operator_did, &auth.did)?;

        if spaces.is_empty() {
            println!("{}🎟️  Access: none{}", dim_start, dim_end);
        } else {
            println!("{}🎟️  Access:{}", dim_start, dim_end);

            // Sort spaces: auth space first, then alphabetically
            let mut sorted_spaces: Vec<_> = spaces.values().collect();
            sorted_spaces.sort_by(|a, b| match (a.is_auth_space, b.is_auth_space) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.space_did.cmp(&b.space_did),
            });

            for space in sorted_spaces {
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
                } else {
                    if let Some(name) = &space_name {
                        if name != &space.space_did {
                            println!(
                                "{}   {} {} ({}){}",
                                dim_start, emoji, name, space.space_did, dim_end
                            );
                        } else {
                            println!(
                                "{}   {} {}{}",
                                dim_start, emoji, space.space_did, dim_end
                            );
                        }
                    } else {
                        println!(
                            "{}   {} {}{}",
                            dim_start, emoji, space.space_did, dim_end
                        );
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
                        println!(
                            "{}      {} ⚙️  {}{}",
                            dim_start, prefix, cmd, dim_end
                        );
                    } else {
                        println!(
                            "{}      {} ⚙️  {} (expires: {}){}",
                            dim_start, prefix, cmd, time_str, dim_end
                        );
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
                    || delegation_path.extension().and_then(|e| e.to_str()) != Some("json")
                {
                    continue;
                }

                if let Ok(json) = fs::read_to_string(&delegation_path) {
                    if let Ok(delegation) = serde_json::from_str::<Delegation>(&json) {
                        // Only consider valid delegations
                        if !delegation.is_valid() {
                            continue;
                        }

                        // Check if this is a powerline delegation (sub: null)
                        if delegation.payload.sub.is_none() {
                            // Powerline: issuer becomes an authorization space
                            spaces
                                .entry(issuer_did.clone())
                                .or_insert_with(|| SpaceAccess {
                                    space_did: issuer_did.clone(),
                                    commands: Vec::new(),
                                    is_auth_space: true, // Issuer of powerline is always auth space
                                })
                                .commands
                                .push((delegation.payload.cmd.clone(), delegation.payload.exp));
                        } else if let Some(ref subject_did) = delegation.payload.sub {
                            // Regular delegation to a space
                            let is_auth_space = subject_did == authority_did;
                            spaces
                                .entry(subject_did.clone())
                                .or_insert_with(|| SpaceAccess {
                                    space_did: subject_did.clone(),
                                    commands: Vec::new(),
                                    is_auth_space,
                                })
                                .commands
                                .push((delegation.payload.cmd.clone(), delegation.payload.exp));
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

            if let Ok(json) = fs::read_to_string(&delegation_path) {
                if let Ok(delegation) = serde_json::from_str::<Delegation>(&json) {
                    if !delegation.is_valid() {
                        continue;
                    }

                    let is_powerline = delegation.payload.sub.is_none();

                    if is_powerline {
                        // Recurse into issuer's capabilities
                        collect_spaces_recursive(
                            authority_did,
                            delegation.issuer(),
                            home,
                            visited,
                            spaces,
                            depth + 1,
                        )?;
                    } else if let Some(ref subject) = delegation.payload.sub {
                        // Regular delegation to a space
                        if subject.starts_with("did:key:") {
                            spaces
                                .entry(subject.clone())
                                .or_insert_with(|| SpaceAccess {
                                    space_did: subject.clone(),
                                    commands: Vec::new(),
                                    is_auth_space: subject == authority_did,
                                })
                                .commands
                                .push((delegation.payload.cmd.clone(), delegation.payload.exp));
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
