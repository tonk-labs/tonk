use crate::crypto::Keypair;
use crate::delegation::Delegation;
use anyhow::{Context, Result};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// Invitation file structure
/// This is the file sent to an invitee to join a space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteFile {
    /// 5 char invite code derived from invitation signature
    /// This is shared separately (e.g., via email) for security
    pub invite_code: String,

    /// Hash/CID of the invitation delegation
    pub invitation_cid: String,

    /// Space DID
    pub space_did: String,

    /// Human-readable space name
    pub space_name: String,

    /// Inviter's identifier (email or DID)
    pub inviter: String,

    /// Invitee's identifier (email)
    pub invitee: String,

    /// Membership delegation: Profile/Space → Membership DID
    /// This is the delegation that grants the membership DID access
    pub membership_delegation: Delegation,

    /// TODO: Time-limited UCAN invocation for reading the invitation
    /// This would allow the invitee to verify the invitation on-chain/remotely
    /// For now, we trust the invite file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_invocation: Option<serde_json::Value>,
}

impl InviteFile {
    /// Save the invite file to a path
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("Failed to serialize invite file")?;

        std::fs::write(path, json).context("Failed to write invite file")?;

        Ok(())
    }

    /// Load an invite file from a path
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let json = std::fs::read_to_string(path).context("Failed to read invite file")?;

        let invite: InviteFile =
            serde_json::from_str(&json).context("Failed to parse invite file")?;

        Ok(invite)
    }
}

/// Generate a short invite code from delegation hash bytes
/// Takes the last 5 bytes and encodes them as a readable string
pub fn generate_invite_code(delegation_hash: &[u8]) -> String {
    // Take last 5 bytes of hash
    let code_bytes = &delegation_hash[delegation_hash.len().saturating_sub(5)..];

    // Encode as base32 for readability (no ambiguous characters like 0/O, 1/l)
    // Using a custom alphabet that's easy to read and type
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

    let mut code = String::with_capacity(8);
    for byte in code_bytes {
        let idx = (byte % ALPHABET.len() as u8) as usize;
        code.push(ALPHABET[idx] as char);
    }

    code
}

/// Derive a membership keypair from an invitation hash and invite code
/// Uses HKDF to deterministically derive a keypair that only the invitee can recreate
///
/// # Arguments
/// * `delegation_hash` - The hash bytes from the invitation delegation (deterministic identifier)
/// * `invite_code` - The short code shared with the invitee (e.g., "A3F9K")
///
/// # Returns
/// A deterministically derived keypair that can be used as the membership identity
pub fn derive_membership_key(delegation_hash: &[u8], invite_code: &str) -> Result<Keypair> {
    // Use HKDF to derive a deterministic key from the delegation hash and code
    // Input Key Material: invitation delegation hash
    // Salt: None (the hash is already strong)
    // Info: "tonk-membership-v1" + invite_code
    let info = format!("tonk-membership-v1:{}", invite_code);

    let hkdf = Hkdf::<Sha256>::new(None, delegation_hash);
    let mut membership_seed = [0u8; 32];

    hkdf.expand(info.as_bytes(), &mut membership_seed)
        .map_err(|_| anyhow::anyhow!("HKDF expansion failed"))?;

    // Create keypair from derived seed
    let keypair = Keypair::from_bytes(&membership_seed);

    Ok(keypair)
}

/// Verify that an invite code matches a membership delegation
/// Returns true if the code correctly derives the membership DID
pub fn verify_invite_code(
    delegation_hash: &[u8],
    invite_code: &str,
    expected_membership_did: &str,
) -> Result<bool> {
    let derived_keypair: Keypair = derive_membership_key(delegation_hash, invite_code)?;
    let derived_did = derived_keypair.to_did_key();

    Ok(derived_did == expected_membership_did)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_invite_code() {
        let hash = b"this is a test hash with enough bytes";
        let code = generate_invite_code(hash);

        // Should be non-empty
        assert!(!code.is_empty());

        // Should only contain valid characters
        assert!(
            code.chars()
                .all(|c| "23456789ABCDEFGHJKLMNPQRSTUVWXYZ".contains(c))
        );
    }

    #[test]
    fn test_derive_membership_key_deterministic() {
        let hash = b"test hash for determinism check";
        let invite_code = "ABC123";

        // Derive twice with same inputs
        let key1 = derive_membership_key(hash, invite_code).unwrap();
        let key2 = derive_membership_key(hash, invite_code).unwrap();

        // Should produce the same DID
        assert_eq!(key1.to_did_key(), key2.to_did_key());
    }

    #[test]
    fn test_derive_membership_key_different_codes() {
        let hash = b"test hash";
        let code1 = "ABC123";
        let code2 = "XYZ789";

        // Derive with different codes
        let key1 = derive_membership_key(hash, code1).unwrap();
        let key2 = derive_membership_key(hash, code2).unwrap();

        // Should produce different DIDs
        assert_ne!(key1.to_did_key(), key2.to_did_key());
    }

    #[test]
    fn test_verify_invite_code() {
        let hash = b"test hash for verification";
        let invite_code = "TEST5";

        // Derive a membership key
        let keypair = derive_membership_key(hash, invite_code).unwrap();
        let did = keypair.to_did_key();

        // Verify should succeed with correct code
        assert!(verify_invite_code(hash, invite_code, &did).unwrap());

        // Verify should fail with wrong code
        assert!(!verify_invite_code(hash, "WRONG", &did).unwrap());
    }
}
