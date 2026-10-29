//! The space writer: the only place revisions for one space and one MCP
//! session are signed.
//!
//! Dialog treats each origin (a branch and the operator DID writing it) as
//! one sequential actor, and names a revision by `(origin, edition)`
//! without its tree. Two writers sharing a DID that commit on the same
//! head both mint the same version, the second one's pull takes the
//! first's head for its own, and its push replaces it: a write is lost
//! with no error (dialog-db/dialog-db#705). A hosted peer rebuilt per
//! request did exactly that to four of five parallel writes.
//!
//! So each MCP session writes as its own DID, derived from the session's
//! seed and the session id ([`WriterKey`]), and one [`SpaceWriter`] per
//! space and session holds the only peer that signs as it. Its writes take
//! a lock from the pull before the commit through the push after it, and
//! answer only once the push has landed; a write answered as failed never
//! lands later. Reads never wait for a write.
//!
//! Nothing here knows about Durable Objects: the Host Worker keeps one
//! writer per object, and native tests drive the same type directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, Uri, header};
use dialog_credentials::Ed25519Signer;
use dialog_effects::storage::Directory;
use dialog_repository::HeldCaches;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::{Did, Principal as _};
use http_body_util::BodyExt as _;
use parking_lot::RwLock;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use super::HostedPeer;
use crate::TonkWorkerError;

/// The bytes of tree nodes a writer's peer keeps, for every repository it
/// opens together.
///
/// A Durable Object has 128 MB for the bundle, the caches, the peer and
/// whatever a query holds while it runs, and wasm memory never shrinks.
/// See the memory budget in `plans/2026-10-28-hosted-agent-peer.md`.
pub const NODE_CACHE_BUDGET: usize = 24 * 1024 * 1024;

/// The context the writer key is derived under, in BLAKE3's
/// `derive_key` form. Changing it moves every session's writer to a new
/// DID, and so starts a new origin in every space it writes.
const WRITER_KEY_CONTEXT: &str = "tonk 2026-10-29 hosted agent space writer key v1";

/// The branch a writer serves of its space.
const BRANCH: &str = "main";

/// The seed of the writer key for the MCP session `mcp_session` of the
/// session whose key `session_seed` is.
///
/// The session seed is always 32 bytes, so the session id after it needs
/// no length prefix to keep two inputs apart.
pub fn writer_seed(session_seed: &[u8; 32], mcp_session: &str) -> Zeroizing<[u8; 32]> {
    let mut hasher = blake3::Hasher::new_derive_key(WRITER_KEY_CONTEXT);
    hasher.update(session_seed);
    hasher.update(mcp_session.as_bytes());
    Zeroizing::new(*hasher.finalize().as_bytes())
}

/// What the front hands a writer about the session a request came in on.
///
/// The seed and the powerline are what an agent session token carries
/// (#1202: `token.seed` in the outer delegation's `meta`, and
/// `AgentSessionToken::powerline`). They are taken apart here so the
/// writer does not depend on the token's encoding, which the front alone
/// decodes and verifies.
pub struct WriterSession {
    /// The session key's seed.
    pub seed: Zeroizing<[u8; 32]>,
    /// The `agent space -> session` powerline.
    pub powerline: DelegationChain,
    /// The MCP session the writer serves: `Mcp-Session-Id`, minted by the
    /// MCP Worker and bound to the token, so a client cannot choose it.
    pub mcp_session: String,
}

/// The key one MCP session writes as, and the chain that lets it.
pub struct WriterKey {
    seed: Zeroizing<[u8; 32]>,
    did: Did,
    chain: DelegationChain,
}

impl WriterKey {
    /// Derive the writer key of `session` and have the session delegate
    /// to it: the session's powerline extended by `session -> writer`, so
    /// revoking the session cuts every writer under it.
    pub async fn derive(session: &WriterSession) -> Result<Self, TonkWorkerError> {
        let internal = |error: &dyn std::fmt::Display| TonkWorkerError::Internal(error.to_string());
        let seed = writer_seed(&session.seed, &session.mcp_session);
        let did = Ed25519Signer::import(&*seed)
            .await
            .map_err(|error| internal(&error))?
            .did();
        let session_key = Ed25519Signer::import(&*session.seed)
            .await
            .map_err(|error| internal(&error))?;
        if session.powerline.audience() != &session_key.did() {
            return Err(TonkWorkerError::Forbidden(
                "the powerline is not addressed to the session's key".into(),
            ));
        }
        let hop = tonk_identity::delegation::mint_device_delegation(session_key, &did)
            .await
            .map_err(|error| internal(&error))?;
        let delegation = hop
            .proofs()
            .last()
            .cloned()
            .ok_or_else(|| TonkWorkerError::Internal("an empty delegation chain".into()))?;
        let chain = session
            .powerline
            .push(delegation)
            .map_err(|error| internal(&error))?;
        Ok(Self { seed, did, chain })
    }

