//! Canonical account-creation recovery contracts shared by clients and
//! providers.

use std::fmt;
use std::str::FromStr;

/// Passkey facts Tonk recorded during the account-creation ceremony.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountCreationPasskey<'a> {
    /// Ceremony time as Unix seconds.
    pub created_at: u64,
    /// Browser and operating-system label captured by the creating client.
    pub created_on: &'a str,
}

/// Every caller-controlled fact that identifies one account creation.
///
/// The algorithm lowercases `email` and trims `passkey.created_on`, matching
/// the provider's input normalization. Provider attachment IDs and provider
/// timestamps are intentionally not inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountCreationFingerprintInput<'a> {
    /// Email address submitted by the creating client.
    pub email: &'a str,
    /// Passkey-derived account root DID.
    pub root_did: &'a str,
    /// Opaque WebAuthn credential identifier.
    pub credential_id: &'a str,
    /// Optional facts from the passkey creation ceremony.
    pub passkey: Option<AccountCreationPasskey<'a>>,
    /// Canonical root-signed account repository descriptor bytes.
    pub descriptor: &'a [u8],
    /// DID of the account's first device.
    pub device_did: &'a str,
    /// Human-readable name of the account's first device.
    pub device_name: &'a str,
    /// CID of the first device's exact root-to-device delegation.
    pub delegation_cid: &'a str,
    /// Exact decoded root-to-device delegation container bytes.
    pub delegation: &'a [u8],
}

impl AccountCreationFingerprintInput<'_> {
    /// Compute the version-1, domain-separated creation fingerprint.
    #[must_use]
    pub fn fingerprint(&self) -> AccountCreationFingerprint {
        fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
            hasher.update(&(bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }

        let normalized_email = self.email.to_lowercase();
        let mut hasher = blake3::Hasher::new();
        field(&mut hasher, b"tonk-account-create-v1");
        field(&mut hasher, normalized_email.as_bytes());
        field(&mut hasher, self.root_did.as_bytes());
        field(&mut hasher, self.credential_id.as_bytes());
        match self.passkey {
            None => field(&mut hasher, &[0]),
            Some(passkey) => {
                field(&mut hasher, &[1]);
                field(&mut hasher, &passkey.created_at.to_be_bytes());
                field(&mut hasher, passkey.created_on.trim().as_bytes());
            }
        }
        field(&mut hasher, self.descriptor);
        field(&mut hasher, self.device_did.as_bytes());
        field(&mut hasher, self.device_name.as_bytes());
        field(&mut hasher, self.delegation_cid.as_bytes());
        field(&mut hasher, self.delegation);
        AccountCreationFingerprint(*hasher.finalize().as_bytes())
    }
}

/// A version-1 canonical account-creation fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountCreationFingerprint([u8; 32]);

impl AccountCreationFingerprint {
    /// Parse the canonical wire form: exactly 64 lowercase hexadecimal bytes.
    pub fn from_hex(value: &str) -> Result<Self, AccountCreationFingerprintError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AccountCreationFingerprintError::InvalidFormat);
        }
        let decoded =
            hex::decode(value).map_err(|_| AccountCreationFingerprintError::InvalidFormat)?;
        let bytes: [u8; 32] = decoded
            .try_into()
            .map_err(|_| AccountCreationFingerprintError::InvalidFormat)?;
        Ok(Self(bytes))
    }

    /// Format the canonical lowercase 64-hex wire form.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    /// Borrow the 32 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for AccountCreationFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl FromStr for AccountCreationFingerprint {
    type Err = AccountCreationFingerprintError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_hex(value)
    }
}

/// Invalid account-creation fingerprint wire input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccountCreationFingerprintError {
    /// The value was not exactly 64 lowercase hexadecimal characters.
    #[error("createFingerprint must be 32 bytes of lowercase hex")]
    InvalidFormat,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXED_FINGERPRINT: &str =
        "35cda4b0895490a01c4307584da2fe045c568b53d9817b3f38c97309a07dbb52";

    fn fixed_input<'a>() -> AccountCreationFingerprintInput<'a> {
        AccountCreationFingerprintInput {
            email: "PERSON@EXAMPLE.COM",
            root_did: "did:key:zRoot",
            credential_id: "credential-01",
            passkey: Some(AccountCreationPasskey {
                created_at: 1_725_000_000,
                created_on: "  Chrome on macOS  ",
            }),
            descriptor: &[0x00, 0x01, 0x02, 0xff],
            device_did: "did:key:zDevice",
            device_name: "Jack's MacBook",
            delegation_cid: "bafydelegation",
            delegation: &[0x82, 0x01, 0x02],
        }
    }

    #[test]
    fn it_pins_the_versioned_account_creation_fingerprint() {
        let fingerprint = fixed_input().fingerprint();
        assert_eq!(fingerprint.to_hex(), FIXED_FINGERPRINT);

        let normalized = AccountCreationFingerprintInput {
            email: "person@example.com",
            passkey: Some(AccountCreationPasskey {
                created_at: 1_725_000_000,
                created_on: "Chrome on macOS",
            }),
            ..fixed_input()
        };
        assert_eq!(normalized.fingerprint(), fingerprint);
    }

    #[test]
    fn it_accepts_only_the_canonical_fingerprint_wire_format() {
        let parsed = AccountCreationFingerprint::from_hex(FIXED_FINGERPRINT).unwrap();
        assert_eq!(parsed.to_hex(), FIXED_FINGERPRINT);
        assert!(AccountCreationFingerprint::from_hex(&FIXED_FINGERPRINT[..62]).is_err());
        assert!(AccountCreationFingerprint::from_hex(&"gg".repeat(32)).is_err());
        assert!(AccountCreationFingerprint::from_hex(&FIXED_FINGERPRINT.to_uppercase()).is_err());
    }
}
