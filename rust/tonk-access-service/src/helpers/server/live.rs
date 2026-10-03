//! A space's live side, natively: the local server's twin of the
//! worker's `Live` Durable Object.
//!
//! A socket is accepted at `GET /ucan/?sub=<space>` asking to upgrade,
//! and answered as the worker's object answers it: every frame is an
//! invocation verified on its own, screened as a request is, and refused
//! when it names a space other than the socket's. A cell write, whether
//! over a socket or over HTTP, is read back and told to every socket's
//! watches at once.
//!
//! The object keeps what each socket watches in its storage, because it
//! hibernates. A socket here is a task that lives as long as its
//! connection, so its session lives in the task.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dialog_capability::{Capability, Did, Provider, Subject};
use dialog_effects::archive::{self, ArchiveError};
use dialog_effects::blob::{self, BlobError, BlobReader, BlobWriter};
use dialog_effects::memory::prelude::CellScope;
use dialog_effects::memory::{self, CellState, Edition, MemoryError, Version};
use dialog_effects::rejection::Rejection;
use dialog_remote_s3::helpers::S3Network;
use dialog_remote_s3::{Address, s3::S3Credential};
use dialog_remote_ucan::socket::{Change, Reply, Request as Frame, SUBPROTOCOL, Session};
use dialog_remote_ucan::{Access, RecentInvocations};
use dialog_ucan_core::{Container, InvocationChain};
use futures_util::{SinkExt as _, StreamExt as _};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, HeaderValue, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::broadcast;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;

use super::{RegistrationState, unavailable_provisioning, unix_now};
use crate::error::Refusal;
use crate::revocation::checker::IndexedRevocations;
use crate::revocation::index::MemoryRevocationIndex;

/// How long a watch's authority stands, once found to hold, before it is
/// checked again on delivering to it. The worker's object waits as long.
const RECHECK: Duration = Duration::from_secs(30);

/// The access layer a socket's frames are answered through.
type LiveAccess = Access<
    LocalObjects,
    dialog_remote_ucan_s3::DefaultResolver,
    IndexedRevocations<Arc<MemoryRevocationIndex>>,
>;

/// The local server's live side: the access layer every socket answers
/// through, and the channel each write is told on.
pub(super) struct Live {
    access: LiveAccess,
    changes: broadcast::Sender<Changed>,
}

/// A cell of a space, as it holds after a write.
#[derive(Clone)]
struct Changed {
    subject: Did,
    space: String,
    cell: String,
    state: CellState,
}

impl Live {
    /// The live side over the backing store at `address`, checking
    /// revocations against `revocations`, as `/ucan/` does.
    pub(super) fn new(
        address: Address,
        credential: S3Credential,
        revocations: Arc<MemoryRevocationIndex>,
    ) -> Self {
        let (changes, _) = broadcast::channel(256);
        Self {
            access: Access::new(LocalObjects {
                address,
                network: S3Network::from(credential),
            })
            .with_revocations(IndexedRevocations(revocations))
            .with_presented(Arc::new(RecentInvocations::default())),
            changes,
        }
    }

    /// Tell every socket's watches that the cell `cell` in `space` of
    /// `subject` was written, reading back what it holds now. Best
    /// effort: a watch this does not reach learns of the change when its
    /// host next checks.
    pub(super) async fn changed(&self, subject: &str, space: &str, cell: &str) {
        let Ok(did) = subject.parse::<Did>() else {
            return;
        };
        let resolve = CellScope::new(Subject::from(did.clone()), space, cell).resolve();
        match Provider::<memory::Resolve>::execute(self.access.provider(), resolve).await {
            // Nobody listening is not a failure: there is no watch to tell.
            Ok(state) => {
                let _ = self.changes.send(Changed {
                    subject: did,
                    space: space.to_string(),
                    cell: cell.to_string(),
                    state,
                });
            }
            Err(error) => eprintln!("a write was not told to its space's watches: {error}"),
        }
    }
}

