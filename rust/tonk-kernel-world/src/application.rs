//! What a worker asks of another worker, or of its embedder, and the answers
//! that come back.
//!
//! A request goes out with `application.send` as a message with an id. The
//! embedder answers it with `channel.receive`, under the same id. Both are
//! JSON:
//!
//! - asked of the worker of a space, by the space's DID: `{ "ask": id, "to":
//!   did, "method", "path", "body": text or null }`
//! - asked of the embedder, to read a public text such as a seed: `{ "ask":
//!   id, "read": url }`, where a path is on the deployment's app
//! - asked of the embedder, by the person's persona, to deliver a request to
//!   the deployment's access service, such as an invocation it signed:
//!   `{ "ask": id, "fetch": url, "method", "type": media type or null,
//!   "body64": base64 or null }`, answered with `"body64"`
//! - asked of the person's persona, by a space's worker, for what its space
//!   is to be created from: `{ "ask": id, "seed": true }`, answered with a
//!   body of `{ "seed": seed or null, "copy": { "content64", "revision64" }
//!   or null }`, and that it has been: `{ "ask": id, "seeded": true }`
//! - asked of the embedder, by a persona merging into the permanent persona
//!   the person signed in as, inside `lifecycle.merge`: what the passkey
//!   ceremony made of the account, handing over the permanent persona's
//!   delegation to the representative, `{ "ask": id, "ceremony": link64 }`;
//!   and, for an account the ceremony created, to enroll it at the access
//!   service, `{ "ask": id, "enroll": true }`, answered with the receipt
//! - asked of the person's persona, by a space's worker, for what only the
//!   persona can do for the space, such as to mint an invite to it:
//!   `{ "ask": id, "profile": request }`, answered with the persona's
//!   answer, as JSON, in the body
//! - asked of the worker of a space, by the space's DID, for an answer that
//!   comes in pieces, such as a subscription's: `{ "ask": id, "to": did,
//!   "method", "path", "body", "stream": true }`, answered with its status
//!   first, `{ "answer": id, "status" }`, then each piece, `{ "piece": id,
//!   "body64" }`, then that it ended, `{ "end": id }`; this worker tells the
//!   embedder when it stops reading, `{ "tell": "cancel", "ask": id }`
//! - answered: `{ "answer": id, "status", "body": text }`, or
//!   `{ "answer": id, "error": text }` when it could not be answered.
//!
//! A worker also tells its embedder what it expects no answer to: that the
//! page that asked something, by its client id, is to go to an address,
//! `{ "tell": "navigate", "client", "href", "replace": bool }`, and that what
//! a space's worker takes up of it changed, such as where it syncs, so that
//! worker asks for its delegation again: `{ "tell": "changed", "space": did
//! or null for every space }`. The embedder tells a space's worker to ask:
//! `{ "tell": "renew" }`. The embedder
//! tells a persona that the person refused the passkey ceremony a merge
//! asked for, by the ceremony's kind and the browser's name for the refusal:
//! `{ "tell": "refused", "kind", "name" }`; and that the sign-in failed, with
//! the custody service's code for a refusal it knows: `{ "tell": "failed",
//! "kind", "code": text or null, "message" }`.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use futures::channel::{mpsc, oneshot};
use serde_json::{Value, json};

use crate::world::sapling::kernel::application;
use crate::world::sapling::kernel::types::Error;

/// An answer of another worker: its status and its body.
pub struct Answer {
    pub status: u16,
    pub body: String,
}

/// An answer of a service: its status and its body's bytes.
pub struct Delivered {
    pub status: u16,
    pub body: Vec<u8>,
}

struct Pending {
    next: u64,
    waiting: HashMap<u64, oneshot::Sender<Result<Answer, Error>>>,
    /// The answers that come in pieces, by the id they were asked under:
    /// what their status goes to, until it comes, and their pieces.
    streams: HashMap<u64, Streaming>,
}

struct Streaming {
    status: Option<oneshot::Sender<Result<u16, Error>>>,
    pieces: mpsc::UnboundedSender<Vec<u8>>,
}

