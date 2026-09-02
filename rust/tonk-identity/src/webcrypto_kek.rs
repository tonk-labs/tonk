//! Opening an envelope from a **non-extractable** KEK handle.
//!
//! The KEK is a 32-byte symmetric AES-256 key. The `aes_gcm` path in
//! [`crate::envelope`] needs those bytes (`Aes256Gcm::new_from_slice`).
//! A passkey ceremony necessarily returns its PRF outputs to the page;
//! the page posts those two fixed-length values as transient typed
//! arrays, and the worker immediately imports them as HKDF bases.
//!
//! WebCrypto offers a better shape. `deriveKey` produces a `CryptoKey`
//! that is born non-extractable — no raw KEK copy is ever materialised.
//! The worker retains only opaque handles after importing the PRF
//! transport bytes. Restricted to `["decrypt"]`, a leaked KEK handle
//! cannot even seal something new under that KEK.
//!
//! This module is the receiving half: given such a handle and an
//! envelope, unseal.
//!
//! ## Both operations, two capabilities
//!
//! Sealing gets a handle too, and for a reason independent of
//! transport: a KEK that is never materialised cannot leak. Deriving
//! bytes with `custody_kek(&prf)` puts 32 bytes in a buffer someone has
//! to be trusted to zero; `deriveKey` yields a key that was never
//! readable in the first place. That holds even where sealing happens
//! in the same document that ran the ceremony, with no boundary to
//! cross.
//!
//! | Operation | Usage | Derived by |
//! |---|---|---|
//! | seal (account creation, enrolling a passkey) | `["encrypt"]` | [`derive_custody_sealing_handle`] |
//! | open (unlock, in the worker) | `["decrypt"]` | [`derive_custody_kek_handle`] |
//!
//! Two handles rather than one key with both usages. An opener that
//! cannot forge is the entire value of what the worker receives, and a
//! key carrying `encrypt` is not that. The capability is a type
//! parameter on [`Kek`], so the two cannot be confused: `Kek<C,
//! Opening>` has no `seal_seed` method at all.
//!
//! Both produce the ordinary wire format, so a handle-sealed envelope
//! opens through `aes_gcm` like any other — pinned by
//! `it_seals_through_a_handle_into_the_ordinary_wire_format`.
//!
//! ## Why the byte path stays
//!
//! Native targets (the CLI, native tests) have no `crypto.subtle` at
//! all, and `KekMethod::Phrase` derives from a recovery phrase through
//! Argon2id rather than from a passkey — reserved today, but not a
//! passkey ceremony when it lands.
//!
//! So this is an addition, never a migration.
//!
//! ## Deriving the handle (the page's half)
//!
//! ```js
//! const prfKey = await crypto.subtle.importKey(
//!   "raw", prfOutput, "HKDF", false, ["deriveKey"],
//! );
//! const kek = await crypto.subtle.deriveKey(
//!   { name: "HKDF", hash: "SHA-256",
//!     salt: new Uint8Array(32),      // or an empty array; see below
//!     info: CUSTODY_KEK_CONTEXT },
//!   prfKey,
//!   { name: "AES-GCM", length: 256 },
//!   false,                            // non-extractable
//!   ["decrypt"],                      // open only, never seal
//! );
//! ```
//!
//! **On the salt.** Rust derives with `Hkdf::new(None, ikm)`. RFC 5869
//! says an absent salt is set to `HashLen` zeros, and WebCrypto — which
//! has no `None` — normalises an empty salt to the same thing. So an
//! empty array and `new Uint8Array(32)` both reproduce the Rust key.
//! Pinned by `it_matches_hkdf_with_no_salt`, because a mismatch here
//! would show up only as an envelope that refuses to open.

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AesGcmParams, CryptoKey};
use zeroize::Zeroizing;

use crate::clearance::Clearance;
use crate::envelope::capability::Capability;
use crate::envelope::capability::{Opening, Sealing};
use crate::envelope::{Envelope, EnvelopeError, Kek, KekMethod};

/// Unseal `envelope` using a non-extractable AES-GCM key handle.
///
/// The handle must carry the `decrypt` usage and be the same KEK the
/// envelope was sealed under; anything else fails the AEAD tag check
/// and comes back as [`EnvelopeError::Sealed`], which is deliberately
/// indistinguishable from a wrong key.
pub async fn open_with_handle<C: Clearance>(
    kek: &CryptoKey,
    envelope: &Envelope<C>,
) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
    let subtle = subtle().map_err(|_| EnvelopeError::Sealed)?;

    // `additionalData` is not optional: the header is bound into the
    // AEAD, so omitting it fails the tag check exactly as a wrong key
    // would.
    let nonce = js_sys::Uint8Array::from(&envelope.nonce()[..]);
    let params = AesGcmParams::new("AES-GCM", &nonce);
    let aad = js_sys::Uint8Array::from(&envelope.aad()[..]);
    params.set_additional_data(&aad);

    let ciphertext = js_sys::Uint8Array::from(envelope.ciphertext());
    let promise = subtle
        .decrypt_with_object_and_buffer_source(&params, kek, &ciphertext)
        .map_err(|_| EnvelopeError::Sealed)?;
    let plaintext: JsValue = JsFuture::from(promise)
        .await
        .map_err(|_| EnvelopeError::Sealed)?;

    let plaintext = js_sys::Uint8Array::new(&plaintext);
    if plaintext.length() != 32 {
        return Err(EnvelopeError::Sealed);
    }
    let mut opened = Zeroizing::new([0u8; 32]);
    plaintext.copy_to(opened.as_mut());
    Ok(opened)
}

