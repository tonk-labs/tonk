//! Load a guest kernel, boot it over one subject, and hand it requests, as a
//! service worker would hand it each fetch of its space.

use futures::channel::{mpsc, oneshot};
use futures::stream::FuturesUnordered;
use futures::{FutureExt as _, StreamExt as _};
use wcmp::{Accessor, Component, Engine, Func, Linker, Resource, ResourceDynamic, Store, Val};

use crate::bindings::Host_;
use crate::bindings::exports::sapling::kernel::lifecycle::{Access as StackAccess, Boot, Stacked};
use crate::bindings::sapling::kernel::credential::Access;
use crate::bindings::sapling::kernel::types::Error;
use crate::credentials::{Key, Standing};
use crate::http_types::{self, Body, Fields, HttpHost, Request, Types};
use crate::layers::Layers;
use crate::persona_bindings::exports::sapling::kernel::guest;
use crate::sapling::{KernelHost, Sapling};
use crate::{platform, wasi, wasi_bindings};

const HANDLER: &str = "wasi:http/handler@0.3.0";

/// A guest kernel running in a store of its own.
pub struct Kernel {
    store: Store<KernelHost>,
    bindings: Host_,
    /// What a persona delegates to the person's other workers, if the guest
    /// kernel is one.
    guest: Option<guest::Guest>,
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

/// What a persona answers a delegation with, or why the guest kernel could
/// not be asked.
pub type Delegated = Result<Result<Vec<u8>, Error>, String>;

/// What a served kernel is asked to do.
enum Job {
    Request {
        method: String,
        path: String,
        headers: Vec<(String, Vec<u8>)>,
        body: Vec<u8>,
        reply: oneshot::Sender<Result<Head, String>>,
    },
    Delegate {
        audience: String,
        access: Access,
        reply: oneshot::Sender<Delegated>,
    },
    Receive {
        message: Vec<u8>,
        reply: oneshot::Sender<Result<Result<(), Error>, String>>,
    },
    Merge {
        provisional: Key,
        permanent: Key,
        reply: oneshot::Sender<Merged>,
    },
    Authorize {
        persona: Key,
        context: Vec<u8>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stand {
        standing: Option<(String, Vec<u8>)>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// What came of a merge: the permanent persona and its delegation to the
/// representative, which links it.
pub type Merged = Result<(String, Vec<u8>), String>;

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
}

impl Served {
    /// Hand the guest kernel a message from its embedder, through
    /// `channel.receive`.
    pub async fn receive(&self, message: Vec<u8>) -> Result<Result<(), Error>, String> {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Receive { message, reply })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }

    /// Merge the guest kernel's provisional persona, whose key is
    /// `provisional`, into the permanent one, whose key is `permanent`: both
    /// are lent to it for one call of `lifecycle.merge`.
    pub async fn merge(&self, provisional: Key, permanent: Key) -> Merged {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Merge {
                provisional,
                permanent,
                reply,
            })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }

    /// Lend the guest kernel the permanent persona's key, `persona`, for one
    /// call of `lifecycle.authorize` with `context`, the context it asked to
    /// be authorized in. Refused when `persona` is not the permanent persona
    /// the guest kernel acts for.
    pub async fn authorize(&self, persona: Key, context: Vec<u8>) -> Result<(), String> {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Authorize {
                persona,
                context,
                reply,
            })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }

    /// Have the guest kernel, a persona, act for the permanent persona
    /// `standing` names, with its delegation to the representative, or for
    /// its provisional persona with none.
    pub async fn stand(&self, standing: Option<(String, Vec<u8>)>) -> Result<(), String> {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Stand { standing, reply })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }

    /// Ask the guest kernel, a persona, to delegate `access` to `audience`.
    /// An error when the guest kernel is no persona.
    pub async fn delegate(&self, audience: &str, access: Access) -> Delegated {
        let (reply, answered) = oneshot::channel();
        self.jobs
            .unbounded_send(Job::Delegate {
                audience: audience.to_owned(),
                access,
                reply,
            })
            .map_err(|_| "the kernel has stopped".to_owned())?;
        answered
            .await
            .map_err(|_| "the kernel has stopped".to_owned())?
    }
}

