use crate::delegation::Delegation;
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

/// Represents how a capability was delegated
#[derive(Debug, Clone)]
enum DelegationPath {
    /// Direct delegation to the operator
    Direct { issuer: String },
    /// Indirect delegation via authority chain
    Via { authority: String, hops: usize },
}

/// Represents access to a specific command on a subject
#[derive(Debug, Clone)]
struct CommandAccess {
    command: String,
    expiration: i64,
    path: DelegationPath,
    delegation: Delegation,
    file_path: PathBuf,
}

/// Recursively collect all subjects and commands accessible via a DID's delegations
fn collect_subject_access(
    did: &str,
    home: &PathBuf,
    visited: &mut HashSet<String>,
    depth: usize,
) -> HashMap<String, Vec<(String, i64)>> {
    // Prevent infinite recursion and cycles
    if depth > 10 || visited.contains(did) {
        return HashMap::new();
    }
    visited.insert(did.to_string());

    let mut access: HashMap<String, Vec<(String, i64)>> = HashMap::new();

    let access_dir = home.join(".tonk").join("access").join(did);
    if !access_dir.exists() {
        return access;
    }

    // Walk through subject/issuer directories
    for entry in fs::read_dir(&access_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
    {
        let subject_path = entry.path();
        if !subject_path.is_dir() {
            continue;
        }

        // Read delegation files in this directory
        for delegation_entry in fs::read_dir(&subject_path)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
        {
            let delegation_path = delegation_entry.path();

            // Only process .cbor files
            if delegation_path.extension().and_then(|e| e.to_str()) != Some("cbor") {
                continue;
            }

            if let Ok(cbor_bytes) = fs::read(&delegation_path)
                && let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes)
            {
                // Only consider valid delegations
                if !delegation.is_valid() {
                    continue;
                }

                let is_powerline = delegation.is_powerline();

                if is_powerline {
                    // Powerline delegation - recurse into issuer's capabilities
                    let issuer_access =
                        collect_subject_access(&delegation.issuer(), home, visited, depth + 1);

                    // Merge issuer's access into ours
                    for (subject, commands) in issuer_access {
                        access.entry(subject).or_default().extend(commands);
                    }
                } else {
                    // Regular delegation - direct access to subject
                    let subject = delegation.subject().to_string();
                    let exp_secs = delegation.expiration().unwrap_or(0);
                    access
                        .entry(subject)
                        .or_default()
                        .push((delegation.command_str(), exp_secs));
                }
            }
        }
    }

    access
}

