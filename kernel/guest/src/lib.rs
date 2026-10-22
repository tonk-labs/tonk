//! The Tonk kernel: Tonk's worker, as a component of the world
//! `sapling:kernel`. Its storage and keys are the layers and credentials its
//! host gives it (`tonk-kernel-world`), and it answers each request of its
//! origin with the worker's own API router, over `wasi:http/handler`.

pub mod world {
    wit_bindgen::generate!({
        path: "../../rust/tonk-kernel-world/wit",
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
use tonk_worker::TonkState;
use tower::ServiceExt as _;
use world::exports::sapling::kernel::{channel, lifecycle};
use world::exports::wasi::http::handler;
use world::wasi::http::types::{ErrorCode, Request, Response};

/// The profile the worker keeps in the host's storage.
const PROFILE: &str = "kernel";

/// The worker's router, once `init` opened its state. A `static`, not a
/// thread-local: on `wasm32-wasip3` each export call is a task of its own.
static ROUTER: Mutex<Option<Held>> = Mutex::new(None);

/// The router holds Dialog DB's state, which is not `Send`.
struct Held(Router);

// SAFETY: a component instance runs on a single thread; nothing here is
// reached from another.
unsafe impl Send for Held {}

fn router() -> Option<Router> {
    ROUTER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| held.0.clone())
}

fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

struct Kernel;

impl lifecycle::Guest for Kernel {
    async fn init(_boot: lifecycle::Boot) -> Result<(), Error> {
        let started = world::wasi::clocks::monotonic_clock::now();
        let state = TonkState::open_kernel(PROFILE).await.map_err(invalid)?;
        let (router, _, _) = tonk_worker::api_router_with_state(state);
        *ROUTER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held(router));
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
        let Some(router) = router() else {
            return Ok(http::respond(503, Some("text/plain"), b"booting".to_vec()));
        };
        let request = incoming(request).await?;
        let answer = router
            .oneshot(request)
            .await
            .map_err(|error| ErrorCode::InternalError(Some(error.to_string())))?;
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

world::export!(Kernel with_types_in world);