/// The pieces of an answer of another worker, as they come. Dropping it
/// tells the embedder this worker stopped reading.
pub struct Pieces {
    id: u64,
    pieces: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl Stream for Pieces {
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Vec<u8>>> {
        Pin::new(&mut self.pieces).poll_next(context)
    }
}

impl Drop for Pieces {
    fn drop(&mut self) {
        let id = self.id;
        if pending(|pending| pending.streams.remove(&id)).is_none() {
            return;
        }
        wit_bindgen::spawn_local(async move {
            let cancel = json!({ "tell": "cancel", "ask": id });
            let _ = application::send(cancel.to_string().into_bytes()).await;
        });
    }
}

/// Ask the worker of the space `to` for what `method` on `path` answers,
/// with `body`, in pieces as it writes them: its status, and the pieces.
pub async fn ask_streaming(
    to: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<(u16, Pieces), Error> {
    let (status, answered) = oneshot::channel();
    let (sender, pieces) = mpsc::unbounded();
    let id = pending(|pending| {
        let id = pending.next;
        pending.next += 1;
        pending.streams.insert(
            id,
            Streaming {
                status: Some(status),
                pieces: sender,
            },
        );
        id
    });
    let pieces = Pieces { id, pieces };
    let message = json!({ "ask": id, "to": to, "method": method, "path": path, "body": body, "stream": true });
    application::send(message.to_string().into_bytes()).await?;
    let status = answered
        .await
        .unwrap_or_else(|_| Err(Error::BrokeOff("the embedder dropped the request".into())))?;
    Ok((status, pieces))
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn pending<R>(touch: impl FnOnce(&mut Pending) -> R) -> R {
    let mut held = PENDING.lock().unwrap_or_else(PoisonError::into_inner);
    touch(held.get_or_insert_with(|| Pending {
        next: 1,
        waiting: HashMap::new(),
        streams: HashMap::new(),
    }))
}

/// Ask the worker of the space `to` what `method` on `path` answers, with
/// `body`.
pub async fn ask(to: &str, method: &str, path: &str, body: Option<&str>) -> Result<Answer, Error> {
    send(|id| json!({ "ask": id, "to": to, "method": method, "path": path, "body": body })).await
}

/// Ask the person's persona what only it can do for this worker's space:
/// `request`, which says what. Answers the persona's answer.
pub async fn ask_profile(request: &Value) -> Result<Value, Error> {
    let answer = send(|id| json!({ "ask": id, "profile": request })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    serde_json::from_str(&answer.body)
        .map_err(|error| Error::Invalid(format!("the persona's answer: {error}")))
}

/// Ask the embedder for the text at `url`, which anyone may read: no
/// credentials go with it, and a redirect fails it.
pub async fn read(url: &str) -> Result<Answer, Error> {
    send(|id| json!({ "ask": id, "read": url })).await
}

/// Ask the embedder to deliver `method` on `url`, with `body` of the media
/// type `kind`, and answer what came back. Only a persona's embedder
/// delivers.
pub async fn fetch(
    method: &str,
    url: &str,
    kind: Option<&str>,
    body: Option<&[u8]>,
) -> Result<Delivered, Error> {
    let body64 = body.map(|body| BASE64.encode(body));
    let message = send(
        |id| json!({ "ask": id, "fetch": url, "method": method, "type": kind, "body64": body64 }),
    );
    let answer = message.await?;
    let body = BASE64
        .decode(answer.body)
        .map_err(|error| Error::Invalid(format!("the answer's body is not base64: {error}")))?;
    Ok(Delivered {
        status: answer.status,
        body,
    })
}

/// What a space is to be created from: the seed its persona kept as it
/// created it, as JSON, and, for a space made as a copy of another, that
/// space's snapshot.
pub struct Seed {
    pub seed: String,
    pub copy: Option<Copy>,
}

/// The snapshot of a space another is made a copy of: the CARv1 of what its
/// `main` reaches, and its revision, as JSON.
pub struct Copy {
    pub content: Vec<u8>,
    pub revision: Vec<u8>,
}

/// Ask the person's persona what this worker's space is to be created from,
/// or `None` when there is nothing to create it from.
pub async fn seed() -> Result<Option<Seed>, Error> {
    let answer = send(|id| json!({ "ask": id, "seed": true })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    let malformed = |error: String| Error::Invalid(format!("the seed's answer: {error}"));
    let answered: Value =
        serde_json::from_str(&answer.body).map_err(|e| malformed(e.to_string()))?;
    let seed = match answered.get("seed") {
        None | Some(Value::Null) => return Ok(None),
        Some(seed) => seed.to_string(),
    };
    let decoded = |copy: &Value, field: &str| {
        let text = copy
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| malformed(format!("the copy has no {field}")))?;
        BASE64.decode(text).map_err(|e| malformed(e.to_string()))
    };
    let copy = match answered.get("copy") {
        None | Some(Value::Null) => None,
        Some(copy) => Some(Copy {
            content: decoded(copy, "content64")?,
            revision: decoded(copy, "revision64")?,
        }),
    };
    Ok(Some(Seed { seed, copy }))
}

/// Ask the embedder what the passkey ceremony made of the account this
/// persona merges into, handing it `link`, the permanent persona's
/// delegation to the representative. Answers JSON.
pub async fn ceremony(link: &[u8]) -> Result<Answer, Error> {
    let link64 = BASE64.encode(link);
    let answer = send(|id| json!({ "ask": id, "ceremony": link64 })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    Ok(answer)
}

/// Ask the embedder to enroll the account the passkey ceremony created at
/// the access service. Answers the service's receipt, as JSON.
pub async fn enroll() -> Result<Answer, Error> {
    let answer = send(|id| json!({ "ask": id, "enroll": true })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    Ok(answer)
}

/// Tell the person's persona this worker created its space's content from
/// the seed.
pub async fn seeded() -> Result<(), Error> {
    let answer = send(|id| json!({ "ask": id, "seeded": true })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    Ok(())
}

/// Tell the embedder that what the worker of `space`, or of every space
/// when `None`, takes up of it changed.
pub async fn changed(space: Option<&str>) -> Result<(), Error> {
    let message = json!({ "tell": "changed", "space": space });
    application::send(message.to_string().into_bytes()).await
}

/// Tell the embedder that the page `client` is to go to `href`, as a fresh
/// load replacing its history entry where `replace`.
pub async fn navigate(client: &str, href: &str, replace: bool) -> Result<(), Error> {
    let message = json!({ "tell": "navigate", "client": client, "href": href, "replace": replace });
    application::send(message.to_string().into_bytes()).await
}

/// Send the message `asked` makes of its id, and answer what comes back
/// under it.
async fn send(asked: impl FnOnce(u64) -> Value) -> Result<Answer, Error> {
    let (answered, answer) = oneshot::channel();
    let id = pending(|pending| {
        let id = pending.next;
        pending.next += 1;
        pending.waiting.insert(id, answered);
        id
    });
    let message = asked(id);
    if let Err(error) = application::send(message.to_string().into_bytes()).await {
        pending(|pending| pending.waiting.remove(&id));
        return Err(error);
    }
    answer
        .await
        .unwrap_or_else(|_| Err(Error::BrokeOff("the embedder dropped the request".into())))
}

/// What the embedder told this worker, expecting no answer.
pub enum Told {
    /// The person refused the passkey ceremony `kind` a merge asked for, as
    /// the browser names the refusal.
    Refused { kind: String, name: String },
    /// What this worker's space is to take up changed: ask for its
    /// delegation again.
    Renew,
    /// The sign-in the passkey ceremony `kind` began failed, saying
    /// `message`, with the custody service's `code` for a refusal it knows,
    /// such as that the account awaits activation.
    Failed {
        kind: String,
        code: Option<String>,
        message: String,
    },
}

/// Take up a message the embedder handed over: the answer to a request this
/// worker asked, or what it tells this worker.
pub fn received(message: &[u8]) -> Result<Option<Told>, Error> {
    let message: Value = serde_json::from_slice(message)
        .map_err(|error| Error::Invalid(format!("a message is JSON: {error}")))?;
    let text = |field: &str| {
        message
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match message.get("tell").and_then(Value::as_str) {
        Some("refused") => {
            return Ok(Some(Told::Refused {
                kind: text("kind"),
                name: text("name"),
            }));
        }
        Some("renew") => return Ok(Some(Told::Renew)),
        Some("failed") => {
            return Ok(Some(Told::Failed {
                kind: text("kind"),
                code: Some(text("code")).filter(|code| !code.is_empty()),
                message: text("message"),
            }));
        }
        _ => {}
    }
    if let Some(id) = message.get("piece").and_then(Value::as_u64) {
        let piece = message
            .get("body64")
            .and_then(Value::as_str)
            .and_then(|body| BASE64.decode(body).ok())
            .unwrap_or_default();
        pending(|pending| {
            if let Some(streaming) = pending.streams.get(&id) {
                let _ = streaming.pieces.unbounded_send(piece);
            }
        });
        return Ok(None);
    }
    if let Some(id) = message.get("end").and_then(Value::as_u64) {
        pending(|pending| pending.streams.remove(&id));
        return Ok(None);
    }
    let Some(id) = message.get("answer").and_then(Value::as_u64) else {
        return Err(Error::Invalid("the message answers nothing asked".into()));
    };
    let status = pending(|pending| {
        pending
            .streams
            .get_mut(&id)
            .and_then(|streaming| streaming.status.take())
    });
    if let Some(status) = status {
        let answered = match message.get("error").and_then(Value::as_str) {
            Some(error) => {
                pending(|pending| pending.streams.remove(&id));
                Err(Error::Unavailable(error.to_owned()))
            }
            None => Ok(message
                .get("status")
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or(502)),
        };
        let _ = status.send(answered);
        return Ok(None);
    }
    let Some(answered) = pending(|pending| pending.waiting.remove(&id)) else {
        return Ok(None);
    };
    let answer = match message.get("error").and_then(Value::as_str) {
        Some(error) => Err(Error::Unavailable(error.to_owned())),
        None => Ok(Answer {
            status: message
                .get("status")
                .and_then(Value::as_u64)
                .and_then(|status| u16::try_from(status).ok())
                .unwrap_or(502),
            body: message
                .get("body")
                .or_else(|| message.get("body64"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
    };
    let _ = answered.send(answer);
    Ok(None)
}