/// Seal a 32-byte seed with a KEK, whichever way its key is held.
///
/// The sealing counterpart to [`open_seed`]. A bytes-backed KEK goes
/// through `aes_gcm`; a handle-backed one through WebCrypto, so no raw
/// KEK is materialised at any point.
pub async fn seal_seed<C: Clearance>(
    kek: &Kek<C, Sealing>,
    seed: &Zeroizing<[u8; 32]>,
    method: KekMethod,
) -> anyhow::Result<Envelope<C>> {
    match kek.handle() {
        Some(handle) => seal_with_handle(handle, seed, method).await,
        None => kek.seal_seed(seed, method),
    }
}

/// Seal with a non-extractable AES-GCM key handle carrying `encrypt`.
///
/// Produces the same wire format the `aes_gcm` path does — same header
/// as associated data, same 12-byte nonce — so an envelope sealed here
/// opens through either path.
pub async fn seal_with_handle<C: Clearance>(
    kek: &CryptoKey,
    seed: &Zeroizing<[u8; 32]>,
    method: KekMethod,
) -> anyhow::Result<Envelope<C>> {
    let subtle = subtle().map_err(|_| anyhow::anyhow!("no crypto.subtle in this scope"))?;

    let mut nonce = [0u8; crate::envelope::NONCE_LEN];
    getrandom::fill(&mut nonce)
        .map_err(|error| anyhow::anyhow!("no entropy for an envelope nonce: {error}"))?;

    let generation = 0;
    let aad = Envelope::<C>::header_for(generation, method);
    let params = AesGcmParams::new("AES-GCM", &js_sys::Uint8Array::from(&nonce[..]));
    params.set_additional_data(&js_sys::Uint8Array::from(&aad[..]));

    let plaintext = js_sys::Uint8Array::from(&seed[..]);
    let promise = subtle
        .encrypt_with_object_and_buffer_source(&params, kek, &plaintext)
        .map_err(|_| anyhow::anyhow!("the sealing handle refused to encrypt"))?;
    let sealed = JsFuture::from(promise)
        .await
        .map_err(|_| anyhow::anyhow!("sealing did not complete"))?;

    let sealed = js_sys::Uint8Array::new(&sealed);
    let mut ciphertext = vec![0u8; sealed.length() as usize];
    sealed.copy_to(&mut ciphertext);
    Ok(Envelope::from_parts(generation, method, nonce, ciphertext))
}

/// Open an envelope with a KEK, whichever way its key is held.
///
/// The single entry point callers should reach for on wasm: a
/// bytes-backed KEK goes through `aes_gcm` synchronously, a
/// handle-backed one through WebCrypto. Nothing outside this module
/// branches on which.
pub async fn open_seed<C: Clearance, K: Capability>(
    kek: &Kek<C, K>,
    envelope: &Envelope<C>,
) -> Result<Zeroizing<[u8; 32]>, EnvelopeError> {
    match kek.handle() {
        Some(handle) => open_with_handle(handle, envelope).await,
        None => kek.open_seed_bytes(envelope),
    }
}

/// Derive the custody KEK from a PRF output as a **non-extractable**
/// WebCrypto key.
///
/// Mirrors [`crate::envelope::custody_kek`] exactly —
/// HKDF-SHA256, no salt, expanded at
/// [`crate::envelope::CUSTODY_KEK_CONTEXT`] — but the result is a
/// `CryptoKey` rather than 32 bytes, so:
///
/// - no raw KEK is ever materialised in JS or Rust to be zeroized;
///   `deriveKey` (not `deriveBits`) produces it already sealed
/// - the resulting handle stays in the realm that imported it and
///   cannot be read
/// - it carries only the `decrypt` usage, so a leaked handle can open
///   envelopes and can never seal a new one under this KEK
///
/// The caller still holds the PRF output that produced it. That is
/// unavoidable — the ceremony returns bytes — but its lifetime ends
/// here, and nothing downstream needs it.
pub async fn derive_custody_kek_handle(prf_output: &[u8; 32]) -> Result<CryptoKey, JsValue> {
    derive_custody_kek_for(prf_output, "decrypt").await
}