/// Format time remaining until expiration
fn format_time_remaining(exp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let remaining = exp - now;

    if remaining < 0 {
        return "EXPIRED".to_string();
    }

    let days = remaining / 86400;
    let hours = (remaining % 86400) / 3600;
    let mins = (remaining % 3600) / 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

/// Shorten a DID for display
fn shorten_did(did: &str) -> String {
    if did.len() > 20 {
        format!("{}...{}", &did[..15], &did[did.len() - 8..])
    } else {
        did.to_string()
    }
}

/// Execute the status command
pub async fn execute(verbose: bool) -> Result<()> {
    println!("📊 Status\n");

    // Get operator DID
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    if verbose {
        println!("🤖 Operator: {}\n", operator_did);
    } else {
        println!("🤖 Operator: {}\n", shorten_did(&operator_did));
    }

    // Find delegations directory
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let access_dir = home.join(".tonk").join("access").join(&operator_did);

    if !access_dir.exists() {
        println!("⚠  No access");
        println!("   Run 'tonk login' to authenticate\n");
        return Ok(());
    }

    // Collect all command access grouped by subject
    let mut subject_access: HashMap<String, Vec<CommandAccess>> = HashMap::new();
    let mut powerline_authorities: Vec<(String, Delegation, PathBuf)> = Vec::new();

    // Walk through subject/issuer directories
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
            match fs::read(&delegation_path) {
                Ok(cbor_bytes) => match Delegation::from_cbor_bytes(&cbor_bytes) {
                    Ok(delegation) => {
                        // Skip expired delegations
                        if !delegation.is_valid() {
                            continue;
                        }

                        let is_powerline = delegation.is_powerline();

                        if is_powerline {
                            // Track powerline delegations for later processing
                            powerline_authorities.push((
                                delegation.issuer().to_string(),
                                delegation.clone(),
                                delegation_path,
                            ));
                        } else {
                            // Direct delegation to a subject
                            let subject = delegation.subject().to_string();
                            subject_access
                                .entry(subject)
                                .or_default()
                                .push(CommandAccess {
                                    command: delegation.command_str(),
                                    expiration: delegation.expiration().unwrap_or(0),
                                    path: DelegationPath::Direct {
                                        issuer: delegation.issuer().to_string(),
                                    },
                                    delegation,
                                    file_path: delegation_path,
                                });
                        }
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("⚠ Failed to parse {}: {}", delegation_path.display(), e);
                        }
                    }
                },
                Err(e) => {
                    if verbose {
                        eprintln!("⚠ Failed to read {}: {}", delegation_path.display(), e);
                    }
                }
            }
        }
    }

    // Process powerline delegations to collect indirect access
    for (authority_did, powerline_delegation, powerline_path) in powerline_authorities {
        // A powerline delegation grants access to the authority's DID itself
        subject_access
            .entry(authority_did.clone())
            .or_default()
            .push(CommandAccess {
                command: powerline_delegation.command_str(),
                expiration: powerline_delegation.expiration().unwrap_or(0),
                path: DelegationPath::Direct {
                    issuer: authority_did.clone(),
                },
                delegation: powerline_delegation.clone(),
                file_path: powerline_path.clone(),
            });

        // Also collect everything the authority has access to (indirect access)
        let mut visited = HashSet::new();
        let authority_access = collect_subject_access(&authority_did, &home, &mut visited, 0);

        for (subject, commands) in authority_access {
            for (command, exp) in commands {
                subject_access
                    .entry(subject.clone())
                    .or_default()
                    .push(CommandAccess {
                        command,
                        expiration: exp,
                        path: DelegationPath::Via {
                            authority: authority_did.clone(),
                            hops: 1, // Simplified - could track actual depth
                        },
                        delegation: powerline_delegation.clone(),
                        file_path: powerline_path.clone(),
                    });
            }
        }
    }

    if subject_access.is_empty() {
        println!("⚠  No access");
        println!("   Run 'tonk login' to authenticate\n");
        return Ok(());
    }

    // Display access grouped by subject
    println!("📜 Access:\n");

    let mut subjects: Vec<_> = subject_access.keys().collect();
    subjects.sort();

    for subject in subjects {
        let accesses = &subject_access[subject];

        // Display subject header
        if verbose {
            println!("  {}", subject);
        } else {
            println!("  {}", shorten_did(subject));
        }

        // Group commands by (command, expiration) to deduplicate and show paths
        let mut command_groups: HashMap<(String, i64), Vec<&CommandAccess>> = HashMap::new();
        for access in accesses {
            command_groups
                .entry((access.command.clone(), access.expiration))
                .or_default()
                .push(access);
        }

        let mut commands: Vec<_> = command_groups.keys().collect();
        commands.sort();

        for (i, (cmd, exp)) in commands.iter().enumerate() {
            let is_last = i == commands.len() - 1;
            let prefix = if is_last { "└─" } else { "├─" };

            let time_str = format_time_remaining(*exp);
            let accesses = &command_groups[&(cmd.clone(), *exp)];

            // Show command and expiration
            print!("  {}  {} (expires: {})", prefix, cmd, time_str);

            // Show delegation path
            if accesses.len() == 1 {
                match &accesses[0].path {
                    DelegationPath::Direct { issuer } => {
                        if verbose {
                            println!(" [direct from {}]", issuer);
                        } else {
                            println!(" [direct]");
                        }
                    }
                    DelegationPath::Via { authority, hops } => {
                        if verbose {
                            println!(
                                " [via {} ({} hop{})]",
                                authority,
                                hops,
                                if *hops == 1 { "" } else { "s" }
                            );
                        } else {
                            println!(" [via {}]", shorten_did(authority));
                        }
                    }
                }
            } else {
                // Multiple paths to same capability
                println!();
                for access in accesses {
                    let nested_prefix = if is_last { "    " } else { "│   " };
                    match &access.path {
                        DelegationPath::Direct { issuer } => {
                            if verbose {
                                println!("  {}  • direct from {}", nested_prefix, issuer);
                            } else {
                                println!("  {}  • direct", nested_prefix);
                            }
                        }
                        DelegationPath::Via { authority, hops } => {
                            if verbose {
                                println!(
                                    "  {}  • via {} ({} hop{})",
                                    nested_prefix,
                                    authority,
                                    hops,
                                    if *hops == 1 { "" } else { "s" }
                                );
                            } else {
                                println!("  {}  • via {}", nested_prefix, shorten_did(authority));
                            }
                        }
                    }
                }
            }

            // Show detailed info if verbose
            if verbose {
                let nested_prefix = if is_last { "    " } else { "│   " };
                for access in accesses {
                    println!("  {}  File: {}", nested_prefix, access.file_path.display());

                    // Load and display metadata if available
                    if let Ok(Some(metadata)) = access.delegation.load_metadata() {
                        let site_type = if metadata.is_local { "Local" } else { "Remote" };
                        println!(
                            "  {}  Source: {} ({})",
                            nested_prefix, metadata.site, site_type
                        );
                    }
                }
            }
        }

        println!();
    }

    Ok(())
}