    /// The writer's DID.
    pub fn did(&self) -> &Did {
        &self.did
    }

    /// `agent space -> session -> writer`.
    pub fn chain(&self) -> &DelegationChain {
        &self.chain
    }
}

/// A warm peer for one space and one MCP session, signing as that
/// session's writer key.
///
/// Writes are serialized from the pull before the commit through the push
/// after it, so a write is answered only once it has landed, and two
/// writes never commit on one head. Parallel tool calls in a session are
/// ordinary (several calls in one turn, subagents sharing a connection,
/// retries), and an async runtime interleaves them at every await: a
/// Durable Object's input gate does not hold across the I/O a write waits
/// on, so nothing short of this lock keeps them apart.
///
/// A write that is answered as failed never lands later. When its push
/// fails and the remote does not hold its commit, the writer drops its
/// peer, commit and caches with it, and opens a new one from the remote
/// before it lets go of the lock. Dialog cannot rewind a branch: a reset
/// backwards leaves the version it rewound past in the peer's caches,
/// and the next commit mints that version again (see dialog's
/// `Branch::reset`). A peer opened fresh has seen only what the remote
/// holds.
///
/// Reads never wait for a write. A read pulls first when no write holds
/// the lock, and reads what is local otherwise: the write in flight is
/// pulling and pushing itself, and a pull in the middle of it would merge
/// into its unpushed commit and mint a revision on a read's behalf. Reads
/// do wait for one another's pulls, and share one: a pull that began
/// after a read did serves that read too.
pub struct SpaceWriter {
    /// The peer, replaced when a write that did not land is dropped.
    peer: RwLock<Arc<HostedPeer>>,
    key: WriterKey,
    space: Did,
    remote: url::Url,
    /// Where each peer is opened: `{root}/{generation}`.
    root: String,
    generation: AtomicU64,
    base: String,
    lock: Mutex<()>,
    /// A write did not land and no new peer could be opened in its place,
    /// so the current one may hold it: no write runs until one is.
    stale: AtomicBool,
    /// Taken by a read while it pulls, so reads that arrive meanwhile
    /// wait for that pull instead of reading what was there before it.
    pulls: Mutex<()>,
    /// How many reads have begun; each takes the next number.
    reads: AtomicU64,
    /// Reads numbered below this began before a pull that has landed.
    pulled_for: AtomicU64,
    /// Fail the next push before it is sent, as a refusing remote would.
    #[cfg(test)]
    refuse_push: AtomicBool,
}

impl SpaceWriter {
    /// Open the writer of `space` for `session`, its peers kept under the
    /// directory `root` (which nothing else may use), syncing with `remote`
    /// (the access service's `/ucan/`).
    pub async fn open(
        root: &str,
        session: &WriterSession,
        space: &Did,
        remote: &url::Url,
    ) -> Result<Self, TonkWorkerError> {
        let key = WriterKey::derive(session).await?;
        let root = root.trim_end_matches('/').to_owned();
        let peer = open_peer(&key, &format!("{root}/0"), space, remote).await?;
        Ok(Self {
            peer: RwLock::new(Arc::new(peer)),
            key,
            space: space.clone(),
            remote: remote.clone(),
            root,
            generation: AtomicU64::new(0),
            base: format!("/api/repository/{space}/branch/{BRANCH}"),
            lock: Mutex::new(()),
            stale: AtomicBool::new(false),
            pulls: Mutex::new(()),
            reads: AtomicU64::new(0),
            pulled_for: AtomicU64::new(0),
            #[cfg(test)]
            refuse_push: AtomicBool::new(false),
        })
    }

    /// The space this writer writes.
    pub fn space(&self) -> &Did {
        &self.space
    }

    /// The writer key's DID: the profile of the writer's peer. Revisions
    /// are issued by its operator (see [`Self::operator`]), a key derived
    /// from this one, so distinct writers issue as distinct DIDs.
    pub fn did(&self) -> &Did {
        &self.key.did
    }

