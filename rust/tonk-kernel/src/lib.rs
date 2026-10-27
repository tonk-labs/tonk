//! The Tonk kernel: Tonk's worker, as a component of the world
//! `sapling:kernel`. Its storage is the layers its host gives it, and its keys
//! are the session's and the ones it entrusts to it (`tonk-kernel-world`). It
//! answers each request of its origin with the worker's own API router, over
//! `wasi:http/handler`.
//!
//! Built for `wasm32-wasip3` by `tonk-components`; on every other target the
//! crate is empty.

#![cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]

pub mod world {
    wit_bindgen::generate!({
        path: "../tonk-kernel-world/wit",
        world: "sapling:kernel/kernel",
        with: {
            "sapling:kernel/types@0.1.0": tonk_kernel_world::world::sapling::kernel::types,
            "sapling:kernel/storage@0.1.0": tonk_kernel_world::world::sapling::kernel::storage,
            "sapling:kernel/credential@0.1.0": tonk_kernel_world::world::sapling::kernel::credential,
            "sapling:kernel/session@0.1.0": tonk_kernel_world::world::sapling::kernel::session,
        },
        generate_all,
    });
}

mod http;

use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use axum::Router;
use http_body_util::BodyExt as _;
use tonk_kernel_world::boot;
use tonk_kernel_world::world::sapling::kernel::session::{Claim, Subject};
use tonk_kernel_world::world::sapling::kernel::storage::Layer;
use tonk_kernel_world::world::sapling::kernel::types::Error;
use tonk_worker::{AppState, TonkState};
use tower::ServiceExt as _;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::wasi::http::types::{ErrorCode, Request, Response};

/// The worker's router, once `init` opened its state. A `static`, not a
/// thread-local: on `wasm32-wasip3` each export call is a task of its own.
static ROUTER: Mutex<Option<Held>> = Mutex::new(None);

/// The router holds Dialog DB's state, which is not `Send`.
struct Held {
    router: Router,
}

/// The space this kernel serves: its subject, and the layer the boot gave it,
/// which holds the space's boot cell and the kernel's wasm.
#[derive(Clone)]
struct Space {
    subject: String,
    layer: Rc<Layer>,
}

// SAFETY: a component instance runs on a single thread; nothing here is
// reached from another.
unsafe impl Send for Held {}

fn router() -> Option<Router> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| held.router.clone())
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
            .map(|stacked| Space {
                subject: stacked.layer.subject(),
                layer: Rc::new(stacked.layer),
            });
        // A kernel's peer is its session. A space that is the session's own
        // is a person's profile, which has an account of its own; any other
        // is a space this kernel is the own worker of, which it acts on as
        // its session is delegated.
        let own = tonk_kernel_world::session::credential().did();
        let (standing, space) = match space {
            Some(space) if space.subject == own => (tonk_worker::Standing::Person, None),
            space => (tonk_worker::Standing::Site, space),
        };
        tonk_worker::set_standing(standing);
        let state = TonkState::open_kernel().await.map_err(invalid)?;
        // The space, taken up with the delegation its session claims, or as
        // it was taken up before, with the delegation claimed anew alongside:
        // put back what a restart lost about it, and give it its first commit
        // if it went without.
        let renewing = match &space {
            Some(space) if state.acts().await.unwrap_or(false) => Some(space.clone()),
            Some(space) => {
                take_up(&state, space).await;
                None
            }
            None => None,
        };
        if let Some(space) = &space {
            if let Err(error) = state.resume_space(&space.subject).await {
                eprintln!("tonk-kernel: {} was not resumed: {error}", space.subject);
            }
            if let Err(error) = found(&state, space).await {
                eprintln!("tonk-kernel: {} was not founded: {error}", space.subject);
            }
        }
        let (router, state, _) = tonk_worker::api_router_with_state(state);
        *ROUTER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held { router });
        if let Some(space) = renewing {
            wit_bindgen::spawn_local(async move { renew(&state, &space).await });
        }
        let took = world::wasi::clocks::monotonic_clock::now() - started;
        tonk_common::log!("tonk-kernel: opened the worker in {}us", took / 1000);
        Ok(())
    }

    async fn sync(_changes: Vec<lifecycle::CellChange>) -> Result<(), Error> {
        Ok(())
    }

    async fn shutdown(_deadline_ms: u32) {}
}

impl channel::Guest for Kernel {
    async fn receive(_catchup: Vec<u8>) -> Result<(), Error> {
        Ok(())
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
        let Some(router) = router() else {
            return Ok(http::respond(503, Some("text/plain"), b"booting".to_vec()));
        };
        let request = incoming(request).await?;
        let Ok(answer) = router.oneshot(request).await;
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

/// Claim the delegation of `space` to this kernel's session, and take up
/// what the host hands over.
async fn take_up(state: &TonkState, space: &Space) {
    let chains = delegated(space, "to serve the space").await;
    adopt(state, space, &chains).await;
}

/// Claim the delegation of `space` anew, as the kernel serves what it took up
/// before, so a delegation running out is replaced. Nothing is locked while
/// the host asks the person's profile.
async fn renew(state: &AppState, space: &Space) {
    let chains = delegated(space, "to keep serving the space").await;
    adopt(&*state.read().await, space, &chains).await;
}

/// The delegations of `space` to this kernel's session, as its host answers
/// the claim: none when it answers with none, or fails to.
async fn delegated(space: &Space, reason: &str) -> Vec<Vec<u8>> {
    let claim = Claim {
        subject: Subject::This(space.subject.clone()),
        command: "/".to_owned(),
        arguments: Vec::new(),
        reason: reason.to_owned(),
    };
    tonk_kernel_world::session::claim(claim)
        .await
        .unwrap_or_else(|error| {
            eprintln!(
                "tonk-kernel: {} was not delegated: {error:?}",
                space.subject
            );
            Vec::new()
        })
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
    let cell = space
        .layer
        .resolve(boot::NAMESPACE.to_owned(), boot::CELL.to_owned())
        .await
        .map_err(|error| format!("the boot cell: {error:?}"))?;
    let Some(cell) = cell else {
        return Ok(());
    };
    let id = boot::read_cell(&cell.content)?;
    let hash = boot::digest(&id)?;
    let size = space
        .layer
        .size(hash.to_vec())
        .await
        .map_err(|error| format!("the kernel {id}: {error:?}"))?
        .ok_or_else(|| format!("the space does not hold the kernel {id} whole"))?;
    state
        .found_space(&space.subject, hash, size)
        .await
        .map_err(|error| error.to_string())
}

world::export!(Kernel with_types_in world);
