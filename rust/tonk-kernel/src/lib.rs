//! The Tonk kernel: Tonk's worker, as a component of the world
//! `sapling:kernel`. Its storage is the layers its host gives it, and its keys
//! are the session's and the ones it entrusts to it (`tonk-kernel-world`). It
//! answers each request of its origin with the worker's own API router, over
//! `wasi:http/handler`.
//!
//! Built with the feature `persona`, it targets `sapling:kernel/persona`: the
//! person's own worker, which serves their profile and delegates to their
//! other workers what their hosts ask for (`guest`). Without it,
//! `sapling:kernel/kernel`: a space's own worker, which acts on what its
//! representative delegates to it.
//!
//! Built for `wasm32-wasip3` by `tonk-components`; on every other target the
//! crate is empty.

#![cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]

pub mod world {
    #[cfg(feature = "persona")]
    wit_bindgen::generate!({
        path: "../tonk-kernel-world/wit",
        world: "sapling:kernel/persona",
        with: {
            "sapling:kernel/types@0.1.0": tonk_kernel_world::world::sapling::kernel::types,
            "sapling:kernel/storage@0.1.0": tonk_kernel_world::world::sapling::kernel::storage,
            "sapling:kernel/network@0.1.0": tonk_kernel_world::world::sapling::kernel::network,
            "sapling:kernel/credential@0.1.0": tonk_kernel_world::world::sapling::kernel::credential,
            "sapling:kernel/session@0.1.0": tonk_kernel_world::world::sapling::kernel::session,
            "sapling:kernel/application@0.1.0": tonk_kernel_world::world::sapling::kernel::application,
        },
        generate_all,
    });
    #[cfg(not(feature = "persona"))]
    wit_bindgen::generate!({
        path: "../tonk-kernel-world/wit",
        world: "sapling:kernel/kernel",
        with: {
            "sapling:kernel/types@0.1.0": tonk_kernel_world::world::sapling::kernel::types,
            "sapling:kernel/storage@0.1.0": tonk_kernel_world::world::sapling::kernel::storage,
            "sapling:kernel/network@0.1.0": tonk_kernel_world::world::sapling::kernel::network,
            "sapling:kernel/credential@0.1.0": tonk_kernel_world::world::sapling::kernel::credential,
            "sapling:kernel/session@0.1.0": tonk_kernel_world::world::sapling::kernel::session,
            "sapling:kernel/application@0.1.0": tonk_kernel_world::world::sapling::kernel::application,
        },
        generate_all,
    });
}

mod http;
mod site;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use site::{Routed, Site, SpaceAsk};
use tonk_kernel_world::application::Told;
use tonk_kernel_world::boot;
use tonk_kernel_world::guest::HandleLayer;
use tonk_kernel_world::layer::{SharedLayer, share};
use tonk_kernel_world::signal::Signal;
use tonk_kernel_world::world::sapling::kernel::credential::{Access, PersonaAuthority};
use tonk_kernel_world::world::sapling::kernel::types::Error;
use tonk_worker::axum::RequestOrigin;
use tonk_worker::{AppState, ClientId, TonkState};
use tower::ServiceExt as _;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::wasi::http::types::{ErrorCode, Request, Response, Scheme};

/// The worker's router and state, once `init` opened them. A `static`, not
/// a thread-local: on `wasm32-wasip3` each export call is a task of its own.
static ROUTER: Mutex<Option<Held>> = Mutex::new(None);

/// Whether the space taken up for the first time is still being created:
/// a request waits until it is, so nothing it writes races the creation.
static CREATING: AtomicBool = AtomicBool::new(false);
static CREATED: OnceLock<Signal> = OnceLock::new();

fn created() -> &'static Signal {
    CREATED.get_or_init(Signal::new)
}

/// The router holds Dialog DB's state, which is not `Send`.
struct Held {
    router: Router,
    site: Site,
    state: AppState,
}

