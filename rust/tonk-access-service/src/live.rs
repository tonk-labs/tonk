//! A space's live side: one Durable Object per space, which keeps the
//! space's sockets, answers the invocations they carry, and tells the
//! watches they began of each change to the space's cells.
//!
//! A socket is accepted at `GET /ucan/?sub=<space>` when it asks for
//! dialog's subprotocol, and hibernates while idle. Every frame is an
//! invocation verified on its own, screened as a request is, and refused
//! when it names a space other than the socket's. What each socket
//! watches is kept in the object's storage, so it survives hibernation,
//! and the object keeps the record of the invocations it accepted in its
//! storage too, so one presented again on any of its sockets is refused,
//! however long the object slept in between. A socket's `ping` is answered
//! with `pong` by the runtime, without waking the object.
//!
//! A cell write the object performs itself is fanned out to every socket
//! at once. One the worker performed over HTTP is reported here
//! afterwards (`POST /changed`), and the object reads the cell back and
//! fans it out the same way. R2 stays the store that decides a write:
//! a watcher orders the heads it is told of by what they carry.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use dialog_capability::{Did, Provider, Subject};
use dialog_effects::memory;
use dialog_effects::memory::prelude::CellScope;
use dialog_remote_ucan::socket::{
    Change, PING, PONG, Reply, Request as Frame, SUBPROTOCOL, Session,
};
use dialog_remote_ucan::{Access, Presented, Subscription};
use dialog_ucan_core::{Container, InvocationChain};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use worker::worker_sys::WebSocketRequestResponsePair;
use worker::{
    DurableObject, Env, ListOptions, Method, Request, Response, Result, State, Storage, WebSocket,
    WebSocketIncomingMessage, WebSocketPair, console_error, durable_object,
};

use crate::error::Refusal;
use crate::objects::Objects;
use crate::revocation::checker::IndexedRevocations;
use crate::revocation::index::kv::KvRevocationIndex;

/// The binding the worker reaches a space's object through.
pub const BINDING: &str = crate::service::SOCKETS_BINDING;

/// How long a watch's authority stands, once found to hold, before it is
/// checked again on delivering to it.
const RECHECK: Duration = Duration::from_secs(30);

/// What a socket carries through hibernation: which of the object's
/// sockets it is, and the space it serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Attachment {
    id: String,
    subject: String,
}

/// A cell of the space the worker wrote, as it reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Changed {
    /// The space the cell is in.
    pub space: String,
    /// The cell.
    pub cell: String,
}

/// What a frame invokes, read before it is answered.
struct Invoked {
    /// The frame, as its `Invoke` named it, which a refusal echoes.
    id: Option<u64>,
    /// The invocation's content identifier.
    invocation: String,
    /// The space it names.
    subject: String,
    /// The container's bytes, as screening and metering read them.
    container: Vec<u8>,
    /// The cell it writes, when it writes one.
    writes: Option<(String, String)>,
}

/// One space's live side.
#[durable_object]
pub struct Live {
    state: State,
    env: Env,
    presented: Arc<dyn Presented>,
}

impl DurableObject for Live {
    fn new(state: State, env: Env) -> Self {
        // The runtime answers a ping itself, so a socket kept alive by
        // its client costs no wake of the object.
        if let Ok(pair) = WebSocketRequestResponsePair::new(PING, PONG) {
            state.set_websocket_auto_response(&pair);
        }
        let presented = Arc::new(Stored(state.storage()));
        Self {
            state,
            env,
            presented,
        }
    }

    async fn fetch(&self, mut req: Request) -> Result<Response> {
        crate::secret::load(&self.env).await;
        let url = req.url()?;
        let Some(subject) = url
            .query_pairs()
            .find(|(name, _)| name == "sub")
            .map(|(_, value)| value.into_owned())
        else {
            return Response::error("name the space as ?sub=", 400);
        };
        match (req.method(), url.path()) {
            (Method::Get, "/ucan/") => self.accept(&req, subject),
            (Method::Post, "/changed") => {
                let changed: Changed = req.json().await?;
                self.fan_out(&subject, &changed.space, &changed.cell)
                    .await?;
                Response::empty()
            }
            _ => Response::error("Not found", 404),
        }
    }