    /// The DID this writer's revisions are issued by.
    pub async fn operator(&self) -> Did {
        self.peer().operator().await
    }

    /// Commit one write and push it, holding the lock throughout.
    ///
    /// `request` names a route of the space's branch relative to it, such
    /// as `/evaluate?transact=true` or `/blob`. The answer is the route's
    /// own, sent once the push has landed. A write whose push failed is
    /// answered with `502` and `WRITE_NOT_LANDED`, and is gone: the next
    /// write commits on the remote's head, without it.
    pub async fn write(&self, request: Request<Body>) -> Response<Body> {
        let _turn = self.lock.lock().await;
        if self.stale.load(Ordering::SeqCst) {
            if let Err(error) = self.reopen().await {
                return failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "WRITER_UNAVAILABLE",
                    &error,
                );
            }
            self.stale.store(false, Ordering::SeqCst);
        }
        let peer = self.peer();
        if let Err(error) = self.sync(&peer, "pull").await {
            return failure(StatusCode::BAD_GATEWAY, "PULL_FAILED", &error);
        }
        let response = peer.call(self.on_branch(request)).await;
        if !response.status().is_success() {
            return response;
        }
        let (parts, body) = response.into_parts();
        let body = match body.collect().await {
            Ok(body) => body.to_bytes(),
            Err(error) => {
                return failure(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "WRITE_UNREADABLE",
                    &error.to_string(),
                );
            }
        };
        if let Err(error) = self.push(&peer).await {
            // A push can fail after its publish reached the remote, when
            // only the answer was lost: then the write did land.
            if self.landed(&peer).await {
                return Response::from_parts(parts, Body::from(body));
            }
            if let Err(reopen) = self.reopen().await {
                self.stale.store(true, Ordering::SeqCst);
                return failure(
                    StatusCode::BAD_GATEWAY,
                    "WRITE_NOT_LANDED",
                    &format!("{error}; the writer reopens before its next write: {reopen}"),
                );
            }
            return failure(StatusCode::BAD_GATEWAY, "WRITE_NOT_LANDED", &error);
        }
        Response::from_parts(parts, Body::from(body))
    }

    /// Answer one read without waiting for the lock.
    ///
    /// Only routes that cannot commit are served: `GET` and `HEAD`, a
    /// `query`, and an `evaluate` with `transact=false`. Anything else is
    /// refused with `405`, since a commit here would skip the lock and the
    /// push.
    pub async fn read(&self, request: Request<Body>) -> Response<Body> {
        if !is_read(request.method(), request.uri()) {
            return failure(
                StatusCode::METHOD_NOT_ALLOWED,
                "NOT_A_READ",
                "this route can commit; send it as a write",
            );
        }
        if let Err(error) = self.refresh().await {
            return failure(StatusCode::BAD_GATEWAY, "PULL_FAILED", &error);
        }
        self.peer().call(self.on_branch(request)).await
    }

    /// The branch's sync status: its local head and the remote's, read
    /// without moving either.
    pub async fn status(&self) -> Result<tonk_worker_api::SyncStatusResponse, TonkWorkerError> {
        status(&self.peer(), &self.base).await
    }

    /// Release the peer. Waits for a write in flight to finish.
    pub async fn close(self) {
        drop(self.lock.lock().await);
        self.peer().shut_down().await;
    }

    fn peer(&self) -> Arc<HostedPeer> {
        self.peer.read().clone()
    }

    /// Whether the remote's head is `peer`'s own, as it is when a push
    /// published and only its answer was lost. Unknown counts as no.
    async fn landed(&self, peer: &HostedPeer) -> bool {
        match status(peer, &self.base).await {
            Ok(status) => status.local.is_some() && status.local == status.remote,
            Err(_) => false,
        }
    }

    /// Drop the peer and what it committed, and open a new one from the
    /// remote, pulled to its head. Called with the lock held.
    async fn reopen(&self) -> Result<(), String> {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let directory = format!("{}/{generation}", self.root);
        let peer = open_peer(&self.key, &directory, &self.space, &self.remote)
            .await
            .map_err(|error| error.to_string())?;
        if let Err(error) = self.sync(&peer, "pull").await {
            peer.close().await;
            return Err(error);
        }
        let old = std::mem::replace(&mut *self.peer.write(), Arc::new(peer));
        // A read still on the old peer fails rather than answer from it.
        old.shut_down().await;
        Ok(())
    }

    /// `request`, addressed to the space's branch.
    fn on_branch(&self, request: Request<Body>) -> Request<Body> {
        let (mut parts, body) = request.into_parts();
        let relative = parts
            .uri
            .path_and_query()
            .map(|path| path.as_str())
            .unwrap_or("")
            .trim_start_matches('/');
        let path = if relative.is_empty() || relative.starts_with('?') {
            format!("{}{relative}", self.base)
        } else {
            format!("{}/{relative}", self.base)
        };
        parts.uri = path
            .parse::<Uri>()
            .unwrap_or_else(|_| Uri::from_static("/"));
        Request::from_parts(parts, body)
    }

    /// Pull for a read that is about to begin, unless a pull that began
    /// after it already landed or a write holds the lock.
    async fn refresh(&self) -> Result<(), String> {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        let _pulling = self.pulls.lock().await;
        if read < self.pulled_for.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Ok(_turn) = self.lock.try_lock() else {
            return Ok(());
        };
        let covers = self.reads.load(Ordering::SeqCst);
        self.sync(&self.peer(), "pull").await?;
        self.pulled_for.fetch_max(covers, Ordering::SeqCst);
        Ok(())
    }

    async fn push(&self, peer: &HostedPeer) -> Result<(), String> {
        #[cfg(test)]
        if self.refuse_push.swap(false, Ordering::SeqCst) {
            return Err("push: refused by the test".into());
        }
        self.sync(peer, "push").await
    }

    /// Pull or push the space's branch.
    async fn sync(&self, peer: &HostedPeer, operation: &str) -> Result<(), String> {
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("{}/sync/{operation}", self.base))
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::empty())
            .map_err(|error| error.to_string())?;
        let response = peer.call(request).await;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        Err(format!(
            "{operation}: {status}: {}",
            String::from_utf8_lossy(&body)
        ))
    }
}

