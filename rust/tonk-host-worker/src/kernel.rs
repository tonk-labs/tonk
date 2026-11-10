//! The space's kernel, as the Worker was deployed with it.
//!
//! A Worker compiles no WebAssembly at run time, so it is deployed with the
//! kernel bundled ahead of time (see `README.md`): the component, whose core
//! modules are skeletons, and every module the polyfill makes, compiled. The
//! deployment's entry module puts them on the global object as
//! `TONK_KERNEL`, `{ component, modules }`, before the Rust module loads;
//! the first kernel booted hands the polyfill every module.

use std::cell::OnceCell;
use std::rc::Rc;

use axum::body::Body;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;
use http::{Request, Response};
use http_body_util::BodyExt as _;
use js_sys::{Array, Reflect, Uint8Array, WebAssembly};
use tonk_kernel_host::hosted::{HostedKernel, provide_module};
use tonk_mcp::{Kernels, SpaceKernel};
use wasm_bindgen::{JsCast as _, JsValue};

/// The global the entry module puts the kernel on.
const DEPLOYED: &str = "TONK_KERNEL";

thread_local! {
    /// The component, once the polyfill holds its modules.
    static COMPONENT: OnceCell<Rc<Vec<u8>>> = const { OnceCell::new() };
}

/// The component the Worker was deployed with, its modules handed to the
/// polyfill the first time.
fn component() -> Result<Rc<Vec<u8>>, String> {
    COMPONENT.with(|component| {
        if let Some(bytes) = component.get() {
            return Ok(bytes.clone());
        }
        let deployed = Reflect::get(&js_sys::global(), &DEPLOYED.into())
            .ok()
            .filter(|deployed| !deployed.is_undefined())
            .ok_or("the Worker was deployed with no kernel")?;
        let field =
            |name: &str| Reflect::get(&deployed, &name.into()).unwrap_or(JsValue::UNDEFINED);
        let modules: Array = field("modules")
            .dyn_into()
            .map_err(|_| "the kernel's modules are not an array")?;
        for entry in modules.iter() {
            let entry = Array::from(&entry);
            let key = entry.get(0).as_string().ok_or("a module has no key")?;
            let module: WebAssembly::Module = entry
                .get(1)
                .dyn_into()
                .map_err(|_| "a module is not compiled")?;
            provide_module(key, module);
        }
        let bytes = Rc::new(Uint8Array::new(&field("component")).to_vec());
        let _ = component.set(bytes.clone());
        Ok(bytes)
    })
}

/// Boots a space's kernel in this isolate.
#[derive(Clone, Copy, Default)]
pub struct WorkerKernels;

impl Kernels for WorkerKernels {
    type Kernel = WorkerKernel;

    async fn boot(
        &self,
        space: &Did,
        seed: &[u8; 32],
        chain: &DelegationChain,
    ) -> Result<WorkerKernel, String> {
        let chain = chain.to_bytes().map_err(|error| error.to_string())?;
        let kernel = HostedKernel::start(
            &component()?,
            space.as_ref(),
            ("https", "space-writer.invalid"),
            *seed,
            Some(chain),
        )
        .await?;
        Ok(WorkerKernel(kernel))
    }
}

/// A space's kernel in this isolate.
pub struct WorkerKernel(HostedKernel);

impl SpaceKernel for WorkerKernel {
    async fn call(&self, request: Request<Body>) -> Response<Body> {
        let (parts, body) = request.into_parts();
        let body = match body.collect().await {
            Ok(body) => body.to_bytes(),
            Err(error) => return failure(&error.to_string()),
        };
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
        match self
            .0
            .answer(parts.method.as_str(), &path, headers, &body)
            .await
        {
            Ok(answered) => {
                let mut response = Response::builder().status(answered.status);
                for (name, value) in answered.headers {
                    response = response.header(name, value);
                }
                response
                    .body(Body::from(answered.body))
                    .unwrap_or_else(|error| failure(&error.to_string()))
            }
            Err(error) => failure(&error),
        }
    }
}

fn failure(message: &str) -> Response<Body> {
    let mut response = Response::new(Body::from(
        serde_json::json!({ "error": message, "code": "KERNEL_UNREACHABLE" }).to_string(),
    ));
    *response.status_mut() = http::StatusCode::BAD_GATEWAY;
    response
}
