//! The embedder a space's service worker drives. A space holds its own
//! kernel (see [`crate::boot`]): the bootloader installs one into a space
//! that holds none, boots the one a space pins, and hands it each request of
//! the space's origin. Each layer the kernel opens is kept where Dialog DB
//! keeps its location (see [`crate::layers`]), and the session's key in
//! Dialog DB's credential store.

use std::rc::Rc;
use std::sync::Arc;

use dialog_credentials::Ed25519Signer;
use dialog_effects::storage::Directory;
use dialog_identity::OpenCredential;
use dialog_storage::provider::storage::CredentialStore;
use dialog_varsig::Principal as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::lock::Mutex;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wcmp::Engine;

use crate::bindings::sapling::kernel::session::{Claim, Subject};
use crate::bindings::sapling::kernel::types::Error;
use crate::credentials::{Answer, Claims, Standing};
use crate::kernel::{Kernel, Served};
use crate::layers::{LayerSpace, Layers};
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

/// The key this origin's session acts under: the key dialog's credential
/// store keeps under `name`, made there, where nothing reads its seed, if it
/// keeps none. A profile's worker before the kernel kept the profile's key
/// there under its name, so the profile goes on as who it was.
#[wasm_bindgen]
pub struct Session {
    key: Ed25519Signer,
}

#[wasm_bindgen]
impl Session {
    #[wasm_bindgen(getter)]
    pub fn did(&self) -> String {
        self.key.did().to_string()
    }
}

/// Open the key dialog's credential store keeps under `name`, or make one.
#[wasm_bindgen(js_name = openSession)]
pub async fn open_session(name: String) -> Result<Session, JsError> {
    let store = CredentialStore::<LayerSpace>::new();
    let credential = OpenCredential::open(name)
        .at(Directory::Profile)
        .perform(&store)
        .await
        .map_err(|error| JsError::new(&error.to_string()))?;
    let key = credential
        .signer()
        .as_ed25519()
        .cloned()
        .ok_or_else(|| JsError::new("the session's key is not Ed25519"))?;
    Ok(Session { key })
}

/// The space this origin's host serves, or `undefined` before it serves one.
#[wasm_bindgen(js_name = heldSpace)]
pub async fn held_space() -> Result<Option<String>, JsError> {
    boot::held(&Layers::default())
        .await
        .map_err(|error| JsError::new(&error))
}

/// Say that this origin's host serves the space `subject`.
#[wasm_bindgen(js_name = holdSpace)]
pub async fn hold_space(subject: String) -> Result<(), JsError> {
    boot::hold(&Layers::default(), &subject)
        .await
        .map_err(|error| JsError::new(&error))
}

/// The content identifier of a kernel's wasm.
#[wasm_bindgen(js_name = kernelId)]
pub fn kernel_id(kernel: Uint8Array) -> String {
    boot::kernel_id(&kernel.to_vec())
}

/// Install `kernel` into the space `subject`, where the space is kept.
#[wasm_bindgen]
pub async fn install(subject: String, kernel: Uint8Array) -> Result<String, JsError> {
    boot::install(&Layers::default(), &subject, &kernel.to_vec())
        .await
        .map_err(|error| JsError::new(&error))
}

/// The id of the kernel the space `subject` pins, or `undefined` when it pins
/// none.
#[wasm_bindgen]
pub async fn pinned(subject: String) -> Result<Option<String>, JsError> {
    let pinned = boot::pinned(&Layers::default(), &subject)
        .await
        .map_err(|error| JsError::new(&error))?;
    Ok(pinned.map(|(id, _)| id))
}

/// Boot the kernel the space `subject` pins, with a session that acts under
/// `session`, whose claims `claim` answers: a function that takes
/// `{ subject, command, arguments, reason }`, with `subject` undefined for
/// any subject and `arguments` as `[name, value]` pairs, and resolves to the
/// delegations that grant it, each an encoded chain.
#[wasm_bindgen]
pub async fn boot(
    subject: String,
    session: &Session,
    claim: Function,
) -> Result<Embedder, JsError> {
    let standing = Standing::sealed(session.key.clone(), claims_of(claim));
    let started = platform::now_millis();
    let layers = Layers::default();
    let (_, bytes) = boot::pinned(&layers, &subject)
        .await
        .map_err(|error| JsError::new(&error))?
        .ok_or_else(|| JsError::new("the space pins no kernel"))?;
    let mut timings = vec![("read".to_owned(), platform::now_millis() - started)];
    let engine = Engine::with_backend(wcmp_wasm_core_web::Web::new())
        .map_err(|error| JsError::new(&error.to_string()))?;
    let mut kernel = Kernel::load(&engine, &bytes, layers)
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

/// A claim, as `claim_to_js` hands one to the bootloader.
fn claim_from_js(claim: &JsValue) -> Result<Claim, JsError> {
    let field = |name: &str| Reflect::get(claim, &name.into()).unwrap_or(JsValue::UNDEFINED);
    let text = |name: &str| {
        field(name)
            .as_string()
            .ok_or_else(|| JsError::new(&format!("a claim's {name} is text")))
    };
    let subject = match field("subject").as_string() {
        Some(did) => Subject::This(did),
        None => Subject::Any,
    };
    let arguments = Array::from(&field("arguments"))
        .iter()
        .map(|pair| {
            let pair = Array::from(&pair);
            let name = pair
                .get(0)
                .as_string()
                .ok_or_else(|| JsError::new("a claim's argument is named"))?;
            Ok((name, Uint8Array::new(&pair.get(1)).to_vec()))
        })
        .collect::<Result<_, JsError>>()?;
    Ok(Claim {
        subject,
        command: text("command")?,
        arguments,
        reason: text("reason")?,
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

#[wasm_bindgen]
impl Embedder {
    /// Ask the kernel, a persona, to delegate what `claim` asks to
    /// `audience`, another worker's session: `claim` as `{ subject, command,
    /// arguments, reason }`, as a kernel's claim reaches the bootloader.
    /// Resolves to the delegations, each an encoded chain.
    pub fn grant(&self, audience: String, claim: JsValue) -> Result<Promise, JsError> {
        let claim = claim_from_js(&claim)?;
        let kernel = self.kernel.clone();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let granted = kernel
                .grant(&audience, claim)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?
                .map_err(|error| JsValue::from(JsError::new(&format!("{error:?}"))))?;
            Ok(granted
                .iter()
                .map(|chain| JsValue::from(Uint8Array::from(chain.as_slice())))
                .collect::<Array>()
                .into())
        }))
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
