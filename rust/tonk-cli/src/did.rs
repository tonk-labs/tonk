use anyhow::{Result, anyhow};
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A universal DID type that can be either Ed25519 (did:key) or mailto.
///
/// With the new dialog-varsig crate, `Did` is a universal string wrapper,
/// so this type mainly provides parsing validation and convenience constructors.
#[derive(Debug, Clone, PartialEq)]
pub enum UniversalDid {
    Ed25519(Did),
    Mailto(String), // Just store the email directly
}

impl UniversalDid {
    pub fn mailto(email: String) -> Self {
        UniversalDid::Mailto(email)
    }

    pub fn ed25519(did: Did) -> Self {
        UniversalDid::Ed25519(did)
    }

    /// Convert to a `Did` (for use with UCAN builder APIs).
    /// For mailto DIDs, this creates a Did from the full "did:mailto:..." string.
    pub fn to_did(&self) -> Did {
        self.to_string()
            .parse()
            .expect("UniversalDid always produces valid Did strings")
    }
}

impl fmt::Display for UniversalDid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UniversalDid::Ed25519(did) => write!(f, "{}", did),
            UniversalDid::Mailto(email) => write!(f, "did:mailto:{}", email),
        }
    }
}

impl FromStr for UniversalDid {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("did:mailto:") {
            let email = s
                .strip_prefix("did:mailto:")
                .ok_or_else(|| anyhow!("Failed to parse did:mailto"))?
                .to_string();

            if email.is_empty() {
                return Err(anyhow!("Email cannot be empty in did:mailto"));
            }

            Ok(UniversalDid::Mailto(email))
        } else if s.starts_with("did:key:") {
            let did: Did = s
                .parse()
                .map_err(|e| anyhow!("Failed to parse did:key: {:?}", e))?;
            Ok(UniversalDid::Ed25519(did))
        } else {
            Err(anyhow!(
                "Unsupported DID method, expected did:key or did:mailto"
            ))
        }
    }
}

impl Serialize for UniversalDid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for UniversalDid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse()
            .map_err(|e| serde::de::Error::custom(format!("Unable to parse DID: {}", e)))
    }
}

impl From<Did> for UniversalDid {
    fn from(did: Did) -> Self {
        UniversalDid::Ed25519(did)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_universal_did_mailto_creation() {
        let did = UniversalDid::mailto("user@example.com".to_string());
        assert_eq!(did.to_string(), "did:mailto:user@example.com");
    }

    #[test]
    fn test_universal_did_mailto_parsing() {
        let did: UniversalDid = "did:mailto:user@example.com".parse().unwrap();
        assert_eq!(did.to_string(), "did:mailto:user@example.com");
    }

    #[test]
    fn test_universal_did_invalid() {
        assert!("not-a-did".parse::<UniversalDid>().is_err());
        assert!("did:mailto:".parse::<UniversalDid>().is_err());
    }

    #[test]
    fn test_universal_did_mailto_serialization() {
        let did = UniversalDid::mailto("user@example.com".to_string());
        let json = serde_json::to_string(&did).unwrap();
        assert_eq!(json, r#""did:mailto:user@example.com""#);

        let deserialized: UniversalDid = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, did);
    }

    #[test]
    fn test_universal_did_ed25519_parsing() {
        // Test that we can still parse did:key
        let did_str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let did: UniversalDid = did_str.parse().unwrap();
        assert_eq!(did.to_string(), did_str);
    }
}
