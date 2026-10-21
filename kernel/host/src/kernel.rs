//! Load a guest kernel, boot it over one subject, and hand it requests, as a
//! service worker would hand it each fetch of its space.

use wcmp::{Accessor, Component, Engine, Func, Linker, ResourceDynamic, Store, Val};

use crate::bindings::Host_;
use crate::bindings::exports::sapling::kernel::lifecycle::{Access, Boot, Stacked};
use crate::http_types::{self, Body, Fields, HttpHost, Request, Types};
use crate::sapling::{KernelHost, Sapling};
use crate::{platform, wasi, wasi_bindings};

const HANDLER: &str = "wasi:http/handler@0.3.0";

/// A guest kernel running in a store of its own.
pub struct Kernel {
    store: Store<KernelHost>,
    bindings: Host_,
    handle: Func,
    types: Types,
    /// How long each stage took, in milliseconds.
    pub timings: Vec<(&'static str, f64)>,
}

/// An answer of the guest kernel's `wasi:http/handler`.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

impl Kernel {
    /// Compile and instantiate the guest kernel in `bytes` on `engine`, for
    /// a session that signs with the key of `seed`.
    pub async fn load(engine: &Engine, bytes: &[u8], seed: [u8; 32]) -> Result<Kernel, String> {
        Kernel::load_restoring(engine, bytes, seed, Vec::new()).await
    }

    /// [`Kernel::load`], with the store holding `changes` an earlier store
    /// wrote, as a worker that starts again over a space it held does.
    pub async fn load_restoring(
        engine: &Engine,
        bytes: &[u8],
        seed: [u8; 32],
        changes: Vec<crate::sapling::Change>,
    ) -> Result<Kernel, String> {
        let mut timings = Vec::new();
        let started = platform::now_millis();
        let component = Component::new(engine, bytes)
            .await
            .map_err(|error| error.to_string())?;
        timings.push(("compile", platform::now_millis() - started));

        let started = platform::now_millis();
        let mut linker = Linker::new(engine);
        linker.allow_shadowing(true);
        wasi::define(&mut linker, "kernel").map_err(|error| error.to_string())?;
        wasi_bindings::Rest::add_to_linker::<_, Sapling>(&mut linker, |host| host)
            .map_err(|error| error.to_string())?;
        let types = http_types::define(&mut linker, &component)
            .map_err(|error| error.to_string())?
            .ok_or("the guest kernel does not import wasi:http/types")?;
        Host_::add_to_linker::<_, Sapling>(&mut linker, |host| host)
            .map_err(|error| error.to_string())?;

        let mut host = KernelHost::new(seed);
        host.restore(changes);
        host.http.set_types(Some(types));
        let mut store = Store::new(engine, host).map_err(|error| error.to_string())?;
        let instance = linker
            .instantiate(&mut store, &component)
            .await
            .map_err(|error| error.to_string())?;
        let bindings = Host_::new(&mut store, &instance).map_err(|error| error.to_string())?;
        let handle = instance
            .exports()
            .instance(HANDLER)
            .and_then(|handler| handler.func("handle"))
            .ok_or("the guest kernel exports no wasi:http/handler")?;
        timings.push(("instantiate", platform::now_millis() - started));
        Ok(Kernel {
            store,
            bindings,
            handle,
            types,
            timings,
        })
    }

    /// Boot the guest kernel over one subject, as the layer of
    /// `memory:space`, for `persona`.
    pub async fn init(&mut self, persona: &str, subject: &str) -> Result<(), String> {
        let started = platform::now_millis();
        let layer = self.store.data_mut().layer(subject);
        let boot = Boot {
            persona: Some(persona.to_owned()),
            stack: vec![Stacked {
                layer,
                scopes: vec!["memory:space".to_owned()],
                access: Access::ReadWrite,
            }],
        };
        let bindings = &self.bindings;
        let answer = self
            .store
            .run_concurrent(async |accessor: &Accessor<KernelHost>| {
                bindings
                    .sapling_kernel_lifecycle()
                    .call_init(accessor, boot)
                    .await
            })
            .await
            .map_err(|error| platform::describe(&error))?
            .map_err(|error| platform::describe(&error))?;
        self.timings
            .push(("init", platform::now_millis() - started));
        answer.map_err(|error| format!("init answered {error:?}"))
    }

    /// Each write since the last drain, for the host to keep durably.
    pub fn drain(&mut self) -> Vec<crate::sapling::Change> {
        core::mem::take(&mut self.store.data_mut().journal)
    }

    /// Hand the guest kernel one request, and read its whole answer.
    pub async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<Answer, String> {
        let rep = self.store.data_mut().http().insert_request(Request {
            method: method.to_owned(),
            scheme: Some("https".to_owned()),
            authority: Some("space.localhost".to_owned()),
            path_with_query: Some(path.to_owned()),
            headers: Fields {
                entries: Vec::new(),
                immutable: true,
            },
            body: Body::Bytes(body.to_vec()),
        });
        let request = ResourceDynamic::new_own(rep, self.types.request())
            .try_into_resource_any(&mut self.store.as_context_mut())
            .map_err(|error| error.to_string())?;
        let handle = &self.handle;
        self.store
            .run_concurrent(async |accessor: &Accessor<KernelHost>| {
                let returned = handle
                    .call_concurrent(accessor, &[Val::Resource(request)])
                    .await
                    .map_err(|error| platform::describe(&error))?;
                let response = match returned.first() {
                    Some(Val::Result(Ok(Some(response)))) => match response.as_ref() {
                        Val::Resource(response) if response.owned() => *response,
                        other => return Err(format!("`handle` answered {other:?}")),
                    },
                    other => return Err(format!("`handle` answered {other:?}")),
                };
                let (status, body) = accessor.with(|mut access| {
                    let store = access.store();
                    let rep = response
                        .try_into_resource_dynamic(store)
                        .map_err(|error| error.to_string())?
                        .rep();
                    let response = store
                        .data_mut()
                        .http()
                        .take_response(rep)
                        .ok_or("`handle` answered a response the host does not hold")?;
                    let body = http_types::read_body(store, response.body)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>((response.status, body))
                })?;
                Ok(Answer {
                    status,
                    body: body.await,
                })
            })
            .await
            .map_err(|error| platform::describe(&error))?
    }
}
