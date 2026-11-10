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
//! space and session drives the only kernel that signs as it: the space's
//! own kernel, which its host boots with the writer key and the chain down
//! to it ([`Kernels`]). Its writes take a lock from the pull before the
//! commit through the push after it, and answer only once the push has
//! landed; a write answered as failed never lands later. Reads never wait
//! for a write.
//!
//! The writer reaches its kernel only through the routes of its space, as
//! a page of the space would: `sync/pull`, `sync/push`, `sync/status`,
//! `evaluate`, `blob`, and, of the agent space's own kernel, its access,
//! its chains and its grants.
//!
//! Nothing here knows about Durable Objects or where a kernel runs: the
//! Host Worker keeps one writer per object, over the kernel it boots in
//! the isolate, and native tests boot the same kernel on Wasmtime.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::future::Future;
use std::rc::Rc;

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, Uri, header};
use dialog_credentials::{Ed25519Signer, Signer};
use dialog_ucan_core::subject::Subject;
use dialog_ucan_core::{Container, DelegationBuilder, DelegationChain, Tag};
use dialog_varsig::{Did, Principal as _};
use futures_util::lock::Mutex;
use http_body_util::BodyExt as _;
use ipld_core::ipld::Ipld;
use serde_json::{Value, json};
use tonk_worker_api::agent::AgentAccess;
use zeroize::Zeroizing;

use crate::telemetry::Steps;

mod caps;
pub use caps::ReadCaps;

/// The context the writer key is derived under, in BLAKE3's
/// `derive_key` form. Changing it moves every session's writer to a new
/// DID, and so starts a new origin in every space it writes.
const WRITER_KEY_CONTEXT: &str = "tonk 2026-10-29 hosted agent space writer key v1";

/// The branch a writer serves of its space.
const BRANCH: &str = "main";

/// The delegation meta naming whom a space's worker acts for.
pub(crate) const ACTS_FOR: &str = "acts.for";

/// The header that tells a space's kernel its caller syncs the space, so it
/// follows a request with no sync of its own: a write answered as failed
/// must never land by the kernel's own push.
const SYNC_HEADER: &str = "x-tonk-sync";

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
    /// Where the agent space syncs: the powerline's `home.address`.
    pub home: url::Url,
}

impl WriterSession {
    /// The agent space: the powerline's issuer.
    pub fn agent(&self) -> &Did {
        self.powerline.issuer()
    }
}

/// The key one MCP session writes a space as, and the chain that lets it.
pub struct WriterKey {
    seed: Zeroizing<[u8; 32]>,
    did: Did,
    chain: DelegationChain,
}

impl WriterKey {
    /// Derive the writer key of `session` and have the session delegate
    /// to it: the session's powerline extended by `session -> writer`, so
    /// revoking the session cuts every writer under it. The last hop says
    /// whom the writer acts for, the agent space, and where the space it
    /// writes syncs, `remote`, which is what a space's kernel takes a
    /// delegation up with.
    pub async fn derive(session: &WriterSession, remote: &url::Url) -> Result<Self, WriterError> {
        let internal = |error: &dyn std::fmt::Display| WriterError::Internal(error.to_string());
        let seed = writer_seed(&session.seed, &session.mcp_session);
        let did = Ed25519Signer::import(&*seed)
            .await
            .map_err(|error| internal(&error))?
            .did();
        let session_key = Ed25519Signer::import(&*session.seed)
            .await
            .map_err(|error| internal(&error))?;
        if session.powerline.audience() != &session_key.did() {
            return Err(WriterError::Forbidden(
                "the powerline is not addressed to the session's key".into(),
            ));
        }
        let mut meta = tonk_invite::home_address_meta(remote);
        meta.insert(
            ACTS_FOR.to_owned(),
            Ipld::String(session.agent().to_string()),
        );
        let hop = DelegationBuilder::new()
            .issuer(Signer::from(session_key))
            .audience(&did)
            .subject(Subject::Any)
            .command(vec![])
            .meta(meta)
            .try_build()
            .await
            .map_err(|error| internal(&error))?;
        let chain = session
            .powerline
            .clone()
            .push(hop)
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

    /// `grant` (`space -> ... -> agent space`) carried on down to the
    /// writer.
    fn extend(&self, grant: &DelegationChain) -> Result<DelegationChain, WriterError> {
        self.chain
            .proofs()
            .try_fold(grant.clone(), |chain, hop| chain.push(hop.clone()))
            .map_err(|error| WriterError::Internal(error.to_string()))
    }
}

/// Why a writer could not do what it was asked.
#[derive(Debug, Clone)]
pub enum WriterError {
    /// The access service answered `status`, with its `code`: a revoked
    /// grant, say.
    Upstream {
        status: u16,
        code: Option<String>,
        message: String,
    },
    /// The writer may not do it.
    Forbidden(String),
    /// Anything else.
    Internal(String),
}

impl std::fmt::Display for WriterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upstream { message, .. } | Self::Forbidden(message) | Self::Internal(message) => {
                formatter.write_str(message)
            }
        }
    }
}