/// Derive the custody KEK as a **sealing** handle.
///
/// The same key, with `["encrypt"]` instead of `["decrypt"]`. Used
/// where an account secret is wrapped under a new passkey — account
/// creation, and enrolling another passkey.
///
/// The point is not transport but that **no raw KEK is ever
/// materialised**. `custody_kek(&prf)`
/// produces 32 bytes in a buffer that must be trusted to zero;
/// `deriveKey` produces a key that was never readable to begin with.
///
/// Kept separate from [`derive_custody_kek_handle`] rather than
/// deriving one key with both usages: an opener that cannot forge is
/// the entire value of the worker-owned handle, and a key with `encrypt`
/// in its usages is not that.
pub async fn derive_custody_sealing_handle(prf_output: &[u8; 32]) -> Result<CryptoKey, JsValue> {
    derive_custody_kek_for(prf_output, "encrypt").await
}

/// Shared derivation: HKDF-SHA256 at [`CUSTODY_KEK_CONTEXT`], with one
/// usage. Both entry points differ only in that usage.
///
/// [`CUSTODY_KEK_CONTEXT`]: crate::envelope::CUSTODY_KEK_CONTEXT
async fn derive_custody_kek_for(prf_output: &[u8; 32], usage: &str) -> Result<CryptoKey, JsValue> {
    let subtle = subtle()?;

    // The PRF output enters as HKDF key material and is immediately
    // unreachable: imported non-extractable, usable only to derive.
    let material = js_sys::Uint8Array::from(&prf_output[..]);
    let derive_only = js_sys::Array::new();
    derive_only.push(&JsValue::from_str("deriveKey"));
    let base: CryptoKey = JsFuture::from(subtle.import_key_with_str(
        "raw",
        &material,
        "HKDF",
        false,
        &derive_only,
    )?)
    .await?
    .dyn_into()?;

    // RFC 5869: an absent salt is `HashLen` zeros. Rust says that with
    // `Hkdf::new(None, ..)`; WebCrypto has no `None`, and normalises an
    // empty salt to the same thing. Either spelling reproduces the Rust
    // key — pinned by `it_matches_hkdf_with_no_salt`.
    let params = js_sys::Object::new();
    let set = |key: &str, value: &JsValue| {
        js_sys::Reflect::set(&params, &JsValue::from_str(key), value)
            .expect("setting a property on a fresh object cannot fail");
    };
    set("name", &JsValue::from_str("HKDF"));
    set("hash", &JsValue::from_str("SHA-256"));
    set("salt", js_sys::Uint8Array::new_with_length(32).as_ref());
    set(
        "info",
        js_sys::Uint8Array::from(crate::envelope::CUSTODY_KEK_CONTEXT).as_ref(),
    );

    let derived = js_sys::Object::new();
    js_sys::Reflect::set(&derived, &"name".into(), &"AES-GCM".into())?;
    js_sys::Reflect::set(&derived, &"length".into(), &JsValue::from_f64(256.0))?;

    let usages = js_sys::Array::new();
    usages.push(&JsValue::from_str(usage));

    JsFuture::from(
        subtle.derive_key_with_object_and_object(&params, &base, &derived, false, &usages)?,
    )
    .await?
    .dyn_into()
}

/// Import a PRF output as an HKDF base the worker can derive from.
///
/// The worker calls this after receiving one transient typed array.
/// Non-extractable and HKDF-only, the resulting handle derives at
/// contexts the holder chooses and never yields the output that made it.
///
/// Both usages, because the two derivations need different ones: the
/// KEK is a `deriveKey` target, and the custody seed can only be
/// `deriveBits` — WebCrypto has no Ed25519 for `deriveKey`.
///
/// The caller keeps the copied PRF output in zeroizing Rust storage for
/// the duration of this import only.
pub async fn import_derivation_base(prf_output: &[u8; 32]) -> Result<CryptoKey, JsValue> {
    let subtle = subtle()?;
    let material = js_sys::Uint8Array::from(&prf_output[..]);
    let usages = js_sys::Array::new();
    usages.push(&JsValue::from_str("deriveKey"));
    usages.push(&JsValue::from_str("deriveBits"));
    JsFuture::from(subtle.import_key_with_str("raw", &material, "HKDF", false, &usages)?)
        .await?
        .dyn_into()
}

/// The HKDF parameters both derivations share, differing only in `info`.
///
/// RFC 5869: an absent salt is `HashLen` zeros. Rust says that with
/// `Hkdf::new(None, ..)`; WebCrypto has no `None` and normalises an
/// empty salt to the same thing, so either spelling reproduces the byte
/// path.
fn hkdf_params(info: &[u8]) -> js_sys::Object {
    let params = js_sys::Object::new();
    let set = |key: &str, value: &JsValue| {
        js_sys::Reflect::set(&params, &JsValue::from_str(key), value)
            .expect("setting a property on a fresh object cannot fail");
    };
    set("name", &JsValue::from_str("HKDF"));
    set("hash", &JsValue::from_str("SHA-256"));
    set("salt", js_sys::Uint8Array::new_with_length(32).as_ref());
    set("info", js_sys::Uint8Array::from(info).as_ref());
    params
}

