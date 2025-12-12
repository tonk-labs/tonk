use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;

/// Represents an Ed25519 keypair
pub struct Keypair {
    signing_key: SigningKey,
}

impl Keypair {
    /// Generate a new random Ed25519 keypair
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        Self { signing_key }
    }

    /// Create a keypair from a 32-byte seed
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        Self { signing_key }
    }

    /// Get the signing key bytes (secret key)
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Get the verifying key (public key)
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Convert the public key to a did:key identifier
    /// Format: did:key:z<base58btc-multibase-encoded-multicodec-public-key>
    pub fn to_did_key(&self) -> String {
        let public_key = self.verifying_key();
        let public_key_bytes = public_key.as_bytes();

        // Multicodec prefix for Ed25519 public key: 0xed01
        let mut multicodec_key = vec![0xed, 0x01];
        multicodec_key.extend_from_slice(public_key_bytes);

        // Base58btc encode (multibase 'z' prefix)
        let encoded = bs58::encode(&multicodec_key).into_string();

        format!("did:key:z{}", encoded)
    }

    /// Sign a message with this keypair
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keypair_generation() {
        let keypair = Keypair::generate();
        let public_key = keypair.verifying_key();
        assert_eq!(public_key.as_bytes().len(), 32);
    }

    #[test]
    fn test_keypair_roundtrip() {
        let keypair1 = Keypair::generate();
        let bytes = keypair1.to_bytes();
        let keypair2 = Keypair::from_bytes(&bytes);

        assert_eq!(
            keypair1.verifying_key().as_bytes(),
            keypair2.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_did_key_format() {
        let keypair = Keypair::generate();
        let did = keypair.to_did_key();

        assert!(did.starts_with("did:key:z"));
        assert!(did.len() > 50); // Base58 encoded key should be fairly long
    }
}