/// A space's kernel, booted for one writer: what answers the routes of the
/// space as a page of it would.
pub trait SpaceKernel {
    /// Answer `request`, a route of the kernel's API.
    fn call(&self, request: Request<Body>) -> impl Future<Output = Response<Body>>;
}

/// What boots a space's kernel: over storage of its own, acting as the key
/// whose seed is `seed`, holding `chain`, the delegation of `space` down to
/// that key.
pub trait Kernels {
    /// The kernel it boots.
    type Kernel: SpaceKernel;

    /// A kernel of `space`, booted fresh.
    fn boot(
        &self,
        space: &Did,
        seed: &[u8; 32],
        chain: &DelegationChain,
    ) -> impl Future<Output = Result<Self::Kernel, String>>;
}

/// A warm kernel for one space and one MCP session, signing as that
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
/// kernel, and what it committed with it, and boots a new one from the
/// remote before it lets go of the lock. Dialog cannot rewind a branch: a
/// reset backwards leaves the version it rewound past in the kernel's
/// caches, and the next commit mints that version again (see dialog's
/// `Branch::reset`). A kernel booted fresh has seen only what the remote
/// holds.
///
/// Reads never wait for a write. A read pulls first when no write holds
/// the lock, and reads what is local otherwise: the write in flight is
/// pulling and pushing itself, and a pull in the middle of it would merge
/// into its unpushed commit and mint a revision on a read's behalf. Reads
/// do wait for one another's pulls, and share one: a pull that began
/// after a read did serves that read too.
pub struct SpaceWriter<K: Kernels> {
    kernels: K,
    /// The kernel, replaced when a write that did not land is dropped.
    kernel: RefCell<Rc<K::Kernel>>,
    key: WriterKey,
    space: Did,
    /// The agent space.
    agent: Did,
    /// The grants (`space -> ... -> agent space`, each a base64url
    /// container) the kernel holds, as the front last found them.
    admitted: RefCell<BTreeSet<String>>,
    base: String,
    lock: Mutex<()>,
    /// A write did not land and no new kernel could be booted in its
    /// place, so the current one may hold it: no write runs until one is.
    stale: Cell<bool>,
    /// Taken by a read while it pulls, so reads that arrive meanwhile
    /// wait for that pull instead of reading what was there before it.
    pulls: Mutex<()>,
    /// How many reads have begun; each takes the next number.
    reads: Cell<u64>,
    /// Reads numbered below this began before a pull that has landed.
    pulled_for: Cell<u64>,
    /// The most one read answers.
    caps: ReadCaps,
    /// Fail the next push before it is sent, as a refusing remote would.
    #[cfg(test)]
    refuse_push: Cell<bool>,
}

