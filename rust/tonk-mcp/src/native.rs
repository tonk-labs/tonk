//! A space's kernel booted on Wasmtime, as the Host Worker boots it in an
//! isolate: what this crate's tests drive writers over.
//!
//! A kernel is not `Send`, and serves on a task of its own (see
//! `Kernel::serve`), so each one runs on a thread of its own, and a writer
//! holds only what sends it requests.

use std::future::Future;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use futures_util::StreamExt as _;
use http_body_util::BodyExt as _;
use tonk_kernel_host::credentials::Standing;
use tonk_kernel_host::kernel::{Kernel, Served};
use tonk_kernel_host::layers::Layers;
use wcmp::Engine;

use crate::{Kernels, SpaceKernel};

/// Boots the Tonk kernel on one Wasmtime engine.
#[derive(Clone)]
pub struct NativeKernels {
    engine: Engine,
}

impl Default for NativeKernels {
    fn default() -> Self {
        Self {
            engine: Engine::with_backend(
                wcmp_wasm_core_wasmtime::Wasmtime::new().expect("a Wasmtime backend"),
            )
            .expect("an engine"),
        }
    }
}

impl NativeKernels {
    /// [`Kernels::boot`], with the kernel's layers under `layers`, which a
    /// kernel booted before may have left.
    pub async fn boot_in(
        &self,
        layers: String,
        space: &Did,
        seed: &[u8; 32],
        chain: &DelegationChain,
    ) -> Result<NativeKernel, String> {
        let chain = chain.to_bytes().map_err(|error| error.to_string())?;
        let (engine, space, seed) = (self.engine.clone(), space.to_string(), *seed);
        let (served, booted) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            tokio::task::LocalSet::new().block_on(&runtime, async move {
                match serve(&engine, layers, &space, seed, chain).await {
                    Ok((kernel, driver)) => {
                        let _ = served.send(Ok(kernel));
                        if let Err(error) = driver.await {
                            eprintln!("the kernel stopped: {error}");
                        }
                    }
                    Err(error) => {
                        let _ = served.send(Err(error));
                    }
                }
            });
        });
        booted
            .await
            .map_err(|_| "the kernel's thread ended".to_owned())?
            .map(NativeKernel)
    }
}

impl Kernels for NativeKernels {
    type Kernel = NativeKernel;

    async fn boot(
        &self,
        space: &Did,
        seed: &[u8; 32],
        chain: &DelegationChain,
    ) -> Result<NativeKernel, String> {
        let layers = std::env::temp_dir().join(format!("tonk-mcp-{}", rand::random::<u64>()));
        self.boot_in(layers.to_string_lossy().into_owned(), space, seed, chain)
            .await
    }
}

/// The kernel of `space`, booted over the layers under `layers` and acting
/// as the key `seed` is, served, with what drives it.
async fn serve(
    engine: &Engine,
    layers: String,
    space: &str,
    seed: [u8; 32],
    chain: Vec<u8>,
) -> Result<(Served, impl Future<Output = Result<(), String>>), String> {
    let mut kernel = Kernel::load(
        engine,
        tonk_components::TONK_KERNEL_BYTES,
        Layers::under(layers),
    )
    .await?;
    kernel.serve_at("https", "space-writer.invalid");
    kernel
        .init(space, Standing::handed(seed, Some(chain)))
        .await?;
    Ok(kernel.serve())
}

/// A space's kernel on Wasmtime.
pub struct NativeKernel(Served);

impl SpaceKernel for NativeKernel {
    async fn call(&self, request: Request<Body>) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let body = body
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        let path = parts
            .uri
            .path_and_query()
            .map_or("/", |path| path.as_str())
            .to_owned();
        let headers = parts
            .headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
            .collect();
        let head = match self
            .0
            .request(parts.method.as_str(), &path, headers, &body)
            .await
        {
            Ok(head) => head,
            Err(error) => {
                let mut response = Response::new(Body::from(error));
                *response.status_mut() = StatusCode::BAD_GATEWAY;
                return response;
            }
        };
        let pieces: Vec<Vec<u8>> = head.body.collect().await;
        let mut response = Response::builder().status(head.status);
        for (name, value) in head.headers {
            response = response.header(name, value);
        }
        response
            .body(Body::from(pieces.concat()))
            .unwrap_or_else(|_| Response::new(Body::empty()))
    }
}
