//! Root-key derivation from a passkey PRF output.

use hkdf::Hkdf;
use sha2_0_10::Sha256;
use zeroize::Zeroizing;

/// Versioned derivation context. Doubles as the WebAuthn PRF eval input
/// and the HKDF info string; bumping the version is a deliberate root
/// rotation, never a routine change.
pub const ROOT_KEY_CONTEXT: &[u8] = b"tonk/root-key/v1";

/// Derive the root Ed25519 seed from a passkey PRF output.
///
/// HKDF-SHA256 with no salt and [`ROOT_KEY_CONTEXT`] as info. The seed is
/// wiped when the returned guard drops — callers must not copy it out.
pub fn derive_root_seed(prf_output: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(None, prf_output);
    let mut seed = Zeroizing::new([0u8; 32]);
    hkdf.expand(ROOT_KEY_CONTEXT, seed.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    seed
}

#[cfg(test)]
mod tests {
    use super::derive_root_seed;

    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_pinned_seed_vector() {
        let seed = derive_root_seed(&[7u8; 32]);
        assert_eq!(
            hex::encode(seed.as_ref()),
            "365851595e28924dfab3007a2d043c063387e80308a612a27e40ab3c8cfdbb66",
        );
    }

    #[dialog_common::test]
    fn it_derives_deterministically() {
        assert_eq!(
            derive_root_seed(&[9u8; 32]).as_ref(),
            derive_root_seed(&[9u8; 32]).as_ref(),
        );
    }

    #[dialog_common::test]
    fn it_derives_distinct_seeds_from_distinct_prf_outputs() {
        assert_ne!(
            derive_root_seed(&[1u8; 32]).as_ref(),
            derive_root_seed(&[2u8; 32]).as_ref(),
        );
    }
}