impl<K: Kernels> SpaceWriter<K> {
    /// Open the writer of `space` for `session`, its kernel booted by
    /// `kernels`, syncing with `remote` (the space's UCAN endpoint), and
    /// acting through `grants`, the agent space's grants of `space` (each a
    /// chain `space -> ... -> agent space` as a base64url container).
    ///
    /// The agent space's own writer acts through the session's powerline.
    /// The writer of a space with no grant still opens, and the access
    /// service refuses its first pull.
    pub async fn open(
        kernels: K,
        session: &WriterSession,
        space: &Did,
        remote: &url::Url,
        grants: &[String],
    ) -> Result<Self, WriterError> {
        let key = WriterKey::derive(session, remote).await?;
        let agent = session.agent().clone();
        let kernel = boot(&kernels, &key, space, &agent, grants).await?;
        Ok(Self {
            kernels,
            kernel: RefCell::new(Rc::new(kernel)),
            key,
            space: space.clone(),
            agent,
            admitted: RefCell::new(grants.iter().cloned().collect()),
            base: format!("/api/repository/{space}/branch/{BRANCH}"),
            lock: Mutex::new(()),
            stale: Cell::new(false),
            pulls: Mutex::new(()),
            reads: Cell::new(0),
            pulled_for: Cell::new(0),
            caps: ReadCaps::default(),
            #[cfg(test)]
            refuse_push: Cell::new(false),
        })
    }

    /// This writer, answering reads up to `caps`.
    pub fn with_read_caps(mut self, caps: ReadCaps) -> Self {
        self.caps = caps;
        self
    }

    /// The space this writer writes.
    pub fn space(&self) -> &Did {
        &self.space
    }

    /// The writer key's DID, which the kernel acts as.
    pub fn did(&self) -> &Did {
        &self.key.did
    }

    /// Commit one write and push it, holding the lock throughout.
    ///
    /// `request` names a route of the space's branch relative to it, such
    /// as `/evaluate?transact=true` or `/blob`. The answer is the route's
    /// own, sent once the push has landed. A write whose push failed is
    /// answered with `502` and `WRITE_NOT_LANDED`, and is gone: the next
    /// write commits on the remote's head, without it.
    ///
    /// Every answer carries the time of each step in `Server-Timing`:
    /// `lock` (waiting for the write before it), `reopen`, `pull`, `eval`,
    /// `push` and `landed`, each when it ran.
    pub async fn write(&self, request: Request<Body>) -> Response<Body> {
        let mut steps = Steps::start();
        let mut response = self.write_timed(request, &mut steps).await;
        steps.insert(response.headers_mut());
        response
    }

    async fn write_timed(&self, request: Request<Body>, steps: &mut Steps) -> Response<Body> {
        let _turn = self.lock.lock().await;
        steps.lap("lock");
        if self.stale.get() {
            let reopened = self.reopen().await;
            steps.lap("reopen");
            if let Err(error) = reopened {
                return failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "WRITER_UNAVAILABLE",
                    &error,
                );
            }
            self.stale.set(false);
        }
        let kernel = self.kernel();
        let pulled = self.sync(&kernel, "pull").await;
        steps.lap("pull");
        if let Err(error) = pulled {
            return error.answer("PULL_FAILED");
        }
        let response = call(&*kernel, self.on_branch(request)).await;
        if !response.status().is_success() {
            steps.lap("eval");
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
        steps.lap("eval");
        if let Some(failed) = self.land(&kernel, steps).await {
            return failed;
        }
        Response::from_parts(parts, Body::from(body))
    }