/// Whose worker this kernel is: the person's own, for a persona; else a
/// space's own.
#[cfg(feature = "persona")]
const STANDING: tonk_worker::Standing = tonk_worker::Standing::Person;
#[cfg(not(feature = "persona"))]
const STANDING: tonk_worker::Standing = tonk_worker::Standing::Site;

/// The space this kernel serves: its subject, and the layer the boot gave it,
/// which holds the space's boot cell and the kernel's wasm, and everything
/// else the kernel keeps.
#[derive(Clone)]
struct Space {
    subject: String,
    layer: SharedLayer,
}

// SAFETY: a component instance runs on a single thread; nothing here is
// reached from another.
unsafe impl Send for Held {}

fn router() -> Option<(Router, Site)> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| (held.router.clone(), held.site.clone()))
}

fn state() -> Option<AppState> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| held.state.clone())
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

struct Kernel;

impl lifecycle::Guest for Kernel {
    async fn init(boot: lifecycle::Boot) -> Result<(), Error> {
        let started = world::wasi::clocks::monotonic_clock::now();
        tonk_kernel_world::session::take_up(boot.session);
        // The space this kernel serves: the subject of its `memory:space`.
        let space = boot
            .stack
            .into_iter()
            .find(|stacked| stacked.scopes.iter().any(|scope| scope == "memory:space"))
            .map(|stacked| {
                let subject = stacked.layer.subject();
                HandleLayer::new(stacked.layer).map(|layer| Space {
                    subject,
                    layer: share(layer),
                })
            })
            .transpose()
            .map_err(invalid)?;
        // Everything the kernel keeps is in the layer of its stack.
        if let Some(space) = &space {
            tonk_kernel_world::stack::keep(space.layer.clone());
        }
        tonk_worker::set_standing(STANDING);
        // A persona serves the person's profile, which is its own: it takes
        // no space up.
        let space = space.filter(|_| STANDING == tonk_worker::Standing::Site);
        let state = TonkState::open_kernel().await.map_err(invalid)?;
        // The space, taken up with the delegation its session asks for, or
        // as it was taken up before, with the delegation asked for anew
        // alongside: put back what a restart lost about it, and give it its
        // first commit if it went without. A space taken up for the first
        // time is created from what the person's persona kept for it, once
        // the kernel serves, since the persona answers through the host.
        let (renewing, fresh) = match &space {
            Some(space) if state.acts().await.unwrap_or(false) => (Some(space.clone()), None),
            Some(space) => {
                take_up(&state, space).await;
                (None, Some(space.clone()))
            }
            None => (None, None),
        };
        if let Some(space) = &space {
            if let Err(error) = state.resume_space(&space.subject).await {
                eprintln!("tonk-kernel: {} was not resumed: {error}", space.subject);
            }
            if fresh.is_none()
                && let Err(error) = found(&state, space).await
            {
                eprintln!("tonk-kernel: {} was not founded: {error}", space.subject);
            }
        }
        let (router, state, _) = tonk_worker::api_router_with_state(state);
        let site = Site {
            space: space.as_ref().map(|space| space.subject.clone()),
        };
        *ROUTER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held {
            router,
            site,
            state: state.clone(),
        });
        if fresh.is_some() {
            CREATING.store(true, Ordering::Release);
        }
        wit_bindgen::spawn_local(configure());
        if let Some(space) = renewing {
            wit_bindgen::spawn_local(async move { renew(&state, &space.subject).await });
        } else if let Some(space) = fresh {
            wit_bindgen::spawn_local(async move { create(&state, &space).await });
        }
        let took = world::wasi::clocks::monotonic_clock::now() - started;
        tonk_common::log!("tonk-kernel: opened the worker in {}us", took / 1000);
        Ok(())
    }

    async fn sync(_changes: Vec<lifecycle::CellChange>) -> Result<(), Error> {
        Ok(())
    }

    /// The person signed in: the permanent persona delegates everything it
    /// has to the representative, which links this device to it, and the
    /// worker records the account and carries the provisional persona's
    /// spaces over to it.
    async fn merge(
        provisional: &PersonaAuthority,
        permanent: &PersonaAuthority,
    ) -> Result<(), Error> {
        let state =
            state().ok_or_else(|| Error::Unavailable("the persona has not booted".into()))?;
        let representative = tonk_kernel_world::session::representative().did();
        let access = Access {
            subject: None,
            command: "/".to_owned(),
            arguments: Vec::new(),
            reason: "to link this device to your account".to_owned(),
        };
        let link = permanent.delegate(representative, access).await?;
        tonk_worker::merged(&state, link, provisional, permanent)
            .await
            .map_err(|error| Error::Unavailable(error.to_string()))
    }

    async fn authorize(_persona: &PersonaAuthority, _context: Vec<u8>) -> Result<(), Error> {
        Err(Error::Unavailable(
            "this kernel acts as no persona yet".into(),
        ))
    }

    async fn shutdown(_deadline_ms: u32) {}
}

