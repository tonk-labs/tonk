//! The service's own signing identity.
//!
//! Registration mints activation delegations issued by the service, so
//! the service holds an ed25519 keypair, derived from the
//! `SERVICE_SECRET_KEY` secret: 32 seed bytes, hex encoded. Invocations
//! address the key's `did:key` form today; the DID document served at
//! `/.well-known/did.json` publishes the same key under the host's
//! `did:web` name so the service DID can move there once resolution
//! support lands.

use dialog_credentials::Ed25519Signer;
use dialog_varsig::Principal;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

/// Build the service signer from its hex-encoded 32-byte seed.
pub fn signer_from_hex(seed_hex: &str) -> Result<Ed25519Signer, String> {
    let bytes = hex::decode(seed_hex.trim())
        .map_err(|err| format!("SERVICE_SECRET_KEY is not valid hex: {err}"))?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "SERVICE_SECRET_KEY must be 32 bytes of hex".to_string())?;
    Ok(Ed25519Signer::from(SigningKey::from_bytes(&seed)))
}

/// The DID document for `did:web:{host}`, carrying the service's ed25519
/// key as a `Multikey` verification method. The multibase key is the
/// `did:key` identifier's method-specific part, so the two names verify
/// against the same key.
pub fn did_document(host: &str, signer: &Ed25519Signer) -> Value {
    let id = format!("did:web:{host}");
    let did_key = signer.did().to_string();
    let multibase = did_key
        .strip_prefix("did:key:")
        .unwrap_or(&did_key)
        .to_string();
    let method = format!("{id}#{multibase}");
    json!({
        "@context": [
            "https://www.w3.org/ns/did/v1",
            "https://w3id.org/security/multikey/v1"
        ],
        "id": id,
        "verificationMethod": [{
            "id": method,
            "type": "Multikey",
            "controller": id,
            "publicKeyMultibase": multibase
        }],
        "authentication": [method],
        "assertionMethod": [method]
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_derives_a_stable_did_from_the_seed() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let again = signer_from_hex(&"11".repeat(32)).unwrap();
        assert_eq!(signer.did(), again.did());
        assert!(signer.did().to_string().starts_with("did:key:z6Mk"));
    }

    #[dialog_common::test]
    fn it_rejects_a_malformed_seed() {
        assert!(signer_from_hex("not hex").is_err());
        assert!(signer_from_hex("11").is_err());
    }

    #[dialog_common::test]
    fn it_documents_the_key_under_the_web_did() {
        let signer = signer_from_hex(&"11".repeat(32)).unwrap();
        let document = did_document("hub.tonk.xyz", &signer);
        assert_eq!(document["id"], "did:web:hub.tonk.xyz");
        let multibase = document["verificationMethod"][0]["publicKeyMultibase"]
            .as_str()
            .unwrap();
        assert_eq!(format!("did:key:{multibase}"), signer.did().to_string());
    }
}