    /// Push what `kernel` committed, holding the lock: nothing once it
    /// landed, or the answer of the failure, the kernel having been replaced
    /// by one booted fresh from the remote, so the commit is gone.
    async fn land(&self, kernel: &K::Kernel, steps: &mut Steps) -> Option<Response<Body>> {
        let pushed = self.push(kernel).await;
        steps.lap("push");
        let Err(error) = pushed else {
            return None;
        };
        // A push can fail after its publish reached the remote, when only
        // the answer was lost: then the write did land.
        let landed = self.landed(kernel).await;
        steps.lap("landed");
        if landed {
            return None;
        }
        if let Err(reopen) = self.reopen().await {
            self.stale.set(true);
            return Some(error.answer_with(
                "WRITE_NOT_LANDED",
                &format!("{error}; the writer reopens before its next write: {reopen}"),
            ));
        }
        Some(error.answer("WRITE_NOT_LANDED"))
    }

    /// Answer one read without waiting for the lock.
    ///
    /// Only routes that cannot commit are served: `GET` and `HEAD`, a
    /// `query`, and an `evaluate` with `transact=false`. Anything else is
    /// refused with `405`, since a commit here would skip the lock and the
    /// push.
    ///
    /// A `query` or `evaluate` answers at most the writer's [`ReadCaps`],
    /// and is refused over them unless it asks for a page with `limit`
    /// and `cursor` (see [`caps`]).
    ///
    /// Every answer carries the time of each step in `Server-Timing`:
    /// `pull` (including waiting for a pull another read began) and
    /// `eval`.
    pub async fn read(&self, request: Request<Body>) -> Response<Body> {
        let mut steps = Steps::start();
        let mut response = self.read_timed(request, &mut steps).await;
        steps.insert(response.headers_mut());
        response
    }

    async fn read_timed(&self, request: Request<Body>, steps: &mut Steps) -> Response<Body> {
        let (mut parts, body) = request.into_parts();
        let page = match caps::Page::take(&parts.uri) {
            Ok((page, uri)) => {
                parts.uri = uri;
                page
            }
            Err(error) => return failure(StatusCode::BAD_REQUEST, "BAD_PAGE", &error),
        };
        let request = Request::from_parts(parts, body);
        if !is_read(request.method(), request.uri()) {
            return failure(
                StatusCode::METHOD_NOT_ALLOWED,
                "NOT_A_READ",
                "this route can commit; send it as a write",
            );
        }
        steps.skip();
        let refreshed = self.refresh().await;
        steps.lap("pull");
        if let Err(error) = refreshed {
            return error.answer("PULL_FAILED");
        }
        let capped = request.method() == Method::POST;
        let response = call(&*self.kernel(), self.on_branch(request)).await;
        let response = if capped {
            caps::cap(response, self.caps, &page).await
        } else {
            response
        };
        steps.lap("eval");
        response
    }

    /// The views `route` mounts in this writer's space (its home when
    /// `route` is empty), read after a pull as any other read is.
    pub async fn outline(&self, route: &str) -> Result<Value, String> {
        self.refresh().await?;
        let route: String = url::form_urlencoded::byte_serialize(route.as_bytes()).collect();
        let (status, body) = self
            .ask(
                Method::GET,
                &format!("/api/repository/{}/outline?route={route}", self.space),
                Body::empty(),
            )
            .await;
        if !status.is_success() {
            return Err(error_message(&body));
        }
        Ok(body)
    }

    /// The spaces the agent space reaches, read from it after a pull (a
    /// read's pull: never one that would merge into a write in flight).
    ///
    /// Only the agent space's own writer answers: it already holds the
    /// agent space warm, so the front asks it rather than booting a kernel.
    pub async fn agent_access(&self) -> Result<AgentAccess, WriterError> {
        self.only_the_agent()?;
        self.refresh()
            .await
            .map_err(SyncFailure::into_writer_error)?;
        let (status, body) = self
            .ask(
                Method::GET,
                &format!("/api/repository/{}/agent/access", self.agent),
                Body::empty(),
            )
            .await;
        if !status.is_success() {
            return Err(WriterError::Internal(error_message(&body)));
        }
        serde_json::from_value(body).map_err(|error| WriterError::Internal(error.to_string()))
    }

