use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use ucan_core::did::{Did, Ed25519Did};
use varsig::signature::eddsa::EdDsa;
use varsig::curve::Edwards25519;
use varsig::hash::Sha2_512;

/// A universal DID type that can be either Ed25519 or mailto.
/// This allows us to use ucan_core's Delegation<D> with mixed DID types.
#[derive(Debug, Clone, PartialEq)]
pub enum UniversalDid {
    Ed25519(Ed25519Did),
    Mailto(String), // Just store the email directly
}

impl UniversalDid {
    pub fn mailto(email: String) -> Self {
        UniversalDid::Mailto(email)
    }

    pub fn ed25519(did: Ed25519Did) -> Self {
        UniversalDid::Ed25519(did)
    }
}

impl Did for UniversalDid {
    type VarsigConfig = EdDsa<Edwards25519, Sha2_512>;

    fn did_method(&self) -> &str {
        match self {
            UniversalDid::Ed25519(did) => did.did_method(),
            UniversalDid::Mailto(_) => "mailto",
        }
    }

    fn varsig_config(&self) -> &Self::VarsigConfig {
        match self {
            UniversalDid::Ed25519(did) => did.varsig_config(),
            UniversalDid::Mailto(_) => {
                // For mailto, we should never actually use varsig operations
                // But we need to return something, so panic if this is ever called
                panic!("Cannot perform cryptographic operations on did:mailto")
            }
        }
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
            let email = s.strip_prefix("did:mailto:")
                .ok_or_else(|| anyhow!("Failed to parse did:mailto"))?
                .to_string();

            if email.is_empty() {
                return Err(anyhow!("Email cannot be empty in did:mailto"));
            }

            Ok(UniversalDid::Mailto(email))
        } else if s.starts_with("did:key:") {
            let ed25519_did: Ed25519Did = s.parse()
                .map_err(|e| anyhow!("Failed to parse did:key: {:?}", e))?;
            Ok(UniversalDid::Ed25519(ed25519_did))
        } else {
            Err(anyhow!("Unsupported DID method, expected did:key or did:mailto"))
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

impl From<Ed25519Did> for UniversalDid {
    fn from(did: Ed25519Did) -> Self {
        UniversalDid::Ed25519(did)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_did_mailto_creation() {
        let did = DidMailto::new("user@example.com".to_string());
        assert_eq!(did.to_string(), "did:mailto:user@example.com");
        assert_eq!(did.email(), "user@example.com");
        assert_eq!(did.did_method(), "mailto");
    }

    #[test]
    fn test_did_mailto_parsing() {
        let did: DidMailto = "did:mailto:user@example.com".parse().unwrap();
        assert_eq!(did.email(), "user@example.com");
    }

    #[test]
    fn test_did_mailto_invalid() {
        assert!("not-a-did".parse::<DidMailto>().is_err());
        assert!("did:key:z6Mk...".parse::<DidMailto>().is_err());
        assert!("did:mailto:".parse::<DidMailto>().is_err());
    }

    #[test]
    fn test_did_mailto_serialization() {
        let did = DidMailto::new("user@example.com".to_string());
        let json = serde_json::to_string(&did).unwrap();
        assert_eq!(json, r#""did:mailto:user@example.com""#);

        let deserialized: DidMailto = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, did);
    }
}