impl channel::Guest for Kernel {
    async fn receive(message: Vec<u8>) -> Result<(), Error> {
        let told = tonk_kernel_world::application::received(&message)?;
        let (Some(told), Some(state)) = (told, state()) else {
            return Ok(());
        };
        match told {
            Told::Renew => {
                if let Some(subject) = served_space() {
                    wit_bindgen::spawn_local(async move { renew(&state, &subject).await });
                }
            }
            Told::Refused { kind, name } => tonk_worker::refused(&state, &kind, &name).await,
            Told::Failed {
                kind,
                code,
                message,
            } => tonk_worker::failed(&state, &kind, code.as_deref(), &message).await,
        }
        Ok(())
    }
}

/// `request`, as the worker's router takes one, with the origin it was made
/// to, and the page that sent it, which its host names in
/// `x-tonk-client-id`, so a command can send that page somewhere.
async fn incoming(request: Request) -> Result<axum::http::Request<axum::body::Body>, ErrorCode> {
    let method = request.get_method();
    let target = request
        .get_path_with_query()
        .unwrap_or_else(|| "/".to_owned());
    let headers = request.get_headers().copy_all();
    let origin = match (request.get_scheme(), request.get_authority()) {
        (Some(scheme), Some(authority)) => {
            let scheme = match scheme {
                Scheme::Http => "http".to_owned(),
                Scheme::Https => "https".to_owned(),
                Scheme::Other(other) => other,
            };
            RequestOrigin::parse(&format!("{scheme}://{authority}/")).ok()
        }
        _ => None,
    };
    let body = http::read_body(request).await?;
    let mut built = axum::http::Request::builder()
        .method(http::method(&method))
        .uri(target);
    for (name, value) in headers {
        built = built.header(name, value);
    }
    let mut request = built
        .body(axum::body::Body::from(body))
        .map_err(|error| ErrorCode::InternalError(Some(error.to_string())))?;
    if let Some(client) = request
        .headers()
        .get("x-tonk-client-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    {
        request.extensions_mut().insert(ClientId(client));
    }
    if let Some(origin) = origin {
        request.extensions_mut().insert(origin);
    }
    Ok(request)
}