    /// The grants in force the agent space holds for `space`, read from
    /// it after a pull, as [`agent_access`](Self::agent_access) reads
    /// them: what a view's delegation of `space` extends.
    pub async fn space_chains(&self, space: &Did) -> Result<Vec<DelegationChain>, WriterError> {
        self.only_the_agent()?;
        self.refresh()
            .await
            .map_err(SyncFailure::into_writer_error)?;
        let (status, body) = self
            .ask(
                Method::GET,
                &format!("/api/repository/{}/agent/chains?space={space}", self.agent),
                Body::empty(),
            )
            .await;
        if !status.is_success() {
            return Err(WriterError::Internal(error_message(&body)));
        }
        body["chains"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(decode)
            .collect()
    }

    /// Make sure the kernel holds `grants`, the agent space's grants of
    /// this space the front found (each a base64url container): a grant
    /// approved or renewed while the writer stays warm reaches it this way.
    ///
    /// Handing the kernel more grants takes the write lock, so they never
    /// land under a write's proofs.
    pub async fn admit(&self, grants: &[String]) -> Result<(), WriterError> {
        if self.space == self.agent {
            return Ok(());
        }
        let new = |admitted: &BTreeSet<String>| -> Vec<String> {
            grants
                .iter()
                .filter(|grant| !admitted.contains(*grant))
                .cloned()
                .collect()
        };
        if new(&self.admitted.borrow()).is_empty() {
            return Ok(());
        }
        let _turn = self.lock.lock().await;
        let fresh = new(&self.admitted.borrow());
        if fresh.is_empty() {
            return Ok(());
        }
        hand(&*self.kernel(), &self.key, &self.space, &fresh).await?;
        self.admitted.borrow_mut().extend(fresh);
        Ok(())
    }

    /// The branch's sync status: its local head and the remote's, read
    /// without moving either.
    pub async fn status(&self) -> Result<tonk_worker_api::SyncStatusResponse, WriterError> {
        status(&*self.kernel(), &self.base).await
    }

    /// Whether the lock is held: a write is in flight, or a read is
    /// pulling under it. A writer that holds it must not be dropped, since
    /// a write between its commit and its push would be lost.
    pub fn holds_write(&self) -> bool {
        self.lock.try_lock().is_none()
    }

    fn kernel(&self) -> Rc<K::Kernel> {
        self.kernel.borrow().clone()
    }

    /// Ask the kernel `method` `path` and answer its status and body.
    async fn ask(&self, method: Method, path: &str, body: Body) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap_or_else(|_| Request::new(Body::empty()));
        let response = call(&*self.kernel(), request).await;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        (
            status,
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    }

    fn only_the_agent(&self) -> Result<(), WriterError> {
        if self.space != self.agent {
            return Err(WriterError::Forbidden(
                "only the agent space's writer reads its grants".into(),
            ));
        }
        Ok(())
    }

    /// Whether the remote's head is `kernel`'s own, as it is when a push
    /// published and only its answer was lost. Unknown counts as no.
    async fn landed(&self, kernel: &K::Kernel) -> bool {
        match status(kernel, &self.base).await {
            Ok(status) => status.local.is_some() && status.local == status.remote,
            Err(_) => false,
        }
    }

    /// Drop the kernel and what it committed, and boot a new one, pulled to
    /// the remote's head. Called with the lock held.
    async fn reopen(&self) -> Result<(), String> {
        let grants: Vec<String> = self.admitted.borrow().iter().cloned().collect();
        let kernel = boot(&self.kernels, &self.key, &self.space, &self.agent, &grants)
            .await
            .map_err(|error| error.to_string())?;
        self.sync(&kernel, "pull").await?;
        *self.kernel.borrow_mut() = Rc::new(kernel);
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
    async fn refresh(&self) -> Result<(), SyncFailure> {
        let read = self.reads.get();
        self.reads.set(read + 1);
        let _pulling = self.pulls.lock().await;
        if read < self.pulled_for.get() {
            return Ok(());
        }
        let Some(_turn) = self.lock.try_lock() else {
            return Ok(());
        };
        let covers = self.reads.get();
        self.sync(&*self.kernel(), "pull").await?;
        self.pulled_for.set(self.pulled_for.get().max(covers));
        Ok(())
    }

    async fn push(&self, kernel: &K::Kernel) -> Result<(), SyncFailure> {
        #[cfg(test)]
        if self.refuse_push.replace(false) {
            return Err(SyncFailure::local("push: refused by the test"));
        }
        self.sync(kernel, "push").await
    }

    /// Pull or push the space's branch. With no grant of a space other than
    /// the agent space, its kernel holds no space to sync, and nothing could
    /// reach it: refused as the access service refuses an ungranted pull.
    async fn sync(&self, kernel: &K::Kernel, operation: &str) -> Result<(), SyncFailure> {
        if self.space != self.agent && self.admitted.borrow().is_empty() {
            return Err(SyncFailure {
                code: Some(ACCESS_DENIED.to_owned()),
                message: format!("{operation}: no grant reaches the space"),
            });
        }
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("{}/sync/{operation}", self.base))
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::empty())
            .map_err(|error| SyncFailure::local(error.to_string()))?;
        let response = call(kernel, request).await;
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
        Err(SyncFailure::answered(operation, status, &body))
    }
}

