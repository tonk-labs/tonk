//! What a worker asks of another worker, through its embedder, and the
//! answers that come back.
//!
//! A request goes out with `application.send` as a message naming the
//! worker it is for, by the DID of the space that worker serves, and an id.
//! The embedder delivers it and hands the answer back with `channel.receive`,
//! under the same id. Both are JSON:
//!
//! - asked: `{ "ask": id, "to": did, "method", "path", "body": text or null }`
//! - answered: `{ "answer": id, "status", "body": text }`, or
//!   `{ "answer": id, "error": text }` when it could not be delivered.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use futures::channel::oneshot;
use serde_json::{Value, json};

use crate::world::sapling::kernel::application;
use crate::world::sapling::kernel::types::Error;

/// An answer of another worker: its status and its body.
pub struct Answer {
    pub status: u16,
    pub body: String,
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
    let (answered, answer) = oneshot::channel();
    let id = pending(|pending| {
        let id = pending.next;
        pending.next += 1;
        pending.waiting.insert(id, answered);
        id
    });
    let message = json!({ "ask": id, "to": to, "method": method, "path": path, "body": body });
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
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }),
    };
    let _ = answered.send(answer);
    Ok(())
}
