//! The embedder a space's service worker drives. A space holds its own
//! kernel (see [`crate::boot`]): the bootloader installs one into a space
//! that holds none, boots the one a space pins from what the space's store
//! keeps, and hands it each request of the space's origin.

use std::rc::Rc;
use std::sync::Arc;

use dialog_credentials::Ed25519Signer;
use dialog_credentials::key::KeyExport;
use dialog_varsig::Principal as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::lock::Mutex;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wcmp::Engine;

use crate::bindings::sapling::kernel::session::{Claim, Subject};
use crate::bindings::sapling::kernel::storage::Edition;
use crate::bindings::sapling::kernel::types::Error;
use crate::credentials::{Answer, Claims, Standing};
use crate::kernel::{Kernel, Served};
use crate::sapling::Change;
use crate::{boot, platform};

#[wasm_bindgen]
pub struct Embedder {
    kernel: Served,
    timings: Vec<(String, f64)>,
}

/// The body of an answer of the kernel, piece by piece as the kernel writes
/// it. `next` answers the next piece, or `undefined` once the body ended.
#[wasm_bindgen]
pub struct BodyReader {
    pieces: Rc<Mutex<mpsc::UnboundedReceiver<Vec<u8>>>>,
}

#[wasm_bindgen]
impl BodyReader {
    pub async fn next(&self) -> Option<Uint8Array> {
        let piece = self.pieces.lock().await.next().await?;
        Some(Uint8Array::from(piece.as_slice()))
    }

    /// Stop reading: the kernel is told no one wants the rest.
    pub fn cancel(&self) {
        if let Some(mut pieces) = self.pieces.try_lock() {
            pieces.close();
        }
    }
}

/// A key for this origin's session, made here, as the bootloader keeps it:
/// a key nothing reads the seed of. The person's profile delegates the space
/// to it.
#[wasm_bindgen(js_name = generateSession)]
pub async fn generate_session() -> Result<JsValue, JsError> {
    let key = Ed25519Signer::generate()
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;
    kept(&key).await
}

/// The DID of `key`, a key the bootloader keeps.
#[wasm_bindgen(js_name = keyDid)]
pub async fn key_did(key: JsValue) -> Result<String, JsError> {
    Ok(restore(key).await?.did().to_string())
}

/// The space the records `kept` hold a kernel for, or `undefined` when they
/// hold none.
#[wasm_bindgen(js_name = heldSpace)]
pub fn held_space(kept: Array) -> Result<Option<String>, JsError> {
    Ok(boot::held(&changes_of(&kept)?))
}

/// `key`, as the bootloader keeps it.
async fn kept(key: &Ed25519Signer) -> Result<JsValue, JsError> {
    Ok(key
        .export()
        .await
        .map_err(|error| JsError::new(&error.to_string()))?
        .into())
}

/// A key the bootloader kept.
async fn restore(key: JsValue) -> Result<Ed25519Signer, JsError> {
    let export =
        KeyExport::try_from(key).map_err(|_| JsError::new("a kept key is not a key's export"))?;
    Ed25519Signer::import(export)
        .await
        .map_err(|error| JsError::new(&error.to_string()))
}

/// The content identifier of a kernel's wasm.
#[wasm_bindgen(js_name = kernelId)]
pub fn kernel_id(kernel: Uint8Array) -> String {
    boot::kernel_id(&kernel.to_vec())
}

/// The records that install `kernel` into the space `subject`, for the
/// bootloader to keep with the rest of the space's store.
#[wasm_bindgen]
pub fn install(subject: String, kernel: Uint8Array) -> Array {
    let (_, changes) = boot::install(&subject, &kernel.to_vec());
    changes.iter().map(to_js).collect()
}

/// The id of the kernel the space `subject` pins, among the records `kept`,
/// or `undefined` when it pins none.
#[wasm_bindgen]
pub fn pinned(subject: String, kept: Array) -> Result<Option<String>, JsError> {
    let changes = changes_of(&kept)?;
    let pinned = boot::pinned(&subject, &changes).map_err(|error| JsError::new(&error))?;
    Ok(pinned.map(|(id, _)| id))
}