/// Lend the guest kernel both personas' keys for one call of
/// `lifecycle.merge`, take them back, and have the persona's worker act for
/// the permanent persona from then on.
async fn merge(
    accessor: &Accessor<KernelHost>,
    bindings: &Host_,
    provisional: Key,
    permanent: Key,
) -> Merged {
    let (provisional, permanent) = accessor.with(|mut access| {
        let host = access.data_mut();
        host.linked = None;
        (host.lend(provisional), host.lend(permanent))
    });
    let merged = bindings
        .sapling_kernel_lifecycle()
        .call_merge(
            accessor,
            Resource::new_borrow(provisional),
            Resource::new_borrow(permanent),
        )
        .await;
    accessor.with(|mut access| {
        let host = access.data_mut();
        host.withdraw(provisional);
        host.withdraw(permanent);
    });
    merged
        .map_err(|error| platform::describe(&error))?
        .map_err(|error| format!("merge answered {error:?}"))?;
    accessor
        .with(|mut access| access.data_mut().merged())
        .ok_or_else(|| "the permanent persona linked no representative".to_owned())
}

/// Lend the guest kernel the key of the permanent persona it acts for for one
/// call of `lifecycle.authorize`, and take it back.
async fn authorize(
    accessor: &Accessor<KernelHost>,
    bindings: &Host_,
    persona: Key,
    context: Vec<u8>,
) -> Result<(), String> {
    let authority = accessor.with(|mut access| {
        let host = access.data_mut();
        if host.permanent().as_deref() != Some(persona.did().as_str()) {
            return Err("this passkey belongs to a different account".to_owned());
        }
        Ok(host.lend(persona))
    })?;
    let authorized = bindings
        .sapling_kernel_lifecycle()
        .call_authorize(accessor, Resource::new_borrow(authority), context)
        .await;
    accessor.with(|mut access| access.data_mut().withdraw(authority));
    authorized
        .map_err(|error| platform::describe(&error))?
        .map_err(|error| match error {
            Error::Denied(reason) | Error::Invalid(reason) | Error::Unavailable(reason) => reason,
            other => format!("{other:?}"),
        })
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
        let (scheme, authority) = access.data_mut().origin.clone();
        let rep = access.data_mut().http().insert_request(Request {
            method,
            scheme: Some(scheme),
            authority: Some(authority),
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
            guest,
            types,
            bindings,
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
                                Some(Job::Delegate { audience, access, reply }) => {
                                    let guest = &guest;
                                    running.push(
                                        async move {
                                            let delegated = match guest {
                                                Some(guest) => guest
                                                    .call_delegate(accessor, audience, access)
                                                    .await
                                                    .map_err(|error| platform::describe(&error)),
                                                None => Err("the kernel is no persona".to_owned()),
                                            };
                                            let _ = reply.send(delegated);
                                        }
                                        .boxed_local(),
                                    );
                                }
                                Some(Job::Receive { message, reply }) => {
                                    let bindings = &bindings;
                                    running.push(
                                        async move {
                                            let received = bindings
                                                .sapling_kernel_channel()
                                                .call_receive(accessor, message)
                                                .await
                                                .map_err(|error| platform::describe(&error));
                                            let _ = reply.send(received);
                                        }
                                        .boxed_local(),
                                    );
                                }
                                Some(Job::Merge { provisional, permanent, reply }) => {
                                    let bindings = &bindings;
                                    running.push(
                                        async move {
                                            let _ = reply.send(merge(accessor, bindings, provisional, permanent).await);
                                        }
                                        .boxed_local(),
                                    );
                                }
                                Some(Job::Authorize { persona, context, reply }) => {
                                    let bindings = &bindings;
                                    running.push(
                                        async move {
                                            let _ = reply.send(authorize(accessor, bindings, persona, context).await);
                                        }
                                        .boxed_local(),
                                    );
                                }
                                Some(Job::Stand { standing, reply }) => {
                                    let _ = reply.send(accessor.with(|mut access| access.data_mut().stand(standing)));
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

    /// Compile and instantiate the guest kernel in `bytes` on `engine`, over
    /// the layers kept in `layers`.
    pub async fn load(engine: &Engine, bytes: &[u8], layers: Layers) -> Result<Kernel, String> {
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

        let mut host = KernelHost::new(layers);
        host.http.set_types(Some(types));
        let mut store = Store::new(engine, host).map_err(|error| error.to_string())?;
        let instance = linker
            .instantiate(&mut store, &component)
            .await
            .map_err(|error| error.to_string())?;
        let bindings = Host_::new(&mut store, &instance).map_err(|error| error.to_string())?;
        let guest = guest::Guest::new(&instance).ok();
        let handle = instance
            .exports()
            .instance(HANDLER)
            .and_then(|handler| handler.func("handle"))
            .ok_or("the guest kernel exports no wasi:http/handler")?;
        timings.push(("instantiate", platform::now_millis() - started));
        Ok(Kernel {
            store,
            bindings,
            guest,
            handle,
            types,
            timings,
        })
    }

    /// Hand the guest kernel requests as made to the origin `authority`, by
    /// `scheme`: the origin of the site it serves.
    pub fn serve_at(&mut self, scheme: &str, authority: &str) {
        self.store.data_mut().origin = (scheme.to_owned(), authority.to_owned());
    }

    /// The addresses the guest kernel's provisional persona asks to merge
    /// into, from now on, as it asks.
    pub fn merges(&mut self) -> mpsc::UnboundedReceiver<String> {
        let (merges, asked) = mpsc::unbounded();
        self.store.data_mut().merges = Some(merges);
        asked
    }

    /// The authorizations the guest kernel's permanent persona asks for, from
    /// now on, as it asks: the reason to show, and the context.
    pub fn authorizations(&mut self) -> mpsc::UnboundedReceiver<(String, Vec<u8>)> {
        let (authorizations, asked) = mpsc::unbounded();
        self.store.data_mut().authorizations = Some(authorizations);
        asked
    }

    /// The messages the guest kernel sends with `application.send` from now
    /// on, as it sends them.
    pub fn messages(&mut self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (outbox, messages) = mpsc::unbounded();
        self.store.data_mut().outbox = Some(outbox);
        messages
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
                access: StackAccess::ReadWrite,
            }],
            landing: None,
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

    /// Ask the guest kernel, a persona, to delegate `access` to `audience`.
    pub async fn delegate(&mut self, audience: &str, access: Access) -> Delegated {
        let guest = self.guest.as_ref().ok_or("the kernel is no persona")?;
        let audience = audience.to_owned();
        self.store
            .run_concurrent(async |accessor: &Accessor<KernelHost>| {
                guest.call_delegate(accessor, audience, access).await
            })
            .await
            .map_err(|error| platform::describe(&error))?
            .map_err(|error| platform::describe(&error))
    }

    /// The DID of each key the host holds for the guest kernel, and whether
    /// the guest kernel knew its seed.
    pub fn keys(&mut self) -> Vec<(String, bool)> {
        self.store
            .data_mut()
            .keys
            .iter()
            .map(|key| (key.did(), key.imported()))
            .collect()
    }

    /// Hand the guest kernel one request, and read its whole answer.
    pub async fn request(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<Answer, String> {
        let (scheme, authority) = self.store.data_mut().origin.clone();
        let rep = self.store.data_mut().http().insert_request(Request {
            method: method.to_owned(),
            scheme: Some(scheme),
            authority: Some(authority),
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