    async fn websocket_message(
        &self,
        ws: WebSocket,
        message: WebSocketIncomingMessage,
    ) -> Result<()> {
        crate::secret::load(&self.env).await;
        let bytes = match message {
            WebSocketIncomingMessage::Binary(bytes) => bytes,
            // Answered by the runtime where it keeps the auto-response;
            // here where it does not.
            WebSocketIncomingMessage::String(text) if text == PING => {
                return ws.send_with_str(PONG);
            }
            WebSocketIncomingMessage::String(_) => return Ok(()),
        };
        let Some(socket) = ws.deserialize_attachment::<Attachment>()? else {
            return Ok(());
        };
        let invoked = invoked(&bytes);
        if let Some(invoked) = &invoked
            && let Err(refusal) = self.admissible(&socket, invoked).await
        {
            return send(
                &ws,
                &refused(invoked.id, Some(invoked.invocation.clone()), &refusal),
            );
        }

        let access = self.access()?;
        let mut session = self.session(&socket.id).await?;
        let reply = session.receive(&access, &bytes).await;
        self.save(&socket.id, &session).await?;
        let Some(reply) = reply else {
            return Ok(());
        };
        send(&ws, &reply)?;
        if let Some(invoked) = &invoked {
            self.meter(invoked, &reply);
            if let (Reply::Answer { status, .. }, Some((space, cell))) = (&reply, &invoked.writes)
                && (200..300).contains(status)
            {
                self.fan_out(&socket.subject, space, cell).await?;
            }
        }
        Ok(())
    }

    /// Forget the invocations no longer fresh: they are refused as stale
    /// now, so the record need not hold them.
    async fn alarm(&self) -> Result<Response> {
        crate::secret::load(&self.env).await;
        forget_expired(&self.state.storage(), now()).await?;
        Response::empty()
    }

    async fn websocket_close(
        &self,
        ws: WebSocket,
        _code: usize,
        _reason: String,
        _was_clean: bool,
    ) -> Result<()> {
        self.forget(&ws).await
    }

    async fn websocket_error(&self, ws: WebSocket, _error: worker::Error) -> Result<()> {
        self.forget(&ws).await
    }
}