/// Boot the kernel the space `subject` pins, over the records `kept`, with a
/// session that acts under the key `session`, whose claims `claim` answers:
/// a function that takes `{ subject, command, arguments, reason }`, with
/// `subject` undefined for any subject and `arguments` as `[name, value]`
/// pairs, and resolves to the delegations that grant it, each an encoded
/// chain.
#[wasm_bindgen]
pub async fn boot(
    subject: String,
    session: JsValue,
    claim: Function,
    kept: Array,
) -> Result<Embedder, JsError> {
    let standing = Standing::sealed(restore(session).await?, claims_of(claim));
    let started = platform::now_millis();
    let changes = changes_of(&kept)?;
    let (_, bytes) = boot::pinned(&subject, &changes)
        .map_err(|error| JsError::new(&error))?
        .ok_or_else(|| JsError::new("the space pins no kernel"))?;
    let mut timings = vec![("restore".to_owned(), platform::now_millis() - started)];
    let engine = Engine::with_backend(wcmp_wasm_core_web::Web::new())
        .map_err(|error| JsError::new(&error.to_string()))?;
    let mut kernel = Kernel::load_restoring(&engine, &bytes, changes)
        .await
        .map_err(|error| JsError::new(&error))?;
    kernel
        .init(&subject, standing)
        .await
        .map_err(|error| JsError::new(&error))?;
    timings.extend(
        kernel
            .timings
            .iter()
            .map(|(stage, millis)| ((*stage).to_owned(), *millis)),
    );
    let (kernel, driver) = kernel.serve();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = driver.await {
            platform::log("kernel", &format!("the kernel stopped: {error}"), true);
        }
    });
    Ok(Embedder { kernel, timings })
}

/// The kernel's claims, answered by the bootloader's `claim`.
fn claims_of(claim: Function) -> Claims {
    Arc::new(move |asked: Claim| {
        let answer = claim.call1(&JsValue::UNDEFINED, &claim_to_js(asked));
        Box::pin(async move {
            let unanswered = |error: JsValue| Error::Unavailable(format!("{error:?}"));
            let chains = JsFuture::from(Promise::resolve(&answer.map_err(unanswered)?))
                .await
                .map_err(unanswered)?;
            Ok(Array::from(&chains)
                .iter()
                .map(|chain| Uint8Array::new(&chain).to_vec())
                .collect())
        }) as Answer
    })
}

fn claim_to_js(claim: Claim) -> JsValue {
    let object = Object::new();
    let subject = match claim.subject {
        Subject::This(did) => JsValue::from(did),
        Subject::Any => JsValue::UNDEFINED,
    };
    let arguments: Array = claim
        .arguments
        .into_iter()
        .map(|(name, value)| Array::of2(&name.into(), &Uint8Array::from(value.as_slice()).into()))
        .collect();
    let _ = Reflect::set(&object, &"subject".into(), &subject);
    let _ = Reflect::set(&object, &"command".into(), &claim.command.into());
    let _ = Reflect::set(&object, &"arguments".into(), &arguments);
    let _ = Reflect::set(&object, &"reason".into(), &claim.reason.into());
    object.into()
}

/// The changes among `kept`. A host before custodies kept keys and site
/// secrets as records of their own; a kernel keeps them in its cells now.
fn changes_of(kept: &Array) -> Result<Vec<Change>, JsError> {
    kept.iter()
        .filter(|record| !matches!(text(record, "kind").as_deref(), Ok("key" | "secret")))
        .map(|record| from_js(&record))
        .collect()
}

#[wasm_bindgen]
impl Embedder {
    /// Each write since the last drain, as records the bootloader keeps.
    pub async fn drain(&self) -> Result<Array, JsError> {
        let changes = self
            .kernel
            .drain()
            .await
            .map_err(|error| JsError::new(&error))?;
        Ok(changes.iter().map(to_js).collect())
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

    /// Hand the guest kernel one request, with `headers` as `[name, value]`
    /// pairs; answers `{ status, headers, body }`
    /// once it has its status, with the body a [`BodyReader`] the kernel
    /// writes into as it goes. Requests reach the kernel at once, however
    /// many are open.
    pub fn handle(
        &self,
        method: String,
        path: String,
        headers: Array,
        body: Uint8Array,
    ) -> js_sys::Promise {
        let kernel = self.kernel.clone();
        let body = body.to_vec();
        let headers = headers
            .iter()
            .filter_map(|pair| {
                let pair = Array::from(&pair);
                Some((
                    pair.get(0).as_string()?,
                    pair.get(1).as_string()?.into_bytes(),
                ))
            })
            .collect();
        wasm_bindgen_futures::future_to_promise(async move {
            let head = kernel
                .request(&method, &path, headers, &body)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?;
            let headers = Array::new();
            for (name, value) in head.headers {
                let value = String::from_utf8_lossy(&value).into_owned();
                headers.push(&Array::of2(&name.into(), &value.into()));
            }
            let reply = Object::new();
            Reflect::set(&reply, &"status".into(), &head.status.into())?;
            Reflect::set(&reply, &"headers".into(), &headers)?;
            let reader = BodyReader {
                pieces: Rc::new(Mutex::new(head.body)),
            };
            Reflect::set(&reply, &"body".into(), &reader.into())?;
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
