use ed25519_dalek::SigningKey;
#[cfg(not(target_arch = "wasm32"))]
use hkdf::Hkdf;
#[cfg(not(target_arch = "wasm32"))]
use sha2::Sha256;

pub struct Passphrase(String);

impl Passphrase {
    pub fn new(passphrase: String) -> Self {
        Passphrase(passphrase)
    }

    /// Derives an Ed25519 signing key from a passphrase using HKDF key
    /// derivation algorithm. The salt is empty, and the info string provides
    /// domain separation.
    pub async fn derive_signing_key(&self, info: Option<&[u8]>) -> SigningKey {
        let context = info.unwrap_or(HKDF_GENERIC_INFO);
        let passphrase = self.0.as_bytes();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let hk = Hkdf::<Sha256>::new(None, passphrase);
            let mut secret = [0u8; 32];
            hk.expand(context, &mut secret)
                .expect("32 bytes is a valid length for HKDF-SHA256");
            SigningKey::from_bytes(&secret)
        }
        #[cfg(target_arch = "wasm32")]
        {
            use js_sys::{global, Object, Reflect, Uint8Array};
            use wasm_bindgen::JsCast;
            use wasm_bindgen_futures::JsFuture;

            // Get crypto from globalThis - works in both window and worker contexts
            let global = global();
            let crypto: web_sys::Crypto = Reflect::get(&global, &"crypto".into())
                .expect("crypto should be available")
                .unchecked_into();
            let subtle = crypto.subtle();

            // Import the passphrase as raw key material
            let key_data = Uint8Array::from(passphrase);

            let import_promise = subtle
                .import_key_with_object(
                    "raw",
                    &key_data,
                    &{
                        let obj = Object::new();
                        Reflect::set(&obj, &"name".into(), &"HKDF".into()).unwrap();
                        obj
                    },
                    false,
                    &js_sys::Array::of1(&"deriveBits".into()),
                )
                .expect("import_key should succeed");

            let base_key: web_sys::CryptoKey = JsFuture::from(import_promise)
                .await
                .expect("import_key promise should resolve")
                .unchecked_into();

            // Derive 32 bytes using HKDF-SHA256
            let salt = Uint8Array::new_with_length(0);
            let info = Uint8Array::from(context);

            let params = web_sys::HkdfParams::new("HKDF", &"SHA-256".into(), &info, &salt);

            let derive_promise = subtle
                .derive_bits_with_object(&params, &base_key, 256)
                .expect("derive_bits should succeed");

            let derived: js_sys::ArrayBuffer = JsFuture::from(derive_promise)
                .await
                .expect("derive_bits promise should resolve")
                .unchecked_into();

            let derived_array = Uint8Array::new(&derived);
            let mut secret = [0u8; 32];
            derived_array.copy_to(&mut secret);

            SigningKey::from_bytes(&secret)
        }
    }
}

const HKDF_GENERIC_INFO: &[u8] = b"tonk passphrase v1";

impl From<String> for Passphrase {
    fn from(passphrase: String) -> Self {
        Passphrase(passphrase)
    }
}

impl From<&str> for Passphrase {
    fn from(passphrase: &str) -> Self {
        Passphrase(passphrase.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);

    /// Expected secret bytes for "test passphrase" with info "tonk passphrase v1".
    /// This value is used to verify that native (hkdf crate) and WASM (crypto.subtle)
    /// implementations produce identical results.
    const EXPECTED_SECRET: [u8; 32] = [
        203, 227, 143, 122, 44, 152, 14, 232, 79, 24, 120, 193, 105, 97, 226, 252, 1, 215, 12, 224,
        37, 226, 92, 71, 55, 52, 198, 84, 251, 72, 88, 239,
    ];

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_derives_same_key_from_same_passphrase() {
        let p1 = Passphrase::from("test passphrase");
        let p2 = Passphrase::from("test passphrase");
        let key1 = p1.derive_signing_key(None).await;
        let key2 = p2.derive_signing_key(None).await;
        assert_eq!(key1.to_bytes(), key2.to_bytes());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_derives_different_keys_from_different_passphrases() {
        let p1 = Passphrase::from("passphrase one");
        let p2 = Passphrase::from("passphrase two");
        let key1 = p1.derive_signing_key(None).await;
        let key2 = p2.derive_signing_key(None).await;
        assert_ne!(key1.to_bytes(), key2.to_bytes());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_derives_different_keys_with_different_info() {
        let passphrase = Passphrase::from("test passphrase");
        let key1 = passphrase.derive_signing_key(None).await;
        let key2 = Passphrase::from("test passphrase")
            .derive_signing_key(Some(b"custom info"))
            .await;
        assert_ne!(key1.to_bytes(), key2.to_bytes());
    }

    /// This test verifies that both native and WASM produce the exact same key.
    /// The expected value was computed using the native hkdf crate and must match
    /// what crypto.subtle produces in WASM.
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), tokio::test)]
    async fn it_produces_same_key_on_native_and_wasm() {
        let passphrase = Passphrase::from("test passphrase");
        let key = passphrase.derive_signing_key(None).await;
        assert_eq!(
            key.to_bytes(),
            EXPECTED_SECRET,
            "HKDF must produce the expected secret"
        );
    }
}
