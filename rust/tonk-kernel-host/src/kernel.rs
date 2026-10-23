//! Load a guest kernel, boot it over one subject, and hand it requests, as a
//! service worker would hand it each fetch of its space.

use futures::channel::{mpsc, oneshot};
use futures::stream::FuturesUnordered;
use futures::{FutureExt as _, StreamExt as _};
use wcmp::{Accessor, Component, Engine, Func, Linker, ResourceDynamic, Store, Val};

use crate::bindings::Host_;
use crate::bindings::exports::sapling::kernel::lifecycle::{Access, Boot, Stacked};
use crate::credentials::Standing;
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

/// The start of an answer of the guest kernel: its status and headers, and
/// its body, piece by piece as the guest writes it.
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: mpsc::UnboundedReceiver<Vec<u8>>,
}

/// What a served kernel is asked to do.
enum Job {
    Request {
        method: String,
        path: String,
        headers: Vec<(String, Vec<u8>)>,
        body: Vec<u8>,
        reply: oneshot::Sender<Result<Head, String>>,
    },
    Drain(oneshot::Sender<Vec<crate::sapling::Change>>),
}

/// A guest kernel that a driver runs (see [`Kernel::serve`]): requests reach
/// it at once, each answered as the guest writes its answer, however many
/// are open.
#[derive(Clone)]
pub struct Served {
    jobs: mpsc::UnboundedSender<Job>,
}

impl Served {
    /// Hand the guest kernel one request, and answer the start of its answer.
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        headers: Vec<(String, Vec<u8>)>,
        body: &[u8],
    ) -> Result<Head, String> {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Request {
                method: method.to_owned(),
                path: path.to_owned(),
                headers,
                body: body.to_vec(),
                reply,
            })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }

    /// Each write since the last drain, for the host to keep durably.
    pub async fn drain(&self) -> Result<Vec<crate::sapling::Change>, String> {
        let (reply, drained) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Drain(reply))
            .map_err(|_| "the kernel has stopped".to_owned())?;
        drained
            .await
            .map_err(|_| "the kernel has stopped".to_owned())
    }
}

/// Start `request` in the guest kernel and answer the start of its answer,
/// leaving its body to be written as long as the store is driven.
async fn answer(
    accessor: &Accessor<KernelHost>,
    handle: &Func,
    types: Types,
    method: String,
    path: String,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
) -> Result<Head, String> {
    let request = accessor.with(|mut access| {
        let rep = access.data_mut().http().insert_request(Request {
            method,
            scheme: Some("https".to_owned()),
            authority: Some("space.localhost".to_owned()),
            path_with_query: Some(path),
            headers: Fields {
                entries: headers,
                immutable: true,
            },
            body: Body::Bytes(body),
        });
        ResourceDynamic::new_own(rep, types.request())
            .try_into_resource_any(access.store())
            .map_err(|error| error.to_string())
    })?;
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
    accessor.with(|mut access| {
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
        let body =
            http_types::stream_body(store, response.body).map_err(|error| error.to_string())?;
        Ok(Head {
            status: response.status,
            headers: response.headers.entries,
            body,
        })
    })
}

impl Kernel {
    /// Hand the guest kernel to a driver: answer what reaches it, and the
    /// future that drives it, which runs until every [`Served`] is dropped.
    /// The store is driven for as long as that future is polled, so a body
    /// the guest is still writing, such as a subscription's, goes on flowing
    /// while other requests are answered.
    pub fn serve(self) -> (Served, impl Future<Output = Result<(), String>>) {
        let (jobs, mut queued) = mpsc::unbounded::<Job>();
        let Kernel {
            mut store,
            handle,
            types,
            ..
        } = self;
        let driver = async move {
            store
                .run_concurrent(async |accessor: &Accessor<KernelHost>| {
                    let mut running = FuturesUnordered::new();
                    loop {
                        futures::select! {
                            job = queued.next() => match job {
                                None => break,
                                Some(Job::Drain(reply)) => {
                                    let changes = accessor.with(|mut access| {
                                        core::mem::take(&mut access.data_mut().journal)
                                    });
                                    let _ = reply.send(changes);
                                }
                                Some(Job::Request { method, path, headers, body, reply }) => {
                                    let handle = &handle;
                                    running.push(
                                        async move {
                                            let head = answer(
                                                accessor, handle, types, method, path, headers, body,
                                            )
                                            .await;
                                            let _ = reply.send(head);
                                        }
                                        .boxed_local(),
                                    );
                                }
                            },
                            () = running.select_next_some() => {}
                        }
                    }
                })
                .await
                .map_err(|error| platform::describe(&error))
        };
        (Served { jobs }, driver)
    }

    /// Compile and instantiate the guest kernel in `bytes` on `engine`.
    pub async fn load(engine: &Engine, bytes: &[u8]) -> Result<Kernel, String> {
        Kernel::load_restoring(engine, bytes, Vec::new()).await
    }

    /// [`Kernel::load`], with the store holding `changes` an earlier store
    /// wrote, as a worker that starts again over a space it held does.
    pub async fn load_restoring(
        engine: &Engine,
        bytes: &[u8],
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

        let mut host = KernelHost::default();
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
    /// `memory:space`, with `standing` as its session.
    pub async fn init(&mut self, subject: &str, standing: Standing) -> Result<(), String> {
        let started = platform::now_millis();
        let layer = self.store.data_mut().layer(subject);
        let session = self.store.data_mut().session(standing);
        let boot = Boot {
            session,
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