/// Derive the custody signing seed from a base handle.
///
/// `deriveBits` rather than `deriveKey`: WebCrypto cannot produce an
/// Ed25519 key by derivation, so the seed arrives as bytes and is
/// imported. Those bytes exist wherever this runs — the worker, under
/// this design — and X25519 derivation needs them anyway, so a handle
/// would buy nothing even if one were available.
pub async fn derive_custody_seed(base: &CryptoKey) -> Result<Zeroizing<[u8; 32]>, JsValue> {
    let bits = JsFuture::from(subtle()?.derive_bits_with_object(
        &hkdf_params(crate::envelope::CUSTODY_KEY_CONTEXT),
        base,
        256,
    )?)
    .await?;
    let mut seed = Zeroizing::new([0u8; 32]);
    js_sys::Uint8Array::new(&bits).copy_to(seed.as_mut());
    Ok(seed)
}

/// Derive the custody KEK from a base handle, as an opener.
///
/// The counterpart to [`derive_custody_kek_handle`], which takes the PRF
/// output directly. This takes the handle that wraps it, so the bytes
/// never crossed.
pub async fn derive_custody_kek_from_base(base: &CryptoKey) -> Result<CryptoKey, JsValue> {
    derive_kek_from_base(base, "decrypt").await
}

/// The same, as a sealer. See [`derive_custody_sealing_handle`].
pub async fn derive_custody_sealer_from_base(base: &CryptoKey) -> Result<CryptoKey, JsValue> {
    derive_kek_from_base(base, "encrypt").await
}

async fn derive_kek_from_base(base: &CryptoKey, usage: &str) -> Result<CryptoKey, JsValue> {
    let derived = js_sys::Object::new();
    js_sys::Reflect::set(&derived, &"name".into(), &"AES-GCM".into())?;
    js_sys::Reflect::set(&derived, &"length".into(), &JsValue::from_f64(256.0))?;
    let usages = js_sys::Array::new();
    usages.push(&JsValue::from_str(usage));
    JsFuture::from(subtle()?.derive_key_with_object_and_object(
        &hkdf_params(crate::envelope::CUSTODY_KEK_CONTEXT),
        base,
        &derived,
        false,
        &usages,
    )?)
    .await?
    .dyn_into()
}

/// A passkey's two worker-owned derivation handles, held together.
///
/// The authenticator evaluates its PRF at two salts and the pair is
/// meaningless split: one names the custody space, the other opens what
/// is stored there. Holding them as one value keeps a caller from
/// deriving a seed under a KEK handle, which would compile and produce
/// a plausible-looking key for a space that does not exist.
///
/// Neither handle yields bytes. The worker imports both from the
/// transient PRF byte envelope and derives from them for one custody
/// request.
#[derive(Clone)]
pub struct Custodian {
    /// The credential this came from, so a later assertion can name it.
    pub credential_id: Vec<u8>,
    /// PRF at [`CUSTODY_KEY_CONTEXT`]: derives the custody signer.
    key: CryptoKey,
    /// PRF at [`CUSTODY_KEK_CONTEXT`]: derives the wrapping KEK.
    kek: CryptoKey,
}

impl Custodian {
    /// Import a pair of PRF outputs as worker-owned handles.
    ///
    /// The caller keeps the raw values in zeroizing arrays and drops
    /// them immediately after this import completes.
    pub async fn adopt(
        credential_id: Vec<u8>,
        key: &[u8; 32],
        kek: &[u8; 32],
    ) -> Result<Self, JsValue> {
        Ok(Self {
            credential_id,
            key: import_derivation_base(key).await?,
            kek: import_derivation_base(kek).await?,
        })
    }

    /// The custody signing seed, for [`crate::envelope::custody_signer`].
    pub async fn seed(&self) -> Result<Zeroizing<[u8; 32]>, JsValue> {
        derive_custody_seed(&self.key).await
    }

    /// The signer that names this passkey's custody space.
    ///
    /// Derived through the seed, so the bytes exist for the length of
    /// this call — WebCrypto cannot derive an Ed25519 key, and X25519
    /// needs the seed anyway.
    pub async fn signer(&self) -> anyhow::Result<dialog_credentials::Signer> {
        let seed = self
            .seed()
            .await
            .map_err(|error| anyhow::anyhow!("deriving the custody seed failed: {error:?}"))?;
        dialog_credentials::Ed25519Signer::import(&*seed)
            .await
            .map(dialog_credentials::Signer::from)
            .map_err(|error| anyhow::anyhow!("importing the custody seed failed: {error}"))
    }

    /// A KEK that can open this passkey's envelopes and not seal one.
    pub async fn opener(&self) -> Result<Kek<crate::clearance::Recovery, Opening>, JsValue> {
        Ok(Kek::from_handle(
            derive_custody_kek_from_base(&self.kek).await?,
        ))
    }