impl Live {
    /// Accept a socket for `subject` that asks for dialog's subprotocol,
    /// answering with it.
    fn accept(&self, req: &Request, subject: String) -> Result<Response> {
        let asked = req
            .headers()
            .get("Sec-WebSocket-Protocol")?
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|protocol| protocol.trim() == SUBPROTOCOL)
            });
        if !asked {
            return Response::error(format!("this socket speaks {SUBPROTOCOL}"), 426);
        }
        let pair = WebSocketPair::new()?;
        pair.server.serialize_attachment(Attachment {
            id: fresh_id()?,
            subject,
        })?;
        self.state.accept_web_socket(&pair.server);
        let mut response = Response::from_websocket(pair.client)?;
        response
            .headers_mut()
            .set("Sec-WebSocket-Protocol", SUBPROTOCOL)?;
        Ok(response)
    }

    /// Whether the frame's invocation may be answered on this socket: it
    /// names the socket's space, and the space is served.
    async fn admissible(
        &self,
        socket: &Attachment,
        invoked: &Invoked,
    ) -> std::result::Result<(), Refusal> {
        if invoked.subject != socket.subject {
            return Err(dialog_capability::access::AuthorizeError::Declined {
                recourse: dialog_capability::access::Recourse::None,
                reason: format!("this socket serves {}", socket.subject),
            }
            .into());
        }
        crate::handlers::ucan::screen(&invoked.container, &self.env).await
    }

    /// The access layer over the space's bucket, with the record of the
    /// invocations this object accepted.
    fn access(
        &self,
    ) -> Result<
        Access<
            Objects,
            dialog_remote_ucan_s3::DefaultResolver,
            IndexedRevocations<KvRevocationIndex>,
        >,
    > {
        let objects = Objects::new(self.env.bucket("BUCKET")?);
        let revocations = self.env.kv("REVOCATIONS_KV")?;
        Ok(
            Access::with_shared_resolver(objects, crate::handlers::ucan::shared_resolver())
                .with_revocations(IndexedRevocations(KvRevocationIndex::new(revocations)))
                .with_presented(self.presented.clone()),
        )
    }

    /// Tell every socket's watches of the cell `cell` in `space` what it
    /// holds now, all at once.
    async fn fan_out(&self, subject: &str, space: &str, cell: &str) -> Result<()> {
        let did: Did = subject
            .parse()
            .map_err(|_| worker::Error::RustError(format!("{subject} is not a DID")))?;
        let access = self.access()?;
        let resolve = CellScope::new(Subject::from(did.clone()), space, cell).resolve();
        let state = Provider::<memory::Resolve>::execute(access.provider(), resolve)
            .await
            .map_err(|error| worker::Error::RustError(error.to_string()))?;
        let change = Change {
            subject: &did,
            space,
            cell,
            state: &state,
        };
        let at = access.now();
        let deliveries = self.state.get_websockets().into_iter().map(|ws| {
            let access = &access;
            async move {
                let socket = ws.deserialize_attachment::<Attachment>()?;
                let Some(socket) = socket.filter(|socket| socket.subject == subject) else {
                    return Ok(());
                };
                let mut session = self.session(&socket.id).await?;
                let replies = session.deliver(access, change, RECHECK, at).await;
                if replies.is_empty() {
                    return Ok(());
                }
                self.save(&socket.id, &session).await?;
                for reply in &replies {
                    send(&ws, reply)?;
                }
                Ok::<(), worker::Error>(())
            }
        });
        for delivered in join_all(deliveries).await {
            if let Err(error) = delivered {
                console_error!("a change was not delivered to a socket: {error}");
            }
        }
        Ok(())
    }

    /// The watches socket `id` began.
    async fn session(&self, id: &str) -> Result<Session> {
        let watches: Option<Vec<Subscription>> = self.state.storage().get(&watches(id)).await?;
        Ok(Session::restore(watches.unwrap_or_default()))
    }

    /// Keep what socket `id` watches.
    async fn save(&self, id: &str, session: &Session) -> Result<()> {
        let held: Vec<Subscription> = session.watches().cloned().collect();
        if held.is_empty() {
            self.state.storage().delete(&watches(id)).await?;
        } else {
            self.state.storage().put(&watches(id), held).await?;
        }
        Ok(())
    }

    /// Drop what a closed socket watched.
    async fn forget(&self, ws: &WebSocket) -> Result<()> {
        if let Some(socket) = ws.deserialize_attachment::<Attachment>()? {
            self.state.storage().delete(&watches(&socket.id)).await?;
        }
        Ok(())
    }

    /// Meter the frame's invocation as a request's is.
    fn meter(&self, invoked: &Invoked, reply: &Reply) {
        let metered = match reply {
            Reply::Answer { status, body, .. } if (200..300).contains(status) => {
                Some(("ok", None, body.len() as u64))
            }
            Reply::State { .. } => Some(("ok", None, 0)),
            Reply::Answer { status, body, .. } if matches!(status, 401 | 403) => {
                Some(("denied", refusal_kind(body), 0))
            }
            _ => None,
        };
        if let Some((outcome, reason, bytes)) = metered
            && let Some(write) = crate::handlers::ucan::metering(
                &invoked.container,
                outcome,
                reason,
                bytes,
                &self.env,
            )
        {
            self.state.wait_until(write);
        }
    }
}

/// What the frame `bytes` invokes, when it is an invocation that can be
/// read.
fn invoked(bytes: &[u8]) -> Option<Invoked> {
    let Frame::Invoke { id, container, .. } = Frame::decode(bytes).ok()? else {
        return None;
    };
    let chain = InvocationChain::try_from(Container::from_bytes(&container).ok()?).ok()?;
    Some(Invoked {
        id,
        invocation: chain.invocation.to_cid().to_string(),
        subject: chain.subject().to_string(),
        writes: crate::socket::cell_written(&chain),
        container,
    })
}

/// Hand a socket upgrade for `/ucan/?sub=<space>` to the space's object.
pub async fn connect(req: Request, env: &Env) -> Result<Response> {
    let url = req.url()?;
    let Some(subject) = url
        .query_pairs()
        .find(|(name, _)| name == "sub")
        .map(|(_, value)| value.into_owned())
    else {
        return Response::error("name the space as ?sub=", 400);
    };
    // An environment without the binding has no sockets: the client
    // syncs over requests instead.
    let Ok(spaces) = env.durable_object(BINDING) else {
        return Response::error("this service keeps no sockets", 404);
    };
    spaces
        .id_from_name(&subject)?
        .get_stub()?
        .fetch_with_request(req)
        .await
}