/// Boot a kernel of `space` acting as `key`: through the session's
/// powerline when `space` is the agent space, through the first of `grants`
/// otherwise, with the rest handed to it after.
async fn boot<K: Kernels>(
    kernels: &K,
    key: &WriterKey,
    space: &Did,
    agent: &Did,
    grants: &[String],
) -> Result<K::Kernel, WriterError> {
    let (chain, rest) = match grants.split_first() {
        Some((first, rest)) if space != agent => (key.extend(&decode(first)?)?, rest),
        _ => (key.chain.clone(), &[][..]),
    };
    let kernel = kernels
        .boot(space, &key.seed, &chain)
        .await
        .map_err(WriterError::Internal)?;
    hand(&kernel, key, space, rest).await?;
    Ok(kernel)
}

/// Hand `kernel` more of `space`'s grants, each carried down to `key`.
async fn hand<C: SpaceKernel>(
    kernel: &C,
    key: &WriterKey,
    space: &Did,
    grants: &[String],
) -> Result<(), WriterError> {
    if grants.is_empty() {
        return Ok(());
    }
    let chains = grants
        .iter()
        .map(|grant| encode(&key.extend(&decode(grant)?)?))
        .collect::<Result<Vec<_>, _>>()?;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/repository/{space}/delegations"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "chains": chains }).to_string()))
        .map_err(|error| WriterError::Internal(error.to_string()))?;
    let response = call(kernel, request).await;
    if response.status().is_success() {
        return Ok(());
    }
    let body = response
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    Err(WriterError::Internal(format!(
        "the kernel did not take the grants: {}",
        String::from_utf8_lossy(&body)
    )))
}

/// `request`, answered by `kernel`, which is told its caller syncs.
async fn call<C: SpaceKernel>(kernel: &C, mut request: Request<Body>) -> Response<Body> {
    request
        .headers_mut()
        .insert(SYNC_HEADER, header::HeaderValue::from_static("caller"));
    kernel.call(request).await
}

/// A base64url container, as a chain.
fn decode(encoded: &str) -> Result<DelegationChain, WriterError> {
    Container::decode(encoded.as_bytes())
        .ok()
        .and_then(|container| DelegationChain::try_from(container).ok())
        .ok_or_else(|| WriterError::Internal("a grant is not a delegation chain".into()))
}