    /// A KEK that can seal under this passkey. Wanted where an account
    /// secret is wrapped: creation, and enrolling another passkey.
    pub async fn sealer(&self) -> Result<Kek<crate::clearance::Recovery, Sealing>, JsValue> {
        Ok(Kek::from_sealing_handle(
            derive_custody_sealer_from_base(&self.kek).await?,
        ))
    }
}

/// Reaching the account a [`Custodian`] holds.
///
/// `crypto.subtle` from either a window or a service worker.
fn subtle() -> Result<web_sys::SubtleCrypto, JsValue> {
    web_sys::window()
        .map(|window| window.crypto())
        .or_else(|| {
            js_sys::global()
                .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
                .ok()
                .map(|scope| scope.crypto())
        })
        .and_then(Result::ok)
        .map(|crypto| crypto.subtle())
        .ok_or_else(|| JsValue::from_str("no crypto.subtle in this scope"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Crypto;
    use crate::clearance::Recovery;
    use crate::envelope::capability::Opening;
    use crate::envelope::{Kek, KekMethod};
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    /// Import raw bytes as a non-extractable AES-GCM decrypt-only key,
    /// standing in for what the page's `deriveKey` produces.
    async fn handle_for(bytes: &[u8; 32]) -> CryptoKey {
        let subtle = web_sys::window().unwrap().crypto().unwrap().subtle();
        let raw = js_sys::Uint8Array::from(&bytes[..]);
        let usages = js_sys::Array::new();
        usages.push(&JsValue::from_str("decrypt"));
        let promise = subtle
            .import_key_with_str("raw", &raw, "AES-GCM", false, &usages)
            .expect("import starts");
        JsFuture::from(promise)
            .await
            .expect("import resolves")
            .dyn_into()
            .expect("a CryptoKey")
    }

    /// A PRF output crosses structured clone as a typed array, then the
    /// receiver imports a non-extractable HKDF handle and derives the
    /// same custody seed as the byte path.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_imports_posted_prf_bytes_as_a_derivation_handle() {
        let prf = [7u8; 32];
        let posted = js_sys::Uint8Array::from(&prf[..]);
        let cloned: js_sys::Uint8Array = js_sys::Reflect::get(
            &web_sys::window().unwrap(),
            &JsValue::from_str("structuredClone"),
        )
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .expect("structuredClone exists")
        .call1(&JsValue::NULL, &posted)
        .expect("a typed array survives structured clone")
        .dyn_into()
        .expect("and arrives as a Uint8Array");
        let mut received = Zeroizing::new([0u8; 32]);
        cloned.copy_to(received.as_mut());
        cloned.fill(0, 0, cloned.length());
        let base = import_derivation_base(&received)
            .await
            .expect("HKDF import");
        assert!(!base.extractable(), "the worker retains no readable base");
        let seed = derive_custody_seed(&base).await.expect("seed derives");
        assert_eq!(
            seed.as_ref(),
            crate::envelope::custody_seed(&prf).as_ref(),
            "the worker derives the seed the page would have"
        );
    }

    /// Both posted PRF values import to handles that reproduce the byte
    /// path's custody signer and KEK.
    ///
    /// This is the contract the worker design rests on: the page posts
    /// two PRF outputs, the worker imports two HKDF bases and derives the
    /// custody seed from one and the KEK from the other. Both name the
    /// same custody space and open the same envelopes as today. A drift
    /// in either would strand every account whose passkey is the only
    /// way back.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_derives_both_custody_keys_from_posted_bytes() {
        // Two independent PRF outputs, as the authenticator returns:
        // one salted at the key context, one at the KEK context.
        let key_prf = [11u8; 32];
        let kek_prf = [22u8; 32];

        let key_base = import_derivation_base(&key_prf).await.expect("key base");
        let kek_base = import_derivation_base(&kek_prf).await.expect("kek base");

        // The seed names the custody space, so it must be the one the
        // byte path derives or the DID moves.
        let seed = derive_custody_seed(&key_base).await.expect("seed derives");
        assert_eq!(
            seed.as_ref(),
            crate::envelope::custody_seed(&key_prf).as_ref(),
            "the worker derives the seed the page would have"
        );

        // And the KEK must open what the byte path sealed, or every
        // published envelope becomes unreadable.
        let secret = Zeroizing::new([42u8; 32]);
        let envelope = crate::envelope::custody_kek(&kek_prf)
            .seal_seed(&secret, KekMethod::Passkey)
            .expect("the byte path seals");
        let opened = open_with_handle(
            &derive_custody_kek_from_base(&kek_base)
                .await
                .expect("kek derives"),
            &envelope,
        )
        .await
        .expect("the derived KEK opens what the byte path sealed");
        assert_eq!(&*opened, &*secret, "and the same secret comes back");
    }

    /// A `Custodian` rebuilt from posted bytes derives what the byte path
    /// derives — the same seed, and a KEK that opens the same envelopes.
    ///
    /// The same contract as the test above, through the type a caller
    /// actually holds. Worth pinning separately because the pair being
    /// kept together is the point: a seed derived from the KEK handle
    /// would compile and name a space that does not exist.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_rebuilds_a_custodian_from_posted_bytes() {
        let key_prf = [11u8; 32];
        let kek_prf = [22u8; 32];
        let received = Custodian::adopt(vec![1, 2, 3], &key_prf, &kek_prf)
            .await
            .expect("the worker imports both handles");

        assert_eq!(
            received.seed().await.expect("seed derives").as_ref(),
            crate::envelope::custody_seed(&key_prf).as_ref(),
            "the worker names the same custody space"
        );

        let secret = Zeroizing::new([42u8; 32]);
        let envelope = crate::envelope::custody_kek(&kek_prf)
            .seal_seed(&secret, KekMethod::Passkey)
            .expect("the byte path seals");
        let opener = received.opener().await.expect("opener derives");
        let opened = open_seed(&opener, &envelope)
            .await
            .expect("and opens what the byte path sealed");
        assert_eq!(&*opened, &*secret);
    }

    /// Create an account under a passkey, then load it back from the
    /// envelope alone — which is what a second device does.
    ///
    /// The secret never leaves this call on either side: `create`
    /// generates and seals it, `load` opens it, and between them only
    /// the envelope exists. That is the property that makes it safe to
    /// do this in the worker.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_creates_an_account_and_loads_it_back() {
        use dialog_varsig::Principal as _;

        let custodian = Custodian::adopt(vec![1], &[11u8; 32], &[22u8; 32])
            .await
            .expect("handles import");

        let created = crate::custodian::Custodian::Passkey(custodian.clone())
            .account()
            .create()
            .perform(&Crypto)
            .await
            .expect("an account");
        let signer = created.signer().await.expect("a signer");

        // Only the envelope crosses. A device with the same passkey and
        // nothing else gets the same account back.
        let recovered = crate::custodian::Custodian::Passkey(custodian.clone())
            .account()
            .import(created.envelope().clone())
            .perform(&Crypto)
            .await
            .expect("the envelope opens under the same passkey");
        assert_eq!(
            recovered.signer().await.expect("a signer").did(),
            signer.did(),
            "the same account comes back"
        );
    }

    /// The signer a `Custodian` derives names the same space the byte
    /// path does.
    ///
    /// This is what `load` resolves against, so a drift here would send
    /// a recovering device to an empty space rather than failing.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_names_the_same_custody_space_as_the_byte_path() {
        use dialog_varsig::Principal as _;

        let prf = [11u8; 32];
        let custodian = Custodian::adopt(vec![1], &prf, &[22u8; 32])
            .await
            .expect("handles import");

        assert_eq!(
            custodian.signer().await.expect("a signer").did(),
            crate::envelope::custody_signer(&prf)
                .await
                .expect("the byte path signer")
                .did(),
            "a recovering device resolves the space its cell is in"
        );
    }

    /// An envelope sealed under one passkey does not open under another.
    ///
    /// The failure this design must have: a wrong custodian is refused
    /// by the tag check rather than yielding something plausible.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn it_refuses_an_account_sealed_under_another_passkey() {
        let mine = Custodian::adopt(vec![1], &[11u8; 32], &[22u8; 32])
            .await
            .expect("handles import");
        let theirs = Custodian::adopt(vec![2], &[33u8; 32], &[44u8; 32])
            .await
            .expect("handles import");

        let account = crate::custodian::Custodian::Passkey(mine.clone())
            .account()
            .create()
            .perform(&Crypto)
            .await
            .expect("an account");
        assert!(
            crate::custodian::Custodian::Passkey(theirs.clone())
                .account()
                .import(account.envelope().clone())
                .perform(&Crypto)
                .await
                .is_err(),
            "another passkey does not open it"
        );
    }

    /// The whole point: something sealed by the `aes_gcm` path opens
    /// from a key handle that cannot be read.
    #[dialog_common::test]
    async fn it_opens_an_envelope_sealed_by_the_byte_path() {
        let bytes = Zeroizing::new([7u8; 32]);
        let kek = Kek::<Recovery>::from_bytes(bytes.clone());
        let secret = Zeroizing::new([42u8; 32]);
        let envelope = kek
            .seal_seed(&secret, KekMethod::Passkey)
            .expect("sealing works");

        let handle = handle_for(&bytes).await;
        let opened = open_with_handle(&handle, &envelope)
            .await
            .expect("the handle opens what the bytes sealed");
        assert_eq!(&*opened, &*secret, "the same secret comes back");
    }

    /// A different key fails the tag check rather than returning
    /// something wrong.
    #[dialog_common::test]
    async fn it_refuses_a_handle_for_another_key() {
        let kek = Kek::<Recovery>::from_bytes(Zeroizing::new([7u8; 32]));
        let envelope = kek
            .seal_seed(&Zeroizing::new([42u8; 32]), KekMethod::Passkey)
            .expect("sealing works");

        let wrong = handle_for(&[8u8; 32]).await;
        assert!(
            open_with_handle(&wrong, &envelope).await.is_err(),
            "a wrong key must fail, not decrypt to garbage",
        );
    }

    /// The two halves meet: a KEK handle derived in the page from a PRF
    /// output opens an envelope sealed by `custody_kek` from the same
    /// output.
    ///
    /// This is the end-to-end claim. `derive_custody_kek_handle` must
    /// reproduce `custody_kek` bit for bit — same HKDF, same absent
    /// salt, same context — or the envelope simply will not open, with
    /// no other symptom to debug from.
    #[dialog_common::test]
    async fn it_derives_a_handle_that_opens_what_custody_kek_sealed() {
        use crate::envelope::custody_kek;

        let prf = [11u8; 32];
        let sealed = custody_kek(&prf)
            .seal_seed(&Zeroizing::new([99u8; 32]), KekMethod::Passkey)
            .expect("sealing works");

        let handle = super::derive_custody_kek_handle(&prf)
            .await
            .expect("the page derives a handle");
        let opened = open_with_handle(&handle, &sealed)
            .await
            .expect("the derived handle opens the custody envelope");
        assert_eq!(&*opened, &[99u8; 32], "the sealed seed comes back");
    }

    /// The handle cannot be read, and cannot seal.
    ///
    /// Both are what make it safe to post to the worker: the bytes
    /// never cross, and a leaked handle is an opener rather than a
    /// forger.
    #[dialog_common::test]
    async fn it_derives_a_handle_that_is_neither_readable_nor_a_sealer() {
        let handle = super::derive_custody_kek_handle(&[11u8; 32])
            .await
            .expect("the page derives a handle");

        assert!(!handle.extractable(), "the KEK must not be readable");

        let usages = js_sys::Array::from(&handle.usages());
        let usages: Vec<String> = usages.iter().filter_map(|u| u.as_string()).collect();
        assert_eq!(
            usages,
            vec!["decrypt".to_owned()],
            "decrypt only: a leaked handle must not be able to seal",
        );

        // And the browser enforces it.
        let subtle = web_sys::window().unwrap().crypto().unwrap().subtle();
        let raw = js_sys::Uint8Array::new_with_length(32);
        assert!(
            subtle.export_key("raw", &handle).is_err()
                || JsFuture::from(subtle.export_key("raw", &handle).unwrap())
                    .await
                    .is_err(),
            "exporting a non-extractable key must fail",
        );
        // `encrypt` rejects its promise rather than throwing, so the
        // refusal only shows up once awaited.
        let nonce = js_sys::Uint8Array::new_with_length(12);
        let params = AesGcmParams::new("AES-GCM", &nonce);
        let refused = match subtle.encrypt_with_object_and_buffer_source(&params, &handle, &raw) {
            Err(_) => true,
            Ok(promise) => JsFuture::from(promise).await.is_err(),
        };
        assert!(refused, "a decrypt-only key must not encrypt");
    }

    /// A handle-sealed envelope is indistinguishable from a
    /// bytes-sealed one.
    ///
    /// The point of sealing through a handle is that no raw KEK is ever
    /// materialised — not a different wire format. So an envelope
    /// sealed by the handle must open through the ordinary `aes_gcm`
    /// path, which is what every other reader (native, the CLI) uses.
    #[dialog_common::test]
    async fn it_seals_through_a_handle_into_the_ordinary_wire_format() {
        use crate::envelope::custody_kek;

        let prf = [17u8; 32];
        let seed = Zeroizing::new([88u8; 32]);

        let sealing = super::derive_custody_sealing_handle(&prf)
            .await
            .expect("the page derives a sealing handle");
        let sealer =
            Kek::<Recovery, crate::envelope::capability::Sealing>::from_sealing_handle(sealing);
        let envelope = super::seal_seed(&sealer, &seed, KekMethod::Passkey)
            .await
            .expect("the handle seals");

        // Opened by the byte path, which knows nothing about handles.
        let opened = custody_kek(&prf)
            .open_seed(&envelope)
            .expect("the ordinary path opens what the handle sealed");
        assert_eq!(&*opened, &*seed, "same seed, same wire format");
    }

    /// And the reverse: a sealing handle cannot open.
    ///
    /// `encrypt` and `decrypt` are separate usages, so the sealer is
    /// not quietly a general-purpose key.
    #[dialog_common::test]
    async fn it_derives_a_sealer_that_cannot_open() {
        let handle = super::derive_custody_sealing_handle(&[17u8; 32])
            .await
            .expect("the page derives a sealing handle");

        let usages = js_sys::Array::from(&handle.usages());
        let usages: Vec<String> = usages.iter().filter_map(|u| u.as_string()).collect();
        assert_eq!(
            usages,
            vec!["encrypt".to_owned()],
            "encrypt only: sealing and opening stay separate capabilities",
        );
        assert!(!handle.extractable(), "the sealer must not be readable");
    }

    /// The unified opener works whichever way the key is held.
    ///
    /// Callers get one function; the dispatch on `Material` stays
    /// inside. This is the point of the enum — the same code path
    /// serves a native-shaped bytes KEK and a browser handle.
    #[dialog_common::test]
    async fn it_opens_through_one_entry_point_for_both_backings() {
        use crate::envelope::custody_kek;

        let prf = [13u8; 32];
        let sealed = custody_kek(&prf)
            .seal_seed(&Zeroizing::new([77u8; 32]), KekMethod::Passkey)
            .expect("sealing works");

        // Bytes-backed.
        let from_bytes = super::open_seed(&custody_kek(&prf), &sealed)
            .await
            .expect("bytes open");

        // Handle-backed, same envelope.
        let handle = super::derive_custody_kek_handle(&prf).await.unwrap();
        let opener = Kek::<Recovery, Opening>::from_handle(handle);
        let from_handle = super::open_seed(&opener, &sealed)
            .await
            .expect("the handle opens");

        assert_eq!(&*from_bytes, &*from_handle, "both reach the same seed");
    }

    /// An opener has no `seal_seed` at all.
    ///
    /// Compile-tested rather than asserted: the lines below do not
    /// compile if the capability parameter stops gating sealing, which
    /// is the guarantee worth keeping.
    ///
    /// ```compile_fail
    /// # use tonk_identity::clearance::Recovery;
    /// # use tonk_identity::envelope::{Kek, KekMethod, capability::Opening};
    /// # use zeroize::Zeroizing;
    /// fn wants_a_sealer(kek: &Kek<Recovery, Opening>) {
    ///     // no such method on an opener
    ///     let _ = kek.seal_seed(&Zeroizing::new([0u8; 32]), KekMethod::Passkey);
    /// }
    /// ```
    #[dialog_common::test]
    async fn it_gates_sealing_at_the_type_level() {
        // The positive half runs: a sealer seals.
        let sealer = Kek::<Recovery, crate::envelope::capability::Sealing>::from_bytes(
            Zeroizing::new([5u8; 32]),
        );
        assert!(
            sealer
                .seal_seed(&Zeroizing::new([6u8; 32]), KekMethod::Passkey)
                .is_ok(),
            "a sealing KEK seals",
        );
    }

    /// WebCrypto's HKDF agrees with `Hkdf::new(None, ikm)`, under
    /// either spelling of "no salt".
    ///
    /// RFC 5869: an absent salt is set to `HashLen` zeros. Rust says
    /// that with `None`; WebCrypto has no `None`, and normalises an
    /// EMPTY salt to the same thing. So both an empty array and 32 zero
    /// bytes derive the key Rust derives — worth pinning, because it is
    /// the one place a silent mismatch would produce an envelope that
    /// simply refuses to open with no other symptom.
    #[dialog_common::test]
    async fn it_matches_hkdf_with_no_salt() {
        let subtle = web_sys::window().unwrap().crypto().unwrap().subtle();
        let ikm = [3u8; 32];
        let info = b"tonk/kek/recovery/v1";

        let derive = |salt: Vec<u8>| {
            let subtle = subtle.clone();
            let info = info.to_vec();
            async move {
                let raw = js_sys::Uint8Array::from(&ikm[..]);
                let usages = js_sys::Array::new();
                usages.push(&JsValue::from_str("deriveBits"));
                let base: CryptoKey = JsFuture::from(
                    subtle
                        .import_key_with_str("raw", &raw, "HKDF", false, &usages)
                        .unwrap(),
                )
                .await
                .unwrap()
                .dyn_into()
                .unwrap();
                // `web_sys` has no HkdfParams binding here, so build
                // the dictionary directly.
                let params = js_sys::Object::new();
                let set = |k: &str, v: &JsValue| {
                    js_sys::Reflect::set(&params, &JsValue::from_str(k), v).unwrap();
                };
                set("name", &JsValue::from_str("HKDF"));
                set("hash", &JsValue::from_str("SHA-256"));
                set("info", js_sys::Uint8Array::from(&info[..]).as_ref());
                set("salt", js_sys::Uint8Array::from(&salt[..]).as_ref());
                let bits =
                    JsFuture::from(subtle.derive_bits_with_object(&params, &base, 256).unwrap())
                        .await
                        .unwrap();
                let out = js_sys::Uint8Array::new(&bits);
                let mut bytes = [0u8; 32];
                out.copy_to(&mut bytes);
                bytes
            }
        };

        // What Rust produces, via a KEK derived the same way.
        let expected = {
            use hkdf::Hkdf;
            use sha2_0_10::Sha256;
            let hkdf = Hkdf::<Sha256>::new(None, &ikm);
            let mut okm = [0u8; 32];
            hkdf.expand(info, &mut okm).unwrap();
            okm
        };

        assert_eq!(
            derive(vec![0u8; 32]).await,
            expected,
            "a 32-byte zero salt is what `Hkdf::new(None, ..)` means",
        );
        assert_eq!(
            derive(Vec::new()).await,
            expected,
            "and WebCrypto normalises an empty salt to the same zeros",
        );
    }
}