/// Open a hosted peer acting as `key` in `directory`, with `space`
/// mounted as a replica of `remote`.
async fn open_peer(
    key: &WriterKey,
    directory: &str,
    space: &Did,
    remote: &url::Url,
) -> Result<HostedPeer, TonkWorkerError> {
    let peer = HostedPeer::open_in(
        Directory::At(directory.to_owned()),
        &key.seed,
        std::slice::from_ref(&key.chain),
        HeldCaches::with_budget(NODE_CACHE_BUDGET),
    )
    .await?;
    if peer.did().await != key.did {
        peer.close().await;
        return Err(TonkWorkerError::Internal(
            "the hosted peer does not act as the writer key".into(),
        ));
    }
    if let Err(error) = peer.mount(space, remote).await {
        peer.close().await;
        return Err(error);
    }
    Ok(peer)
}

/// `peer`'s sync status for the branch at `base`.
async fn status(
    peer: &HostedPeer,
    base: &str,
) -> Result<tonk_worker_api::SyncStatusResponse, TonkWorkerError> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{base}/sync/status"))
        .body(Body::empty())
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?;
    let response = peer.call(request).await;
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|error| TonkWorkerError::Internal(error.to_string()))?
        .to_bytes();
    if !status.is_success() {
        return Err(TonkWorkerError::Internal(format!(
            "sync status: {status}: {}",
            String::from_utf8_lossy(&body)
        )));
    }
    serde_json::from_slice(&body).map_err(|error| TonkWorkerError::Internal(error.to_string()))
}

/// Whether a request to a branch route cannot commit.
fn is_read(method: &Method, uri: &Uri) -> bool {
    if method == Method::GET || method == Method::HEAD {
        return true;
    }
    if method != Method::POST {
        return false;
    }
    match uri.path().trim_matches('/') {
        "query" => true,
        // `transact` defaults to true, so only an explicit `false`, and
        // no other value beside it, makes an evaluate a read.
        "evaluate" => {
            let mut transact = uri
                .query()
                .unwrap_or("")
                .split('&')
                .filter(|pair| pair.split('=').next() == Some("transact"))
                .peekable();
            transact.peek().is_some() && transact.all(|pair| pair == "transact=false")
        }
        _ => false,
    }
}

fn failure(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "error": message, "code": code }).to_string();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

#[cfg(test)]
mod tests;
