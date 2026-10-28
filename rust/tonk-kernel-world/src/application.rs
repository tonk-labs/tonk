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
//! - answered: `{ "answer": id, "status", "body": text }`, or
//!   `{ "answer": id, "error": text }` when it could not be answered.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::channel::oneshot;
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
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn pending<R>(touch: impl FnOnce(&mut Pending) -> R) -> R {
    let mut held = PENDING.lock().unwrap_or_else(PoisonError::into_inner);
    touch(held.get_or_insert_with(|| Pending {
        next: 1,
        waiting: HashMap::new(),
    }))
}

/// Ask the worker of the space `to` what `method` on `path` answers, with
/// `body`.
pub async fn ask(to: &str, method: &str, path: &str, body: Option<&str>) -> Result<Answer, Error> {
    send(|id| json!({ "ask": id, "to": to, "method": method, "path": path, "body": body })).await
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

/// Tell the person's persona this worker created its space's content from
/// the seed.
pub async fn seeded() -> Result<(), Error> {
    let answer = send(|id| json!({ "ask": id, "seeded": true })).await?;
    if !(200..300).contains(&answer.status) {
        return Err(Error::Unavailable(answer.body));
    }
    Ok(())
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

/// Take up a message the embedder handed over: the answer to a request this
/// worker asked.
pub fn received(message: &[u8]) -> Result<(), Error> {
    let message: Value = serde_json::from_slice(message)
        .map_err(|error| Error::Invalid(format!("a message is JSON: {error}")))?;
    let Some(id) = message.get("answer").and_then(Value::as_u64) else {
        return Err(Error::Invalid("the message answers nothing asked".into()));
    };
    let Some(answered) = pending(|pending| pending.waiting.remove(&id)) else {
        return Ok(());
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
    Ok(())
}
