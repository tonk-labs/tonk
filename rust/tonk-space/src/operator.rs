//! Operator identity for signing operations.
//!
//! An `Operator` represents a principal that can sign UCAN delegations and
//! dialog-db operations. It wraps an Ed25519 signing key and provides
//! conversions to both `Ed25519Signer` (for UCAN) and dialog-db `Credentials`.

use dialog_artifacts::repository::Credentials;
use dialog_credentials::{Ed25519Signer, Ed25519SigningKey};
use dialog_varsig::{Did, Principal};
use ed25519_dalek::SigningKey;
use rand_0_8::rngs::OsRng;

/// An operator identity that can sign UCAN delegations and dialog-db operations.
///
/// This is the primary identity type for tonk-space. It wraps an Ed25519 signing
/// key and can be converted to:
/// - `Ed25519Signer` for signing UCAN delegations
/// - `Credentials` for signing dialog-db repository operations
#[derive(Debug, Clone)]
pub struct Operator(Ed25519Signer);

impl Operator {
    /// Generate a new random operator identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        Self(Ed25519Signer::from(signing_key))
    }

    /// Create an operator from a passphrase using HKDF key derivation.
    ///
    /// Uses HKDF-SHA256 with a domain-specific info string for key derivation.
    /// On native, this uses the `hkdf` crate synchronously.
    /// On WASM, this uses Web Crypto API (`crypto.subtle`) asynchronously.
    pub async fn from_passphrase(passphrase: &str) -> Self {
        let passphrase = crate::Passphrase::from(passphrase);
        let signing_key = passphrase.derive_signing_key(None).await;
        Self(Ed25519Signer::from(signing_key))
    }

    /// Get the DID for this operator.
    pub fn did(&self) -> Did {
        Principal::did(&self.0)
    }

    /// Get the underlying signing key (native platforms only).
    ///
    /// On native, this extracts the `ed25519_dalek::SigningKey` from the
    /// platform-abstracted `Ed25519SigningKey` enum.
    pub fn signer(&self) -> &SigningKey {
        match self.0.signing_key() {
            Ed25519SigningKey::Native(key) => key,
            #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
            Ed25519SigningKey::WebCrypto(_) => {
                panic!("Cannot get native signing key on WASM; use ucan_signer() instead")
            }
        }
    }

    /// Get the underlying `Ed25519Signer` for UCAN operations.
    pub fn ucan_signer(&self) -> &Ed25519Signer {
        &self.0
    }

    /// Get the secret key bytes.
    pub fn to_secret(&self) -> [u8; 32] {
        self.signer().to_bytes()
    }

    /// Create an operator from raw secret key bytes.
    ///
    /// This is the inverse of `to_secret()` - it allows reconstructing an
    /// operator from previously stored secret bytes.
    pub fn from_secret(secret: [u8; 32]) -> Self {
        Self(Ed25519Signer::from(SigningKey::from_bytes(&secret)))
    }
}

impl From<Operator> for Ed25519Signer {
    fn from(operator: Operator) -> Self {
        operator.0
    }
}

impl From<&Operator> for Ed25519Signer {
    fn from(operator: &Operator) -> Self {
        operator.0.clone()
    }
}

impl From<Operator> for Credentials {
    fn from(operator: Operator) -> Self {
        Credentials::from(Ed25519Signer::from(operator))
    }
}

impl From<&Operator> for Credentials {
    fn from(operator: &Operator) -> Self {
        Credentials::from(Ed25519Signer::from(operator))
    }
}

impl From<SigningKey> for Operator {
    fn from(signing_key: SigningKey) -> Self {
        Self(Ed25519Signer::from(signing_key))
    }
}

impl std::fmt::Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.did())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_generates_operator_with_valid_did() {
        let operator = Operator::generate();
        let did = operator.did().to_string();
        assert!(did.starts_with("did:key:z"));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_creates_same_operator_from_same_signing_key() {
        let secret = [42u8; 32];
        let op1 = Operator::from(SigningKey::from_bytes(&secret));
        let op2 = Operator::from(SigningKey::from_bytes(&secret));
        assert_eq!(op1.did().to_string(), op2.did().to_string());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_creates_same_operator_from_same_passphrase() {
        let op1 = Operator::from_passphrase("test passphrase").await;
        let op2 = Operator::from_passphrase("test passphrase").await;
        assert_eq!(op1.did().to_string(), op2.did().to_string());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_roundtrips_through_secret_bytes() {
        let op1 = Operator::generate();
        let secret = op1.to_secret();
        let op2 = Operator::from(SigningKey::from_bytes(&secret));
        assert_eq!(op1.did().to_string(), op2.did().to_string());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_roundtrips_through_from_secret() {
        let op1 = Operator::generate();
        let secret = op1.to_secret();
        let op2 = Operator::from_secret(secret);
        assert_eq!(op1.did().to_string(), op2.did().to_string());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_converts_to_ed25519_signer() {
        let operator = Operator::generate();
        let did_before = operator.did().to_string();
        let signer: Ed25519Signer = operator.into();
        assert_eq!(Principal::did(&signer).to_string(), did_before);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_converts_to_credentials() {
        let operator = Operator::generate();
        let _credentials: Credentials = (&operator).into();
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn it_displays_as_did() {
        let operator = Operator::generate();
        let display = format!("{}", operator);
        assert!(display.starts_with("did:key:z"));
    }
}
