//! The embedder a service worker's bootloader drives: it boots the guest
//! kernel a space pins, after checking the bytes against the pin, and hands
//! it each request of the space's origin.

use std::rc::Rc;

use futures::lock::Mutex;
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wcmp::Engine;

use crate::bindings::sapling::kernel::storage::Edition;
use crate::kernel::Kernel;
use crate::platform;
use crate::sapling::{Change, did_key};

#[wasm_bindgen]
pub struct Embedder {
    kernel: Rc<Mutex<Kernel>>,
    timings: Vec<(String, f64)>,
}

/// Boot the guest kernel in `bytes`, over a space for a persona, each named
/// by a seed. With `check`, the bytes must hash to `pin` (BLAKE3, hex); a
/// bootloader that took them from a cache it filled by that hash has
/// checked them already.
#[wasm_bindgen]
pub async fn boot(
    bytes: Uint8Array,
    pin: String,
    check: bool,
    persona_seed: u8,
    space_seed: u8,
    kept: Array,
) -> Result<Embedder, JsError> {
    let bytes = bytes.to_vec();
    let started = platform::now_millis();
    if check {
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if hash != pin {
            return Err(JsError::new(&format!(
                "the kernel hashes to {hash}, not the pinned {pin}"
            )));
        }
    }
    let mut timings = vec![("check".to_owned(), platform::now_millis() - started)];
    let engine = Engine::with_backend(wcmp_wasm_core_web::Web::new())
        .map_err(|error| JsError::new(&error.to_string()))?;
    let started = platform::now_millis();
    let changes = kept
        .iter()
        .map(|record| from_js(&record))
        .collect::<Result<Vec<_>, _>>()?;
    timings.push(("restore".to_owned(), platform::now_millis() - started));
    let mut kernel = Kernel::load_restoring(&engine, &bytes, changes)
        .await
        .map_err(|error| JsError::new(&error))?;
    let persona = did_key(&ed25519_dalek::SigningKey::from_bytes(&[persona_seed; 32]));
    let space = did_key(&ed25519_dalek::SigningKey::from_bytes(&[space_seed; 32]));
    kernel
        .init(&persona, &space)
        .await
        .map_err(|error| JsError::new(&error))?;
    timings.extend(
        kernel
            .timings
            .iter()
            .map(|(stage, millis)| ((*stage).to_owned(), *millis)),
    );
    Ok(Embedder {
        kernel: Rc::new(Mutex::new(kernel)),
        timings,
    })
}

#[wasm_bindgen]
impl Embedder {
    /// Each write since the last drain, as records the bootloader keeps.
    pub async fn drain(&self) -> Array {
        let changes = self.kernel.lock().await.drain();
        changes.iter().map(to_js).collect()
    }

    /// How long each stage of the boot took, in milliseconds.
    #[wasm_bindgen(getter)]
    pub fn timings(&self) -> JsValue {
        let timings = Object::new();
        for (stage, millis) in &self.timings {
            let _ = Reflect::set(&timings, &stage.into(), &(*millis).into());
        }
        timings.into()
    }

    /// Hand the guest kernel one request; answers `{ status, body }`, with
    /// the body as bytes. One request at a time reaches the kernel's store.
    pub fn handle(&self, method: String, path: String, body: Uint8Array) -> js_sys::Promise {
        let kernel = Rc::clone(&self.kernel);
        let body = body.to_vec();
        wasm_bindgen_futures::future_to_promise(async move {
            let mut kernel = kernel.lock().await;
            let answer = kernel
                .request(&method, &path, &body)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?;
            let reply = Object::new();
            Reflect::set(&reply, &"status".into(), &answer.status.into())?;
            Reflect::set(
                &reply,
                &"body".into(),
                &Uint8Array::from(answer.body.as_slice()).into(),
            )?;
            Ok(reply.into())
        })
    }
}

fn field(record: &JsValue, name: &str) -> Result<JsValue, JsError> {
    Reflect::get(record, &name.into())
        .map_err(|_| JsError::new(&format!("a kept record has no {name}")))
}

fn text(record: &JsValue, name: &str) -> Result<String, JsError> {
    field(record, name)?
        .as_string()
        .ok_or_else(|| JsError::new(&format!("{name} is not text")))
}

fn bytes(record: &JsValue, name: &str) -> Result<Vec<u8>, JsError> {
    Ok(Uint8Array::new(&field(record, name)?).to_vec())
}

/// A record the bootloader kept, as the change it stands for.
fn from_js(record: &JsValue) -> Result<Change, JsError> {
    let subject = text(record, "subject")?;
    Ok(match text(record, "kind")?.as_str() {
        "block" => Change::Block {
            subject,
            catalog: text(record, "catalog")?,
            key: bytes(record, "key")?,
            bytes: bytes(record, "bytes")?,
        },
        "blob" => Change::Blob {
            subject,
            key: bytes(record, "key")?,
            bytes: bytes(record, "bytes")?,
        },
        "cell" => Change::Cell {
            subject,
            namespace: text(record, "namespace")?,
            cell: text(record, "cell")?,
            edition: match field(record, "content")?.is_undefined() {
                true => None,
                false => Some(Edition {
                    content: bytes(record, "content")?,
                    version: bytes(record, "version")?,
                }),
            },
        },
        other => return Err(JsError::new(&format!("a kept record of the kind {other}"))),
    })
}

/// A change, as a record the bootloader keeps.
fn to_js(change: &Change) -> JsValue {
    let record = Object::new();
    let set = |name: &str, value: JsValue| {
        let _ = Reflect::set(&record, &name.into(), &value);
    };
    let blob = |value: &[u8]| JsValue::from(Uint8Array::from(value));
    match change {
        Change::Block {
            subject,
            catalog,
            key,
            bytes,
        } => {
            set("kind", "block".into());
            set("subject", subject.into());
            set("catalog", catalog.into());
            set("key", blob(key));
            set("bytes", blob(bytes));
        }
        Change::Blob {
            subject,
            key,
            bytes,
        } => {
            set("kind", "blob".into());
            set("subject", subject.into());
            set("key", blob(key));
            set("bytes", blob(bytes));
        }
        Change::Cell {
            subject,
            namespace,
            cell,
            edition,
        } => {
            set("kind", "cell".into());
            set("subject", subject.into());
            set("namespace", namespace.into());
            set("cell", cell.into());
            if let Some(edition) = edition {
                set("content", blob(&edition.content));
                set("version", blob(&edition.version));
            }
        }
    }
    record.into()
}