/// Report to `subject`'s object that the worker wrote the cell `cell` in
/// `space`, so its watches are told. Best effort: a watch the report does
/// not reach learns of the change when its host next checks.
pub async fn changed(env: Env, subject: String, space: String, cell: String) {
    // Without the binding there are no sockets, so no watch to tell.
    let Ok(spaces) = env.durable_object(BINDING) else {
        return;
    };
    let report = async {
        let mut init = worker::RequestInit::new();
        init.with_method(Method::Post).with_body(Some(
            serde_json::to_string(&Changed { space, cell })
                .map_err(|error| worker::Error::RustError(error.to_string()))?
                .into(),
        ));
        let url = format!(
            "https://live.invalid/changed?sub={}",
            url::form_urlencoded::byte_serialize(subject.as_bytes()).collect::<String>()
        );
        spaces
            .id_from_name(&subject)?
            .get_stub()?
            .fetch_with_request(Request::new_with_init(&url, &init)?)
            .await
    };
    if let Err(error) = report.await {
        console_error!("a write was not reported to its space's watches: {error}");
    }
}

/// The answer a refused frame gets: what a refused request's body says.
fn refused(id: Option<u64>, invocation: Option<String>, refusal: &Refusal) -> Reply {
    Reply::Answer {
        id,
        invocation,
        status: refusal.status(),
        version: None,
        body: refusal.body(),
    }
}

fn refusal_kind(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value["kind"].as_str().map(str::to_owned))
}

fn send(ws: &WebSocket, reply: &Reply) -> Result<()> {
    ws.send_with_bytes(reply.encode())
}

fn watches(id: &str) -> String {
    format!("watches/{id}")
}

/// Where the record of accepted invocations is kept, by invocation.
const PRESENTED: &str = "presented/";

/// The invocations the object accepted, kept in its storage, each until it
/// is no longer fresh: memory would be lost each time the object
/// hibernates, and an invocation captured before could be presented again
/// after. The object handles one event at a time, so the read and the
/// write of a record are not interleaved with another socket's.
struct Stored(Storage);

#[async_trait(?Send)]
impl Presented for Stored {
    async fn record(&self, invocation: &str, until: u64, now: u64) -> bool {
        let key = format!("{PRESENTED}{invocation}");
        match self.0.get::<u64>(&key).await {
            Ok(Some(remembered)) if remembered >= now => return false,
            Ok(_) => {}
            // A record that cannot be read refuses the invocation rather
            // than let a copy through.
            Err(error) => {
                console_error!("the record of accepted invocations could not be read: {error}");
                return false;
            }
        }
        if let Err(error) = self.0.put(&key, until).await {
            console_error!("an accepted invocation could not be recorded: {error}");
            return false;
        }
        // Forget it once it is no longer fresh, if nothing is due sooner.
        if matches!(self.0.get_alarm().await, Ok(None)) {
            let wait = Duration::from_secs(until.saturating_sub(now).max(1));
            if let Err(error) = self.0.set_alarm(wait).await {
                console_error!("the record of accepted invocations will not be pruned: {error}");
            }
        }
        true
    }
}

/// Delete the records of invocations no longer fresh at `now`, and wake
/// again when the next one stops being.
async fn forget_expired(storage: &Storage, now: u64) -> Result<()> {
    let held = storage
        .list_with_options(ListOptions::new().prefix(PRESENTED))
        .await?;
    let mut expired = Vec::new();
    let mut next: Option<u64> = None;
    held.for_each(&mut |until, key| {
        let (Some(key), Some(until)) = (key.as_string(), until.as_f64()) else {
            return;
        };
        let until = until as u64;
        if until < now {
            expired.push(key);
        } else {
            next = Some(next.map_or(until, |next| next.min(until)));
        }
    });
    // Storage deletes at most 128 keys at once.
    for keys in expired.chunks(128) {
        storage.delete_multiple(keys.to_vec()).await?;
    }
    if let Some(next) = next {
        storage
            .set_alarm(Duration::from_secs(next.saturating_sub(now).max(1)))
            .await?;
    }
    Ok(())
}

fn fresh_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| worker::Error::RustError(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now() -> u64 {
    (worker::Date::now().as_millis() / 1_000) as u64
}