impl handler::Guest for Kernel {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let Some((router, site)) = router() else {
            return Ok(http::respond(503, Some("text/plain"), b"booting".to_vec()));
        };
        if CREATING.load(Ordering::Acquire) {
            created().changed(0).await;
        }
        let request = incoming(request).await?;
        let method = request.method().clone();
        let answer = match site::route(request, &site) {
            Routed::Answer(answer) => answer,
            Routed::Pass(request) => {
                let Ok(answer) = router.oneshot(request).await;
                answer
            }
            Routed::Visit { request, viewed } => {
                let Ok(answer) = router.oneshot(request).await;
                site::visited(answer, viewed)
            }
            Routed::Store(request) => {
                let Ok(answer) = router.oneshot(request).await;
                site::stored(answer).await
            }
            Routed::Seed(space) => seed_of(&space).await,
            Routed::Settle(space) => settle(&space).await,
            Routed::Snapshot(space) => snapshot(&space).await,
            Routed::SpaceAsk {
                space,
                ask,
                request,
            } => space_ask(&space, ask, request).await,
        };
        // A write, or a page's poke, is followed by a sync with the remote,
        // as the browser's worker drains its queue after each request: once
        // the write is done, so the sync does not move the branch under it.
        if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
            drain_soon();
        }
        let (parts, mut body) = answer.into_parts();
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
            .collect::<Vec<_>>();
        let (response, mut writer, _) = http::stream(parts.status.as_u16(), &headers);
        // The body goes as it comes, so a subscription keeps flowing.
        wit_bindgen::spawn_local(async move {
            while let Some(frame) = body.frame().await {
                let Ok(frame) = frame else { break };
                if let Ok(chunk) = frame.into_data() {
                    let rest = writer.write_all(chunk.to_vec()).await;
                    if !rest.is_empty() {
                        break;
                    }
                }
            }
        });
        Ok(response)
    }
}

/// Do for the space `space` what its own kernel asks of the persona: the
/// answer is `{ invited }`, `{ authority }` or `{ receipt }`, as a profile's
/// worker answered the same asks in a browser.
async fn space_ask(
    space: &str,
    ask: SpaceAsk,
    request: axum::http::Request<axum::body::Body>,
) -> axum::http::Response<axum::body::Body> {
    let Some(state) = state() else {
        return site::answer(StatusCode::SERVICE_UNAVAILABLE, &json!("booting"));
    };
    let body: Value = match request.into_body().collect().await {
        Ok(collected) => serde_json::from_slice(&collected.to_bytes()).unwrap_or(Value::Null),
        Err(error) => {
            return site::answer(StatusCode::BAD_REQUEST, &json!(error.to_string()));
        }
    };
    let answered = match ask {
        SpaceAsk::Invite => {
            let time = body.get("time").and_then(Value::as_f64).unwrap_or_default();
            tonk_worker::profile_asks::invite_space(&state, space, time)
                .await
                .map(|()| json!({ "invited": true }))
        }
        SpaceAsk::Authority => tonk_worker::profile_asks::space_authority(&state, space)
            .await
            .map(|authority| json!({ "authority": authority })),
        SpaceAsk::Revoke => tonk_worker::profile_asks::revoke_grant(&state, space, body)
            .await
            .map(|receipt| json!({ "receipt": receipt })),
    };
    match answered {
        Ok(answer) => site::answer(StatusCode::OK, &answer),
        Err(error) => site::answer(StatusCode::INTERNAL_SERVER_ERROR, &json!(error.to_string())),
    }
}

/// Sync with the remote shortly: after a moment, so a burst of writes is
/// one sync, and never two at once (see [`tonk_worker::drain_sync`]).
fn drain_soon() {
    static PENDING: AtomicBool = AtomicBool::new(false);
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    wit_bindgen::spawn_local(async {
        let _ = tonk_worker::sleep(Duration::from_millis(DRAIN_AFTER_MS)).await;
        PENDING.store(false, Ordering::Release);
        if let Some(state) = state() {
            tonk_worker::drain_sync(&state).await;
        }
    });
}

/// How long a sync waits after the write that asks for it.
const DRAIN_AFTER_MS: u64 = 500;

/// Ask for the delegation of `space` to this kernel's session, and take up
/// what the representative hands over.
async fn take_up(state: &TonkState, space: &Space) {
    let chains = delegated(&space.subject, "to serve the space").await;
    adopt(state, &space.subject, &chains).await;
}

/// Learn from the deployment's configuration, which the host reads, which
/// app it serves: where the access service answers, and where a link made
/// for the person opens.
async fn configure() {
    let read = tonk_kernel_world::application::read("/.well-known/tonk").await;
    let app = read.ok().and_then(|answer| {
        let configuration: Value = serde_json::from_str(&answer.body).ok()?;
        configuration
            .get("sites")?
            .get("app")?
            .as_str()
            .map(|app| app.trim_end_matches('/').to_owned())
    });
    match app {
        Some(app) => tonk_worker::set_app_origin(app),
        None => tonk_common::log!("tonk-kernel: the deployment names no app"),
    }
}