/// `chain` as a base64url container.
fn encode(chain: &DelegationChain) -> Result<String, WriterError> {
    let bytes = Container::from(chain)
        .encode(Tag::Base64Url)
        .map_err(|error| WriterError::Internal(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| WriterError::Internal(error.to_string()))
}

/// The message of a kernel's error answer.
fn error_message(body: &Value) -> String {
    body["error"]["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .or_else(|| body.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| body.to_string())
}

/// The code a pull or push answers when the access service refused it
/// because a grant it proves through was revoked.
pub const CREDENTIAL_REVOKED: &str = "CREDENTIAL_REVOKED";

/// The code a pull or push answers when no chain the kernel holds reaches
/// the space: what a revoked grant becomes once its revoked chain is
/// dropped.
pub const ACCESS_DENIED: &str = "ACCESS_DENIED";

/// A pull or push that did not succeed, with the reason the branch's
/// sync route gave, so a revocation is answered as one rather than as a
/// failed sync.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    /// The sync route's error code (`error.code` in its body).
    code: Option<String>,
    message: String,
}

impl SyncFailure {
    fn local(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    /// The failure a sync route answered with `status` and `body`.
    pub fn answered(operation: &str, status: StatusCode, body: &[u8]) -> Self {
        let code = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|body| body["error"]["code"].as_str().map(str::to_owned));
        Self {
            code,
            message: format!("{operation}: {status}: {}", String::from_utf8_lossy(body)),
        }
    }

    /// Whether the access service refused it for a revoked grant.
    pub fn revoked(&self) -> bool {
        self.code.as_deref() == Some(CREDENTIAL_REVOKED)
    }

    /// The refusal's code when access was refused (a revoked grant, or no
    /// grant left), rather than the sync failing.
    fn refusal(&self) -> Option<&'static str> {
        match self.code.as_deref() {
            Some(CREDENTIAL_REVOKED) => Some(CREDENTIAL_REVOKED),
            Some(ACCESS_DENIED) => Some(ACCESS_DENIED),
            _ => None,
        }
    }

    /// The writer's answer: `403` and the refusal's code when access was
    /// refused, `502` and `code` otherwise.
    fn answer(&self, code: &str) -> Response<Body> {
        self.answer_with(code, &self.message)
    }

    /// [`answer`](Self::answer), with `message`.
    fn answer_with(&self, code: &str, message: &str) -> Response<Body> {
        match self.refusal() {
            Some(refusal) => failure(StatusCode::FORBIDDEN, refusal, message),
            None => failure(StatusCode::BAD_GATEWAY, code, message),
        }
    }

    /// As a writer error: [`WriterError::Upstream`] with `403` when access
    /// was refused, so a caller can tell it from a failure.
    fn into_writer_error(self) -> WriterError {
        if self.refusal().is_some() {
            WriterError::Upstream {
                status: StatusCode::FORBIDDEN.as_u16(),
                code: self.code,
                message: self.message,
            }
        } else {
            WriterError::Internal(self.message)
        }
    }
}

impl std::fmt::Display for SyncFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl From<SyncFailure> for String {
    fn from(failure: SyncFailure) -> Self {
        failure.message
    }
}

/// `kernel`'s sync status for the branch at `base`.
async fn status<C: SpaceKernel>(
    kernel: &C,
    base: &str,
) -> Result<tonk_worker_api::SyncStatusResponse, WriterError> {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("{base}/sync/status"))
        .body(Body::empty())
        .map_err(|error| WriterError::Internal(error.to_string()))?;
    let response = call(kernel, request).await;
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .map_err(|error| WriterError::Internal(error.to_string()))?
        .to_bytes();
    if !status.is_success() {
        return Err(WriterError::Internal(format!(
            "sync status: {status}: {}",
            String::from_utf8_lossy(&body)
        )));
    }
    serde_json::from_slice(&body).map_err(|error| WriterError::Internal(error.to_string()))
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
    let body = json!({ "error": message, "code": code }).to_string();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

