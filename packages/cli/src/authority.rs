use crate::delegation::Delegation;
use crate::keystore::Keystore;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;

/// Information about an available authority (login session)
#[derive(Debug, Clone)]
pub struct Authority {
    /// The authority's DID
    pub did: String,

    /// Commands the authority has delegated to operator (usually "/")
    pub commands: Vec<String>,

    /// Expiration timestamp of the delegation
    pub expiration: i64,
}

/// Get all available authorities (issuers from powerline delegations)
pub fn get_authorities() -> Result<Vec<Authority>> {
    let keystore = Keystore::new().context("Failed to initialize keystore")?;
    let operator = keystore
        .get_or_create_keypair()
        .context("Failed to get operator keypair")?;
    let operator_did = operator.to_did_key();

    let home = crate::util::home_dir().context("Could not determine home directory")?;
    let access_dir = home.join(".tonk").join("access").join(&operator_did);

    if !access_dir.exists() {
        return Ok(Vec::new());
    }

    let mut authorities: HashMap<String, Authority> = HashMap::new();

    // Walk through issuer directories
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
            if let Ok(cbor_bytes) = fs::read(&delegation_path) {
                if let Ok(delegation) = Delegation::from_cbor_bytes(&cbor_bytes) {
                    // Only consider valid powerline delegations
                    if !delegation.is_valid() {
                        continue;
                    }

                    if !delegation.is_powerline() {
                        continue;
                    }

                    let authority_did = delegation.issuer();

                    // Track this authority
                    authorities
                        .entry(authority_did.clone())
                        .or_insert_with(|| Authority {
                            did: authority_did,
                            commands: Vec::new(),
                            expiration: delegation.expiration().unwrap_or(i64::MAX),
                        })
                        .commands
                        .push(delegation.command_str());
                }
            }
        }
    }

    // Sort authorities by DID for consistent ordering
    let mut result: Vec<Authority> = authorities.into_values().collect();
    result.sort_by(|a, b| a.did.cmp(&b.did));
    Ok(result)
}

/// Get the currently active authority, or the first available one
pub fn get_active_authority() -> Result<Option<Authority>> {
    let authorities = get_authorities()?;

    if authorities.is_empty() {
        return Ok(None);
    }

    // Use state to get active session
    let active_did = crate::state::get_active_session()?;

    if let Some(active_did) = active_did {
        // Try to find the active authority
        if let Some(auth) = authorities.iter().find(|a| a.did == active_did) {
            return Ok(Some(auth.clone()));
        }
    }

    // Default to first authority if no active one is set or not found
    Ok(Some(authorities[0].clone()))
}

/// Format authority DID for display (shortened)
pub fn format_authority_did(did: &str) -> String {
    if did.len() > 20 {
        format!("{}...{}", &did[..15], &did[did.len() - 8..])
    } else {
        did.to_string()
    }
}
