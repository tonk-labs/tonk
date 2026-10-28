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

use axum::Router;
use axum::http::StatusCode;
use http_body_util::BodyExt as _;
use serde_json::{Value, json};
use site::{Routed, Site};
use tonk_kernel_world::boot;
use tonk_kernel_world::guest::HandleLayer;
use tonk_kernel_world::layer::{SharedLayer, share};
use tonk_kernel_world::signal::Signal;
use tonk_kernel_world::world::sapling::kernel::credential::{Access, PersonaAuthority};
use tonk_kernel_world::world::sapling::kernel::types::Error;
use tonk_worker::{AppState, TonkState};
use tower::ServiceExt as _;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::wasi::http::types::{ErrorCode, Request, Response};

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
            wit_bindgen::spawn_local(async move { renew(&state, &space).await });
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

    async fn merge(
        _provisional: &PersonaAuthority,
        _permanent: &PersonaAuthority,
    ) -> Result<(), Error> {
        Err(Error::Unavailable(
            "this kernel merges no persona yet".into(),
        ))
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
        tonk_kernel_world::application::received(&message)
    }
}

/// `request`, as the worker's router takes one.
async fn incoming(request: Request) -> Result<axum::http::Request<axum::body::Body>, ErrorCode> {
    let method = request.get_method();
    let target = request
        .get_path_with_query()
        .unwrap_or_else(|| "/".to_owned());
    let headers = request.get_headers().copy_all();
    let body = http::read_body(request).await?;
    let mut built = axum::http::Request::builder()
        .method(http::method(&method))
        .uri(target);
    for (name, value) in headers {
        built = built.header(name, value);
    }
    built
        .body(axum::body::Body::from(body))
        .map_err(|error| ErrorCode::InternalError(Some(error.to_string())))
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
        let answer = match site::route(request, &site) {
            Routed::Answer(answer) => answer,
            Routed::Pass(request) => {
                let Ok(answer) = router.oneshot(request).await;
                answer
            }
            Routed::Store(request) => {
                let Ok(answer) = router.oneshot(request).await;
                site::stored(answer).await
            }
            Routed::Seed(space) => seed_of(&space).await,
            Routed::Settle(space) => settle(&space).await,
            Routed::Snapshot(space) => snapshot(&space).await,
        };
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

/// Ask for the delegation of `space` to this kernel's session, and take up
/// what the representative hands over.
async fn take_up(state: &TonkState, space: &Space) {
    let chains = delegated(space, "to serve the space").await;
    adopt(state, space, &chains).await;
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
async fn renew(state: &AppState, space: &Space) {
    let chains = delegated(space, "to keep serving the space").await;
    adopt(&*state.read().await, space, &chains).await;
}

/// The delegation of `space` to this kernel's session, as its representative
/// answers: none when it refuses, or fails to.
async fn delegated(space: &Space, reason: &str) -> Vec<Vec<u8>> {
    let access = Access {
        subject: space.subject.clone(),
        command: "/".to_owned(),
        arguments: Vec::new(),
        reason: reason.to_owned(),
    };
    match tonk_kernel_world::session::delegate(access).await {
        Ok(chain) => vec![chain],
        Err(error) => {
            eprintln!(
                "tonk-kernel: {} was not delegated: {error:?}",
                space.subject
            );
            Vec::new()
        }
    }
}

/// Take up `chains`. A chain is taken up only if the space signed it all the
/// way down to this kernel's session, so a host cannot make the kernel act
/// for another space.
async fn adopt(state: &TonkState, space: &Space, chains: &[Vec<u8>]) {
    for chain in chains {
        if let Err(error) = state.adopt_space(&space.subject, chain).await {
            eprintln!(
                "tonk-kernel: a delegation of {} was not taken up: {error}",
                space.subject
            );
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
        let space = access.subject;
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
