//! The local wrapping: this-browser custody of the account secret.
//!
//! The KEK is a non-extractable WebCrypto AES-256-GCM key. It and the
//! sealed envelope persist together in one small IndexedDB record
//! keyed by account DID — a key-value pair, not a repository; nothing
//! here shows up as a database anyone syncs. Until a durable wrapping
//! (passkey, phrase) exists this record is the only custody of the
//! account, and the UI must say so.
//!
//! `navigator.storage.persist()` is requested on establishment and
//! treated as advisory.

use anyhow::{Context, Result, anyhow};
use idb::{Database, DatabaseEvent, Factory, ObjectStoreParams, TransactionMode};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AesGcmParams, AesKeyGenParams, CryptoKey, SubtleCrypto};
use zeroize::Zeroizing;

use crate::envelope::{AccountSecret, Envelope, KekMethod};

const DATABASE: &str = "tonk-custody";
const STORE: &str = "wrappings";
const KEY_PROPERTY: &str = "key";
const ENVELOPE_PROPERTY: &str = "envelope";
const AES_GCM: &str = "AES-GCM";
const NONCE_LEN: usize = 12;

fn web_error(context: &str, value: JsValue) -> anyhow::Error {
    anyhow!("{context}: {value:?}")
}

fn store_error(context: &str, error: idb::Error) -> anyhow::Error {
    anyhow!("{context}: {error}")
}

fn subtle() -> Result<SubtleCrypto> {
    Ok(web_sys::window()
        .context("no window: local custody is page-only")?
        .crypto()
        .map_err(|e| web_error("no WebCrypto", e))?
        .subtle())
}

async fn open_database() -> Result<Database> {
    let factory = Factory::new().map_err(|e| store_error("no IndexedDB", e))?;
    let mut request = factory
        .open(DATABASE, Some(1))
        .map_err(|e| store_error("failed to open the custody store", e))?;
    request.on_upgrade_needed(|event| {
        if let Ok(database) = event.database() {
            let _ = database.create_object_store(STORE, ObjectStoreParams::new());
        }
    });
    request
        .await
        .map_err(|e| store_error("failed to open the custody store", e))
}

async fn generate_kek(subtle: &SubtleCrypto) -> Result<CryptoKey> {
    let algorithm = AesKeyGenParams::new(AES_GCM, 256);
    let usages = Array::of2(&"encrypt".into(), &"decrypt".into());
    let promise = subtle
        .generate_key_with_object(&algorithm, false, &usages)
        .map_err(|e| web_error("failed to request a local KEK", e))?;
    JsFuture::from(promise)
        .await
        .map_err(|e| web_error("failed to generate the local KEK", e))?
        .dyn_into()
        .map_err(|_| anyhow!("generateKey answered a non-key"))
}

async fn seal(subtle: &SubtleCrypto, kek: &CryptoKey, secret: &AccountSecret) -> Result<Envelope> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce)
        .map_err(|error| anyhow!("no entropy for the local envelope nonce: {error}"))?;
    // Header fields are fixed at generation 0 until rotation ships; the
    // associated data must match what the assembled envelope carries.
    let shell = Envelope::from_parts(0, KekMethod::Local, nonce, Vec::new());
    let algorithm = AesGcmParams::new(AES_GCM, &Uint8Array::from(nonce.as_slice()).into());
    algorithm.set_additional_data_u8_array(&Uint8Array::from(shell.associated_data().as_slice()));
    let mut plaintext = Zeroizing::new(*secret.bytes());
    let promise = subtle
        .encrypt_with_object_and_u8_array(&algorithm, kek, plaintext.as_mut())
        .map_err(|e| web_error("failed to request the local sealing", e))?;
    let buffer = JsFuture::from(promise)
        .await
        .map_err(|e| web_error("the local sealing failed", e))?;
    let ciphertext = Uint8Array::new(&buffer).to_vec();
    Ok(Envelope::from_parts(0, KekMethod::Local, nonce, ciphertext))
}

async fn open(
    subtle: &SubtleCrypto,
    kek: &CryptoKey,
    envelope: &Envelope,
) -> Result<AccountSecret> {
    let nonce = envelope.nonce_bytes();
    let algorithm = AesGcmParams::new(AES_GCM, &Uint8Array::from(nonce.as_slice()).into());
    algorithm
        .set_additional_data_u8_array(&Uint8Array::from(envelope.associated_data().as_slice()));
    let mut ciphertext = envelope.ciphertext_bytes().to_vec();
    let promise = subtle
        .decrypt_with_object_and_u8_array(&algorithm, kek, ciphertext.as_mut())
        .map_err(|e| web_error("failed to request the local unwrap", e))?;
    let buffer = JsFuture::from(promise)
        .await
        .map_err(|_| anyhow!("the local envelope did not open"))?;
    let plaintext = Uint8Array::new(&buffer);
    if plaintext.length() != 32 {
        anyhow::bail!("the local envelope holds no account secret");
    }
    let mut bytes = Zeroizing::new([0u8; 32]);
    plaintext.copy_to(bytes.as_mut());
    plaintext.fill(0, 0, plaintext.length());
    Ok(AccountSecret::from_bytes(bytes))
}