/// Ask for the delegation of `space` anew, as the kernel serves what it took
/// up before, so a delegation running out is replaced. Nothing is locked
/// while the host asks the person's profile.
async fn renew(state: &AppState, space: &str) {
    let chains = delegated(space, "to keep serving the space").await;
    adopt(&*state.read().await, space, &chains).await;
}

/// The space this kernel serves, if it serves one.
fn served_space() -> Option<String> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|held| held.site.space.clone())
}

/// The delegation of `space` to this kernel's session, as its representative
/// answers: none when it refuses, or fails to.
async fn delegated(space: &str, reason: &str) -> Vec<Vec<u8>> {
    let access = Access {
        subject: Some(space.to_owned()),
        command: "/".to_owned(),
        arguments: Vec::new(),
        reason: reason.to_owned(),
    };
    match tonk_kernel_world::session::delegate(access).await {
        Ok(chain) => vec![chain],
        Err(error) => {
            eprintln!("tonk-kernel: {space} was not delegated: {error:?}");
            Vec::new()
        }
    }
}

/// Take up `chains`. A chain is taken up only if the space signed it all the
/// way down to this kernel's session, so a host cannot make the kernel act
/// for another space.
async fn adopt(state: &TonkState, space: &str, chains: &[Vec<u8>]) {
    for chain in chains {
        if let Err(error) = state.adopt_space(space, chain).await {
            eprintln!("tonk-kernel: a delegation of {space} was not taken up: {error}");
        }
    }
}

/// Give the space its first commit, if it has none: the kernel its boot cell
/// pins, as an asset the space's layer already holds, and the pin to it. A
/// space whose layer holds no boot cell pins nothing, and is left alone.
async fn found(state: &TonkState, space: &Space) -> Result<(), String> {
    let Some((hash, size)) = pinned(space).await? else {
        return Ok(());
    };
    state
        .found_space(&space.subject, hash, size)
        .await
        .map_err(|error| error.to_string())
}

/// Create the space taken up for the first time: its content from what the
/// person's persona kept for it, or, when it kept nothing, as for a space
/// joined, only its first commit.
async fn create(state: &AppState, space: &Space) {
    let seeded = match seed(state, space).await {
        Ok(seeded) => seeded,
        Err(error) => {
            tonk_common::log!("tonk-kernel: {} was not seeded: {error}", space.subject);
            false
        }
    };
    if !seeded && let Err(error) = found(&*state.read().await, space).await {
        tonk_common::log!("tonk-kernel: {} was not founded: {error}", space.subject);
    }
    CREATING.store(false, Ordering::Release);
    created().notify();
}

/// Create the space's content from what the person's persona kept for it as
/// it created the space, with the kernel its boot cell pins recorded on top,
/// and tell the persona it was. Whether the persona kept anything. Nothing is
/// locked while the host asks the persona.
async fn seed(state: &AppState, space: &Space) -> Result<bool, String> {
    let Some((hash, size)) = pinned(space).await? else {
        return Ok(false);
    };
    let Some(seed) = tonk_kernel_world::application::seed()
        .await
        .map_err(|error| format!("the persona was not asked: {error:?}"))?
    else {
        tonk_common::log!(
            "tonk-kernel: the persona kept nothing to create {} from",
            space.subject
        );
        return Ok(false);
    };
    let tonk = state.read().await;
    let created = match &seed.copy {
        Some(copy) => {
            tonk.copy_space(
                &space.subject,
                &seed.seed,
                &copy.content,
                &copy.revision,
                hash,
                size,
            )
            .await
        }
        None => {
            tonk.seed_space(&space.subject, &seed.seed, hash, size)
                .await
        }
    };
    drop(tonk);
    created.map_err(|error| error.to_string())?;
    tonk_kernel_world::application::seeded()
        .await
        .map_err(|error| format!("the persona was not told: {error:?}"))?;
    Ok(true)
}

