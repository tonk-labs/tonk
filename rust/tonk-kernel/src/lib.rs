//! The Tonk kernel: Tonk's worker, as a component of the world
//! `sapling:kernel`. Its storage and keys are the layers and credentials its
//! host gives it (`tonk-kernel-world`), and it answers each request of its
//! origin with the worker's own API router, over `wasi:http/handler`.
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
            "sapling:kernel/session@0.1.0": tonk_kernel_world::world::sapling::kernel::session,
            "sapling:kernel/credentials@0.1.0": tonk_kernel_world::world::sapling::kernel::credentials,
        },
        generate_all,
    });
}

mod http;

use std::sync::{Mutex, PoisonError};

use axum::Router;
use http_body_util::BodyExt as _;
use tonk_kernel_world::world::sapling::kernel::types::Error;
use tonk_worker::{AppState, TonkState};
use tower::ServiceExt as _;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::wasi::http::types::{ErrorCode, Request, Response};

/// The worker's router, once `init` opened its state. A `static`, not a
/// thread-local: on `wasm32-wasip3` each export call is a task of its own.
static ROUTER: Mutex<Option<Held>> = Mutex::new(None);

/// The router holds Dialog DB's state, which is not `Send`: the router, the
/// state behind it, and the space the boot named.
struct Held {
    router: Router,
    state: AppState,
    space: Option<String>,
}

/// Where the bootloader hands the kernel the delegation for its space: the
/// chain from the space to the profile `/api/identify` names. A chain is
/// taken up only if the space signed it all the way down, so whoever posts
/// it here cannot make the kernel act for another space.
const ADOPT_PATH: &str = "/.kernel/adopt";

// SAFETY: a component instance runs on a single thread; nothing here is
// reached from another.
unsafe impl Send for Held {}

fn held() -> Option<(Router, AppState, Option<String>)> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| (held.router.clone(), held.state.clone(), held.space.clone()))
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

struct Kernel;

impl lifecycle::Guest for Kernel {
    async fn init(boot: lifecycle::Boot) -> Result<(), Error> {
        let started = world::wasi::clocks::monotonic_clock::now();
        // The space this kernel serves: the subject of its `memory:space`.
        let space = boot
            .stack
            .iter()
            .find(|stacked| stacked.scopes.iter().any(|scope| scope == "memory:space"))
            .map(|stacked| stacked.layer.subject());
        let state = TonkState::open_kernel().await.map_err(invalid)?;
        // A space taken up before: put back what a restart lost about it.
        if let Some(space) = &space
            && let Err(error) = state.resume_space(space).await
        {
            eprintln!("tonk-kernel: {space} was not resumed: {error}");
        }
        let (router, state, _) = tonk_worker::api_router_with_state(state);
        *ROUTER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held {
            router,
            state,
            space,
        });
        let took = world::wasi::clocks::monotonic_clock::now() - started;
        eprintln!("tonk-kernel: opened the worker in {}us", took / 1000);
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
        let Some((router, state, space)) = held() else {
            return Ok(http::respond(503, Some("text/plain"), b"booting".to_vec()));
        };
        let request = incoming(request).await?;
        if request.uri().path() == ADOPT_PATH {
            return adopt(request, &state, space.as_deref()).await;
        }
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

/// Take up the delegation in the body of `request` for `space`.
async fn adopt(
    request: axum::http::Request<axum::body::Body>,
    state: &AppState,
    space: Option<&str>,
) -> Result<Response, ErrorCode> {
    if request.method() != axum::http::Method::POST {
        return Ok(http::respond(
            405,
            Some("text/plain"),
            b"POST a delegation".to_vec(),
        ));
    }
    let Some(space) = space else {
        return Ok(http::respond(
            409,
            Some("text/plain"),
            b"this kernel serves no space".to_vec(),
        ));
    };
    let chain = request
        .into_body()
        .collect()
        .await
        .map_err(|error| ErrorCode::InternalError(Some(error.to_string())))?
        .to_bytes();
    match state.read().await.adopt_space(space, &chain).await {
        Ok(()) => Ok(http::respond(204, None, Vec::new())),
        Err(error) => Ok(http::respond(
            400,
            Some("text/plain"),
            error.to_string().into_bytes(),
        )),
    }
}

world::export!(Kernel with_types_in world);