/// Establish this browser's local wrapping of the account: generate a
/// non-extractable KEK, seal the secret under it, and persist both.
/// Idempotent per account — a re-establishment overwrites the record.
pub async fn establish(account_did: &str, secret: &AccountSecret) -> Result<()> {
    let subtle = subtle()?;
    let kek = generate_kek(&subtle).await?;
    let envelope = seal(&subtle, &kek, secret).await?;

    let record = Object::new();
    Reflect::set(&record, &KEY_PROPERTY.into(), &kek.into())
        .map_err(|e| web_error("failed to build the custody record", e))?;
    Reflect::set(
        &record,
        &ENVELOPE_PROPERTY.into(),
        &Uint8Array::from(envelope.encode().as_slice()).into(),
    )
    .map_err(|e| web_error("failed to build the custody record", e))?;

    let database = open_database().await?;
    let transaction = database
        .transaction(&[STORE], TransactionMode::ReadWrite)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    let store = transaction
        .object_store(STORE)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    store
        .put(&record, Some(&JsValue::from_str(account_did)))
        .map_err(|e| store_error("failed to write the custody record", e))?
        .await
        .map_err(|e| store_error("failed to write the custody record", e))?;
    transaction
        .commit()
        .map_err(|e| store_error("failed to commit the custody record", e))?
        .await
        .map_err(|e| store_error("failed to commit the custody record", e))?;

    // Advisory: ask the browser not to evict this origin's storage.
    if let Some(window) = web_sys::window()
        && let Ok(promise) = window.navigator().storage().persist()
    {
        let _ = JsFuture::from(promise).await;
    }
    Ok(())
}

async fn load(account_did: &str) -> Result<Option<(CryptoKey, Envelope)>> {
    let database = open_database().await?;
    let transaction = database
        .transaction(&[STORE], TransactionMode::ReadOnly)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    let store = transaction
        .object_store(STORE)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    let record = store
        .get(JsValue::from_str(account_did))
        .map_err(|e| store_error("failed to read the custody record", e))?
        .await
        .map_err(|e| store_error("failed to read the custody record", e))?;
    let Some(record) = record else {
        return Ok(None);
    };
    let kek: CryptoKey = Reflect::get(&record, &KEY_PROPERTY.into())
        .map_err(|e| web_error("the custody record is malformed", e))?
        .dyn_into()
        .map_err(|_| anyhow!("the custody record holds no key"))?;
    let bytes = Uint8Array::new(
        &Reflect::get(&record, &ENVELOPE_PROPERTY.into())
            .map_err(|e| web_error("the custody record is malformed", e))?,
    )
    .to_vec();
    let envelope = Envelope::decode(&bytes)
        .map_err(|error| anyhow!("the local envelope is unreadable: {error}"))?;
    Ok(Some((kek, envelope)))
}

/// Unlock the account from this browser's local wrapping, if one
/// exists for `account_did`.
pub async fn unlock(account_did: &str) -> Result<Option<AccountSecret>> {
    let Some((kek, envelope)) = load(account_did).await? else {
        return Ok(None);
    };
    let secret = open(&subtle()?, &kek, &envelope).await?;
    Ok(Some(secret))
}

/// Whether this browser holds a local wrapping for `account_did`.
pub async fn exists(account_did: &str) -> Result<bool> {
    Ok(load(account_did).await?.is_some())
}

/// Discard this browser's local wrapping — sign-out. The account
/// survives wherever a durable wrapping exists; a local-only account
/// dies here, which the UI must confirm before calling.
pub async fn discard(account_did: &str) -> Result<()> {
    let database = open_database().await?;
    let transaction = database
        .transaction(&[STORE], TransactionMode::ReadWrite)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    let store = transaction
        .object_store(STORE)
        .map_err(|e| store_error("failed to open the custody store", e))?;
    store
        .delete(JsValue::from_str(account_did))
        .map_err(|e| store_error("failed to delete the custody record", e))?
        .await
        .map_err(|e| store_error("failed to delete the custody record", e))?;
    transaction
        .commit()
        .map_err(|e| store_error("failed to commit the deletion", e))?
        .await
        .map_err(|e| store_error("failed to commit the deletion", e))?;
    Ok(())
}