impl<K: Kernels> SpaceWriter<K> {
    /// Keep what a join from the chat grants the agent space (see
    /// [`crate::join`]): the chains in `body`, each checked to reach the
    /// agent space and grant the space named, and the space's directory
    /// entry.
    pub(crate) async fn keep_grant(&self, body: &crate::join::GrantBody) -> Result<(), String> {
        let (chains, entry) = body.open(&self.agent)?;
        self.change(json!({
            "retain": chains.iter().map(base58).collect::<Result<Vec<_>, _>>()?,
            "directory": [entry],
        }))
        .await
    }

    /// Read an invite link the person pasted, and when `request` names an
    /// access, join with it: the link's key delegates that access to the
    /// agent space, which keeps it (see [`crate::join`]). Only the agent
    /// space's own writer answers. A refusal is a sentence for the person.
    pub async fn join_link(
        &self,
        request: &crate::join::LinkRequest,
    ) -> Result<crate::join::LinkAnswer, String> {
        use tonk_invite::connection::AgentAccess as Level;
        if self.space != self.agent {
            return Err("Only the agent's own space joins spaces.".into());
        }
        let offer = crate::join::read_link(&request.link).await?;
        let granted = match request.access.as_deref() {
            None => None,
            Some(access) => {
                let level = match access {
                    "read" => Level::Read,
                    "write" => Level::ReadWrite,
                    _ => return Err("Choose read or write access.".into()),
                };
                let chains = offer
                    .grant(&self.agent, level, dialog_ucan_core::time::Timestamp::now())
                    .await?;
                let body = crate::join::GrantBody::new(&offer, &chains)?;
                self.keep_grant(&body)
                    .await
                    .map_err(|error| format!("Could not join: {error}"))?;
                Some(access.to_owned())
            }
        };
        Ok(crate::join::LinkAnswer {
            space: offer.subject.to_string(),
            name: offer.name.clone(),
            write: offer.write,
            granted,
        })
    }

    /// Change what the agent space holds, as the approval does (`retain`,
    /// `forget` and `directory`; see the worker's `router/agent_space.rs`),
    /// under the lock, and push. Only the agent space's own writer changes
    /// it.
    async fn change(&self, change: Value) -> Result<(), String> {
        if self.space != self.agent {
            return Err("only the agent space's writer keeps its grants".into());
        }
        let _turn = self.lock.lock().await;
        let kernel = self.kernel();
        self.sync(&kernel, "pull").await?;
        let (status, body) = self
            .ask(
                Method::POST,
                &format!("/api/repository/{}/agent", self.agent),
                Body::from(change.to_string()),
            )
            .await;
        if !status.is_success() {
            return Err(error_message(&body));
        }
        let mut steps = Steps::start();
        match self.land(&kernel, &mut steps).await {
            None => Ok(()),
            Some(_) => Err("the agent space's grants did not reach its remote; try again".into()),
        }
    }

    /// Grant the agent space, as the approval does: retain `chains` and
    /// record `directory` (each `{space, name, home}`), and push.
    #[cfg(test)]
    pub(crate) async fn approve(
        &self,
        chains: &[DelegationChain],
        directory: Value,
    ) -> Result<(), String> {
        self.change(json!({
            "retain": chains.iter().map(base58).collect::<Result<Vec<_>, _>>()?,
            "directory": directory,
        }))
        .await
    }

    /// Drop `chains` from the agent space and push, as revoking a grant
    /// joined from a share link does: the access service is never told, so
    /// only the agent space says it is gone.
    #[cfg(test)]
    pub(crate) async fn withdraw(&self, chains: &[DelegationChain]) -> Result<(), String> {
        self.change(json!({
            "forget": chains.iter().map(base58).collect::<Result<Vec<_>, _>>()?,
        }))
        .await
    }
}

/// Base58 of `chain`'s bytes, as the agent space's change takes it.
fn base58(chain: &DelegationChain) -> Result<String, String> {
    chain
        .to_bytes()
        .map(|bytes| bs58::encode(bytes).into_string())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
