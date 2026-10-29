//! The embedder a space's service worker drives. A space holds its own
//! kernel (see [`crate::boot`]): the bootloader installs one into a space
//! that holds none, boots the one a space pins, and hands it each request of
//! the space's origin. Each layer the kernel opens is kept where Dialog DB
//! keeps its location (see [`crate::layers`]), and the session's key in
//! Dialog DB's credential store.

use std::rc::Rc;

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

use crate::bindings::sapling::kernel::credential::{Access, Argument};
use crate::bindings::sapling::kernel::types::Error;
use crate::credentials::{Answer, Delegates, Representation, Standing, delegates};
use crate::kernel::{Kernel, Served};
use crate::layers::{LayerSpace, Layers};
use crate::signing::{Asked, Kind, SignIn};
use crate::{boot, platform};

#[wasm_bindgen]
pub struct Embedder {
    kernel: Served,
    timings: Vec<(String, f64)>,
    /// The key the persona's worker acts under, which the provisional
    /// persona is and the representative is.
    session: Option<Ed25519Signer>,
    sign_in: SignIn,
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

/// What this device is called, from a browser's user agent, as an account's
/// devices are listed.
#[wasm_bindgen(js_name = deviceLabel)]
pub fn device_label(agent: String) -> String {
    tonk_common::device_label::from_navigator(&agent, "", 0)
}

/// The space this origin's host serves, or `undefined` before it serves one.
#[wasm_bindgen(js_name = heldSpace)]
pub async fn held_space() -> Result<Option<String>, JsError> {
    boot::held(&Layers::default())
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

/// Boot the persona the profile `subject` pins, with a session that acts
/// under `session`, whose key represents the persona: the provisional
/// persona it is itself, until the person makes themselves known, and after
/// that the permanent persona `standing` names, `{ persona, link }`, with the
/// persona's delegation to it as `link`.
///
/// Each message the kernel sends its embedder goes to `message`, as bytes,
/// and each address its provisional persona asks to merge into to `merge`.
#[wasm_bindgen(js_name = bootPersona)]
pub async fn boot_persona(
    subject: String,
    session: &Session,
    message: Function,
    merge: Function,
    standing: JsValue,
) -> Result<Embedder, JsError> {
    let standing = if standing.is_object() {
        let persona = Reflect::get(&standing, &"persona".into())
            .ok()
            .and_then(|persona| persona.as_string())
            .ok_or_else(|| JsError::new("the standing names no persona"))?;
        let link = Uint8Array::new(&Reflect::get(&standing, &"link".into()).unwrap_or_default());
        Standing::permanent(session.key.clone(), persona, link.to_vec())
    } else {
        Standing::persona(session.key.clone())
    };
    let mut embedder = run(subject, standing, message, Some(merge)).await?;
    embedder.session = Some(session.key.clone());
    Ok(embedder)
}

/// Boot the kernel the space `subject` pins, with a session that acts under
/// `session`, for the persona `persona`, through the representative
/// `representative`, whose delegations `delegate` answers: a function that
/// takes the audience's DID and `{ subject, command, arguments, reason }`,
/// with `arguments` as `[name, value]` pairs, and resolves to the delegation,
/// an encoded chain. Each message the kernel sends its embedder goes to
/// `message`, as bytes.
#[wasm_bindgen(js_name = bootSpace)]
pub async fn boot_space(
    subject: String,
    session: &Session,
    representative: String,
    persona: String,
    delegate: Function,
    message: Function,
) -> Result<Embedder, JsError> {
    let representative = Representation::remote(representative, delegates_of(delegate));
    run(
        subject,
        Standing::space(session.key.clone(), representative, persona),
        message,
        None,
    )
    .await
}

async fn run(
    subject: String,
    standing: Standing,
    message: Function,
    merge: Option<Function>,
) -> Result<Embedder, JsError> {
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
    let mut messages = kernel.messages();
    wasm_bindgen_futures::spawn_local(async move {
        while let Some(sent) = messages.next().await {
            let sent = Uint8Array::from(sent.as_slice());
            if let Err(error) = message.call1(&JsValue::UNDEFINED, &sent) {
                platform::log(
                    "kernel",
                    &format!("a message was not taken: {error:?}"),
                    true,
                );
            }
        }
    });
    if let Some(merge) = merge {
        let mut asked = kernel.merges();
        wasm_bindgen_futures::spawn_local(async move {
            while let Some(address) = asked.next().await {
                if let Err(error) = merge.call1(&JsValue::UNDEFINED, &address.into()) {
                    platform::log("kernel", &format!("a merge was not taken: {error:?}"), true);
                }
            }
        });
    }
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
    Ok(Embedder {
        kernel,
        timings,
        session: None,
        sign_in: SignIn::default(),
    })
}

/// The representative's delegations, answered by the bootloader's
/// `delegate`.
fn delegates_of(delegate: Function) -> Delegates {
    delegates(move |audience: String, access: Access| {
        let answer = delegate.call2(
            &JsValue::UNDEFINED,
            &JsValue::from(audience),
            &access_to_js(access),
        );
        Box::pin(async move {
            let unanswered = |error: JsValue| Error::Unavailable(format!("{error:?}"));
            let chain = JsFuture::from(Promise::resolve(&answer.map_err(unanswered)?))
                .await
                .map_err(unanswered)?;
            if chain.is_undefined() || chain.is_null() {
                return Err(Error::Denied("the persona delegated nothing".into()));
            }
            Ok(Uint8Array::new(&chain).to_vec())
        }) as Answer
    })
}

/// An access, as `access_to_js` hands one to the bootloader.
fn access_from_js(access: &JsValue) -> Result<Access, JsError> {
    let field = |name: &str| Reflect::get(access, &name.into()).unwrap_or(JsValue::UNDEFINED);
    let text = |name: &str| {
        field(name)
            .as_string()
            .ok_or_else(|| JsError::new(&format!("an access's {name} is text")))
    };
    let arguments = Array::from(&field("arguments"))
        .iter()
        .map(|pair| {
            let pair = Array::from(&pair);
            let key = pair
                .get(0)
                .as_string()
                .ok_or_else(|| JsError::new("an access's argument is named"))?;
            Ok(Argument {
                key,
                value: Uint8Array::new(&pair.get(1)).to_vec(),
            })
        })
        .collect::<Result<_, JsError>>()?;
    Ok(Access {
        subject: Reflect::get(access, &"subject".into())
            .ok()
            .and_then(|subject| subject.as_string()),
        command: text("command")?,
        arguments,
        reason: text("reason")?,
    })
}

fn access_to_js(access: Access) -> JsValue {
    let object = Object::new();
    let arguments: Array = access
        .arguments
        .into_iter()
        .map(|argument| {
            Array::of2(
                &argument.key.into(),
                &Uint8Array::from(argument.value.as_slice()).into(),
            )
        })
        .collect();
    let subject = access.subject.map_or(JsValue::NULL, JsValue::from);
    let _ = Reflect::set(&object, &"subject".into(), &subject);
    let _ = Reflect::set(&object, &"command".into(), &access.command.into());
    let _ = Reflect::set(&object, &"arguments".into(), &arguments);
    let _ = Reflect::set(&object, &"reason".into(), &access.reason.into());
    object.into()
}

#[wasm_bindgen]
impl Embedder {
    /// Ask the kernel, a persona, to delegate `access` to `audience`,
    /// another worker's session: `access` as `{ subject, command, arguments,
    /// reason }`, as a kernel's request reaches the bootloader. Resolves to
    /// the delegation, an encoded chain.
    pub fn delegate(&self, audience: String, access: JsValue) -> Result<Promise, JsError> {
        let access = access_from_js(&access)?;
        let kernel = self.kernel.clone();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let chain = kernel
                .delegate(&audience, access)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?
                .map_err(|error| JsValue::from(JsError::new(&format!("{error:?}"))))?;
            Ok(Uint8Array::from(chain.as_slice()).into())
        }))
    }

    /// Sign the person in with what the page derived with a passkey,
    /// `handoff` as `{ credentialId, key, kek }`, for the ceremony `kind`,
    /// `create` or `log-in`, as `email`, from this device named
    /// `deviceName`, to an account that syncs through `remote`: the
    /// persona's provisional persona merges into the account. Resolves to
    /// `{ persona, link }`, the account's DID and its delegation to this
    /// device, to boot the persona with from then on; rejects with an error
    /// that may carry the custody service's `code`.
    #[wasm_bindgen(js_name = signIn)]
    pub fn sign_in(
        &self,
        handoff: JsValue,
        kind: String,
        email: String,
        device_name: String,
        remote: String,
    ) -> Result<Promise, JsError> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| JsError::new("only a persona signs in"))?;
        let kind = match kind.as_str() {
            "create" => Kind::Create,
            "log-in" => Kind::LogIn,
            other => return Err(JsError::new(&format!("no ceremony is {other}"))),
        };
        let (kernel, sign_in) = (self.kernel.clone(), self.sign_in.clone());
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let (persona, link) = sign_in
                .sign_in(
                    &kernel,
                    &session,
                    handoff,
                    Asked {
                        kind,
                        email,
                        device_name,
                        remote,
                    },
                )
                .await?;
            let merged = Object::new();
            let _ = Reflect::set(&merged, &"persona".into(), &persona.into());
            let _ = Reflect::set(&merged, &"link".into(), &Uint8Array::from(link.as_slice()));
            Ok(merged.into())
        }))
    }

    /// What the ceremony of the sign-in under way made of the account, as
    /// JSON, for the kernel, which hands over `link`, the account's
    /// delegation to this device.
    pub fn ceremony(&self, link: Uint8Array) -> Promise {
        let sign_in = self.sign_in.clone();
        let link = link.to_vec();
        wasm_bindgen_futures::future_to_promise(async move {
            let answer = sign_in
                .ceremony(&link)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?;
            Ok(answer.into())
        })
    }

    /// The invocation that enrolls the account the sign-in under way
    /// created at the access service, as the bytes to post to it.
    pub fn enrollment(&self) -> Result<Promise, JsError> {
        let session = self
            .session
            .clone()
            .ok_or_else(|| JsError::new("only a persona enrolls"))?;
        let sign_in = self.sign_in.clone();
        Ok(wasm_bindgen_futures::future_to_promise(async move {
            let body = sign_in
                .enrollment(&session)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?;
            Ok(Uint8Array::from(body.as_slice()).into())
        }))
    }

    /// Hand the kernel `message` from its embedder. Resolves once the kernel
    /// took it up.
    pub fn receive(&self, message: Uint8Array) -> Promise {
        let kernel = self.kernel.clone();
        let message = message.to_vec();
        wasm_bindgen_futures::future_to_promise(async move {
            kernel
                .receive(message)
                .await
                .map_err(|error| JsValue::from(JsError::new(&error)))?
                .map_err(|error| JsValue::from(JsError::new(&format!("{error:?}"))))?;
            Ok(JsValue::UNDEFINED)
        })
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