/// Whether `req` asks to upgrade to a WebSocket.
pub(super) fn is_upgrade(req: &Request<Incoming>) -> bool {
    req.headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

/// Accept the socket `req` asks for, for the space its `?sub=` names,
/// when it asks for dialog's subprotocol; the connection is then served
/// as the space's socket until it closes.
pub(super) fn upgrade(
    mut req: Request<Incoming>,
    registration: Arc<RegistrationState>,
) -> Response<http_body_util::Full<bytes::Bytes>> {
    let refuse = |status: StatusCode, message: String| {
        Response::builder()
            .status(status)
            .body(http_body_util::Full::new(bytes::Bytes::from(message)))
            .expect("a refusal builds")
    };
    let subject = req.uri().query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(name, _)| name == "sub")
            .map(|(_, value)| value.into_owned())
    });
    let Some(subject) = subject else {
        return refuse(StatusCode::BAD_REQUEST, "name the space as ?sub=".into());
    };
    let asked = req
        .headers()
        .get_all("Sec-WebSocket-Protocol")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|protocol| protocol.trim() == SUBPROTOCOL);
    if !asked {
        return refuse(
            StatusCode::UPGRADE_REQUIRED,
            format!("this socket speaks {SUBPROTOCOL}"),
        );
    }
    let Some(key) = req.headers().get(SEC_WEBSOCKET_KEY) else {
        return refuse(StatusCode::BAD_REQUEST, "no Sec-WebSocket-Key".into());
    };
    let accept = derive_accept_key(key.as_bytes());

    let upgrading = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        match upgrading.await {
            Ok(upgraded) => {
                let socket =
                    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)
                        .await;
                serve(socket, subject, registration).await;
            }
            Err(error) => eprintln!("a socket was not upgraded: {error}"),
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, HeaderValue::from_static("Upgrade"))
        .header(UPGRADE, HeaderValue::from_static("websocket"))
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .header(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(SUBPROTOCOL),
        )
        .body(http_body_util::Full::new(bytes::Bytes::new()))
        .expect("an upgrade builds")
}

