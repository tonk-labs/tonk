//! The embedder of a host that keeps nothing between runs and compiles no
//! WebAssembly: a Durable Object of a Cloudflare Worker, which serves the
//! hosted agent. It boots a space's kernel over layers kept in memory, acting
//! as a key it derives from what the caller hands it and holding the
//! delegation of the space to that key, and hands the kernel requests as its
//! space's origin would.
//!
//! A Worker refuses to compile WebAssembly at run time, so the polyfill
//! takes every module it makes from those the Worker was deployed with,
//! which [`provide_module`] hands it by the hash of their bytes. To learn
//! which modules those are, a host that can compile, such as Node, boots the
//! same kernel with [`record_modules`] on and writes out what
//! [`recorded_modules`] answers.

use std::cell::{Cell, RefCell};

use futures::StreamExt as _;
use js_sys::{Array, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::prelude::*;
use wcmp::Engine;
use wcmp_wasm_core_web::{Precompiled, Web};

use crate::credentials::Standing;
use crate::kernel::{Kernel, Served};
use crate::layers::Layers;
use crate::platform;

thread_local! {
    /// The engine every kernel of this isolate runs on: one backend, so the
    /// polyfill's generated modules are made once.
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
    /// How many kernels this isolate booted, which names each one's layers.
    static BOOTED: Cell<u64> = const { Cell::new(0) };
}

/// The engine of this isolate.
fn engine() -> Result<Engine, JsError> {
    ENGINE.with(|engine| {
        if let Some(engine) = engine.borrow().as_ref() {
            return Ok(engine.clone());
        }
        let made =
            Engine::with_backend(Web::new()).map_err(|error| JsError::new(&error.to_string()))?;
        *engine.borrow_mut() = Some(made.clone());
        Ok(made)
    })
}

/// Serve `module`, compiled ahead of time, for every module whose bytes have
/// the key `key`: their SHA-256, in lowercase hexadecimal.
#[wasm_bindgen(js_name = provideModule)]
pub fn provide_module(key: String, module: WebAssembly::Module) {
    Precompiled::provide(key, module);
}

/// Keep the bytes of every module the polyfill makes from now on.
#[wasm_bindgen(js_name = recordModules)]
pub fn record_modules() {
    Precompiled::record();
}

/// The modules made since [`record_modules`], each once: an object from each
/// key to the module's bytes.
#[wasm_bindgen(js_name = recordedModules)]
pub fn recorded_modules() -> Object {
    Precompiled::recorded()
}

/// A space's kernel, booted over layers in memory.
#[wasm_bindgen]
pub struct HostedKernel {
    kernel: Served,
    /// How long each stage of the boot took, in milliseconds.
    timings: Vec<(String, f64)>,
}

#[wasm_bindgen]
impl HostedKernel {
    /// Boot the kernel `component` over the space `subject`, serving the
    /// origin of `scheme` and `authority`, acting as the key whose seed is
    /// `seed`, holding `chain`, the delegation of the space to that key, if
    /// there is one.
    pub async fn boot(
        component: Uint8Array,
        subject: String,
        scheme: String,
        authority: String,
        seed: Uint8Array,
        chain: Option<Uint8Array>,
    ) -> Result<HostedKernel, JsError> {
        let seed: [u8; 32] = seed
            .to_vec()
            .try_into()
            .map_err(|_| JsError::new("a seed is 32 bytes"))?;
        let booted = BOOTED.with(|booted| {
            booted.set(booted.get() + 1);
            booted.get()
        });
        let started = platform::now_millis();
        let engine = engine()?;
        let mut kernel = Kernel::load(
            &engine,
            &component.to_vec(),
            Layers::under(format!("/hosted/{booted}")),
        )
        .await
        .map_err(|error| JsError::new(&error))?;
        let mut timings = vec![("load".to_owned(), platform::now_millis() - started)];
        kernel.serve_at(&scheme, &authority);
        kernel
            .init(
                &subject,
                Standing::handed(seed, chain.map(|chain| chain.to_vec())),
            )
            .await
            .map_err(|error| JsError::new(&error))?;
        timings.extend(
            kernel
                .timings
                .iter()
                .map(|(stage, millis)| ((*stage).to_owned(), *millis)),
        );
        let (kernel, driver) = kernel.serve();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = driver.await {
                platform::log("kernel", &format!("the kernel stopped: {error}"), true);
            }
        });
        Ok(HostedKernel { kernel, timings })
    }

    /// How long each stage of the boot took: `[stage, milliseconds]` pairs.
    pub fn timings(&self) -> Array {
        self.timings
            .iter()
            .map(|(stage, millis)| {
                Array::of2(&JsValue::from_str(stage), &JsValue::from_f64(*millis))
            })
            .collect()
    }

    /// Hand the kernel the request `method` `path`, with `headers`, an array
    /// of `[name, value]` pairs, and `body`, and answer `{ status, headers,
    /// body }` once the kernel has written all of its answer.
    pub async fn request(
        &self,
        method: String,
        path: String,
        headers: Array,
        body: Uint8Array,
    ) -> Result<Object, JsError> {
        let headers = headers
            .iter()
            .filter_map(|pair| {
                let pair = Array::from(&pair);
                Some((
                    pair.get(0).as_string()?,
                    pair.get(1).as_string()?.into_bytes(),
                ))
            })
            .collect();
        let head = self
            .kernel
            .request(&method, &path, headers, &body.to_vec())
            .await
            .map_err(|error| JsError::new(&error))?;
        let mut answer = Vec::new();
        let mut pieces = head.body;
        while let Some(piece) = pieces.next().await {
            answer.extend_from_slice(&piece);
        }
        let headers: Array = head
            .headers
            .iter()
            .map(|(name, value)| {
                Array::of2(
                    &JsValue::from_str(name),
                    &JsValue::from_str(&String::from_utf8_lossy(value)),
                )
            })
            .collect();
        let response = Object::new();
        for (key, value) in [
            ("status", JsValue::from(head.status)),
            ("headers", headers.into()),
            ("body", Uint8Array::from(answer.as_slice()).into()),
        ] {
            Reflect::set(&response, &key.into(), &value)
                .map_err(|_| JsError::new("an answer could not be written"))?;
        }
        Ok(response)
    }
}