/// The kernel the space's boot cell pins, as its BLAKE3 hash and size, which
/// the space's layer holds whole. `None` for a space with no boot cell.
async fn pinned(space: &Space) -> Result<Option<([u8; 32], u64)>, String> {
    let cell = space
        .layer
        .resolve(boot::NAMESPACE, boot::CELL)
        .await
        .map_err(|error| format!("the boot cell: {error:?}"))?;
    let Some(cell) = cell else {
        return Ok(None);
    };
    let id = boot::read_cell(&cell.content)?;
    let hash = boot::digest(&id)?;
    let size = space
        .layer
        .size(&hash)
        .await
        .map_err(|error| format!("the kernel {id}: {error:?}"))?
        .ok_or_else(|| format!("the space does not hold the kernel {id} whole"))?;
    Ok(Some((hash, size)))
}

/// What the space `space` is to be created from: the seed this persona kept
/// as it created it, or `null`.
async fn seed_of(space: &str) -> axum::http::Response<axum::body::Body> {
    let Some(state) = state() else {
        return site::answer(StatusCode::SERVICE_UNAVAILABLE, &json!("booting"));
    };
    let pending = state.read().await.pending_seed(space).await;
    match pending {
        Ok(Some(seed)) => match serde_json::from_str(&seed) {
            Ok(seed) => site::answer(StatusCode::OK, &seed),
            Err(error) => {
                site::answer(StatusCode::INTERNAL_SERVER_ERROR, &json!(error.to_string()))
            }
        },
        Ok(None) => {
            eprintln!("tonk-kernel: no seed is kept for {space}");
            site::answer(StatusCode::OK, &Value::Null)
        }
        Err(error) => site::answer(StatusCode::INTERNAL_SERVER_ERROR, &json!(error.to_string())),
    }
}

/// The space `space` was created from its seed.
async fn settle(space: &str) -> axum::http::Response<axum::body::Body> {
    let Some(state) = state() else {
        return site::answer(StatusCode::SERVICE_UNAVAILABLE, &json!("booting"));
    };
    let settled = state.read().await.settle_seed(space).await;
    match settled {
        Ok(()) => site::answer(StatusCode::OK, &Value::Null),
        Err(error) => site::answer(StatusCode::INTERNAL_SERVER_ERROR, &json!(error.to_string())),
    }
}

/// The space `space`'s `main`, for another space to be made a copy of: the
/// CARv1 of everything its revision reaches, with the revision, JSON, in
/// base64 in `x-tonk-revision`. `404` for a space with nothing on `main`.
async fn snapshot(space: &str) -> axum::http::Response<axum::body::Body> {
    let Some(state) = state() else {
        return site::answer(StatusCode::SERVICE_UNAVAILABLE, &json!("booting"));
    };
    let snapshot = state.read().await.snapshot_space(space).await;
    match snapshot {
        Ok(Some((content, revision))) => site::snapshot(content, &revision),
        Ok(None) => site::answer(StatusCode::NOT_FOUND, &json!("the space is empty")),
        Err(error) => site::answer(StatusCode::INTERNAL_SERVER_ERROR, &json!(error.to_string())),
    }
}

#[cfg(feature = "persona")]
impl world::exports::sapling::kernel::guest::Guest for Kernel {
    async fn delegate(audience: String, access: Access) -> Result<Vec<u8>, Error> {
        let space = access
            .subject
            .ok_or_else(|| Error::Denied("a persona delegates a space, not everything".into()))?;
        let state =
            state().ok_or_else(|| Error::Unavailable("the persona has not booted".into()))?;
        let tonk = state.read().await;
        let chain = tonk
            .delegate_space(&space, &audience)
            .await
            .map_err(|error| Error::Denied(error.to_string()))?;
        Ok(chain)
    }
}

world::export!(Kernel with_types_in world);