/// One socket of `subject`'s: a session answering its frames, and
/// delivering each change to the space to the watches it began.
async fn serve<S>(socket: WebSocketStream<S>, subject: String, registration: Arc<RegistrationState>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let live = &registration.live;
    let mut changes = live.changes.subscribe();
    let (mut sink, mut source) = socket.split();
    let mut session = Session::new();
    loop {
        tokio::select! {
            message = source.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    let invoked = invoked(&bytes);
                    if let Some(invoked) = &invoked
                        && let Err(refusal) = admissible(&registration, &subject, invoked).await
                    {
                        let reply = refused(Some(invoked.invocation.clone()), &refusal);
                        if sink.send(Message::binary(reply.encode())).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    let Some(reply) = session.receive(&live.access, &bytes).await else {
                        continue;
                    };
                    if sink.send(Message::binary(reply.encode())).await.is_err() {
                        break;
                    }
                    if let Some(invoked) = &invoked {
                        meter(&registration, invoked, &reply).await;
                        if let (Reply::Answer { status, .. }, Some((space, cell))) =
                            (&reply, &invoked.writes)
                            && (200..300).contains(status)
                        {
                            live.changed(&subject, space, cell).await;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            change = changes.recv() => match change {
                Ok(change) if change.subject.to_string() == subject => {
                    let delivered = session
                        .deliver(
                            &live.access,
                            Change {
                                subject: &change.subject,
                                space: &change.space,
                                cell: &change.cell,
                                state: &change.state,
                            },
                            RECHECK,
                            unix_now(),
                        )
                        .await;
                    for reply in delivered {
                        if sink.send(Message::binary(reply.encode())).await.is_err() {
                            return;
                        }
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
}

/// What a frame invokes, read before it is answered.
struct Invoked {
    /// The invocation's content identifier.
    invocation: String,
    /// The space it names.
    subject: String,
    /// The container's bytes, as screening and metering read them.
    container: Vec<u8>,
    /// The cell it writes, when it writes one.
    writes: Option<(String, String)>,
}

/// What the frame `bytes` invokes, when it is an invocation that can be
/// read.
fn invoked(bytes: &[u8]) -> Option<Invoked> {
    let Frame::Invoke { container, .. } = Frame::decode(bytes).ok()? else {
        return None;
    };
    let chain = InvocationChain::try_from(Container::from_bytes(&container).ok()?).ok()?;
    Some(Invoked {
        invocation: chain.invocation.to_cid().to_string(),
        subject: chain.subject().to_string(),
        writes: crate::socket::cell_written(&chain),
        container,
    })
}

/// Whether the frame's invocation may be answered on `subject`'s socket:
/// it names that space, and the space is served.
async fn admissible(
    registration: &RegistrationState,
    subject: &str,
    invoked: &Invoked,
) -> Result<(), Refusal> {
    if invoked.subject != subject {
        return Err(dialog_capability::access::AuthorizeError::Declined {
            recourse: dialog_capability::access::Recourse::None,
            reason: format!("this socket serves {subject}"),
        }
        .into());
    }
    match crate::provisioning::screen(&registration.store, subject, unix_now()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(reason)) => Err(reason.into()),
        Err(error) => {
            eprintln!("a socket's frame was refused, control store unreachable: {error}");
            Err(unavailable_provisioning().into())
        }
    }
}

/// Meter the frame's invocation as a request's is.
async fn meter(registration: &RegistrationState, invoked: &Invoked, reply: &Reply) {
    use crate::store::ingest::IngestStore;

    let metered = match reply {
        Reply::Answer { status, body, .. } if (200..300).contains(status) => {
            Some(("ok", None, body.len() as u64))
        }
        Reply::State { .. } => Some(("ok", None, 0)),
        Reply::Answer {
            status: 401 | 403,
            body,
            ..
        } => Some(("denied", refusal_kind(body), 0)),
        _ => None,
    };
    if let Some((outcome, reason, bytes)) = metered
        && let Some(record) =
            crate::metering::collect(&invoked.container, outcome, reason, bytes, unix_now())
        && let Err(error) = registration.ingest.record(&record).await
    {
        eprintln!("metering write failed: {error}");
    }
}

/// The answer a refused frame gets: what a refused request's body says.
fn refused(invocation: Option<String>, refusal: &Refusal) -> Reply {
    Reply::Answer {
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

/// The backing store, as the provider of the effects a socket's frames
/// perform: each is performed against the local S3 at the key the permit
/// route addresses it by, so what a socket writes a request reads.
pub(super) struct LocalObjects {
    address: Address,
    network: S3Network,
}

#[async_trait]
impl Provider<archive::Get> for LocalObjects {
    async fn execute(
        &self,
        capability: Capability<archive::Get>,
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        capability.fork(&self.address).perform(&self.network).await
    }
}

#[async_trait]
impl Provider<archive::Put> for LocalObjects {
    async fn execute(&self, capability: Capability<archive::Put>) -> Result<(), ArchiveError> {
        capability.fork(&self.address).perform(&self.network).await
    }
}

#[async_trait]
impl Provider<memory::Resolve> for LocalObjects {
    async fn execute(
        &self,
        capability: Capability<memory::Resolve>,
    ) -> Result<Option<Edition<Vec<u8>>>, MemoryError> {
        capability.fork(&self.address).perform(&self.network).await
    }
}

#[async_trait]
impl Provider<memory::Publish> for LocalObjects {
    async fn execute(
        &self,
        capability: Capability<memory::Publish>,
    ) -> Result<Version, MemoryError> {
        capability.fork(&self.address).perform(&self.network).await
    }
}

#[async_trait]
impl Provider<memory::Retract> for LocalObjects {
    async fn execute(&self, capability: Capability<memory::Retract>) -> Result<(), MemoryError> {
        capability.fork(&self.address).perform(&self.network).await
    }
}

/// Blobs are not carried over a socket: they go as requests, to
/// `/object/`, which streams them.
#[async_trait]
impl Provider<blob::Read> for LocalObjects {
    async fn execute(&self, _: Capability<blob::Read>) -> Result<BlobReader, BlobError> {
        Err(blobs_go_as_requests())
    }
}

#[async_trait]
impl Provider<blob::Import> for LocalObjects {
    async fn execute(&self, _: Capability<blob::Import>) -> Result<BlobWriter, BlobError> {
        Err(blobs_go_as_requests())
    }
}

fn blobs_go_as_requests() -> BlobError {
    Rejection::Unsupported {
        reason: "blobs are not carried over a socket; request them instead".into(),
    }
    .into()
}
