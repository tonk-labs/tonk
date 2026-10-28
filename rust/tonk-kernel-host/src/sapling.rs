//! The interfaces of `sapling:kernel` that are Sapling's own: each layer's
//! blocks, cells and blobs, kept where Dialog DB keeps the layer's location
//! (see [`crate::layers`]), and the catch-ups a guest kernel sends. The keys
//! are `credentials`'.

use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::HashMap;

use ed25519_dalek::SigningKey;
use futures::channel::mpsc::UnboundedSender;
use wcmp::{
    Accessor, HasData, Resource, Source, StoreContext, StreamConsumer, StreamReader, StreamResult,
};

use crate::bindings::sapling::kernel::{application, network, storage, types};
use crate::credentials::{Held, Key, Representation, Standing};
use crate::http_types::{HttpHost, HttpTable};
use crate::layers::{self, LayerSpace, Layers};
use storage::{BlobRead, ByteRange, Edition, Encoding, Layer};
use types::Error;

/// The host data of a store that runs a guest kernel.
#[derive(Default)]
pub struct KernelHost {
    pub http: HttpTable,
    /// Where the layers are kept.
    pub layers: Layers,
    /// The subject of each layer handle, by its representation.
    names: Vec<String>,
    /// Each layer opened, by its subject.
    opened: HashMap<String, LayerSpace>,
    /// The key each `credential` handle names, by its representation.
    pub(crate) keys: Vec<Key>,
    /// What each `representative` handle names, by its representation.
    pub(crate) representatives: Vec<Representation>,
    /// The persona each persona handle names, by its representation.
    pub(crate) personas: Vec<Held>,
    /// The standing each `session` handle names, by its representation.
    pub(crate) sessions: Vec<Standing>,
    /// The messages the guest kernel sent with `application.send`, when
    /// nothing takes them.
    pub sent: Vec<Vec<u8>>,
    /// What takes the messages the guest kernel sends, as it sends them.
    pub outbox: Option<UnboundedSender<Vec<u8>>>,
}

impl KernelHost {
    /// A host that keeps its layers in `layers`.
    pub fn new(layers: Layers) -> Self {
        Self {
            layers,
            ..Self::default()
        }
    }

    /// A handle on the layer of `subject`, for the boot or for `open`.
    pub fn layer(&mut self, subject: &str) -> Resource<Layer> {
        self.names.push(subject.to_owned());
        Resource::new_own((self.names.len() - 1) as u32)
    }

    fn subject_name(&self, layer: &Resource<Layer>) -> String {
        self.names[layer.rep() as usize].clone()
    }
}

/// The space of `layer`, opened once and kept.
async fn space<T: 'static>(
    accessor: &Accessor<T, Sapling>,
    layer: &Resource<Layer>,
) -> Result<LayerSpace, Error> {
    let (name, layers, held) = accessor.with(|mut access| {
        let host = access.get();
        let name = host.subject_name(layer);
        let held = host.opened.get(&name).cloned();
        (name, host.layers.clone(), held)
    });
    if let Some(space) = held {
        return Ok(space);
    }
    let space = layers.open(&name).await?;
    accessor.with(|mut access| access.get().opened.insert(name, space.clone()));
    Ok(space)
}

impl HttpHost for KernelHost {
    fn http(&mut self) -> &mut HttpTable {
        &mut self.http
    }
}

/// The projection the generated bindings reach the host through.
pub struct Sapling;

impl HasData for Sapling {
    type Data<'a> = &'a mut KernelHost;
}

impl types::Host for KernelHost {}

impl storage::Host for KernelHost {}

impl storage::HostLayer for KernelHost {
    fn subject(&mut self, layer: Resource<Layer>) -> String {
        self.subject_name(&layer)
    }

    fn drop(&mut self, _layer: Resource<Layer>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> storage::HostWithStore<T> for Sapling {
    async fn open(
        accessor: &Accessor<T, Self>,
        subject: String,
        chain: Vec<u8>,
    ) -> Result<Resource<Layer>, Error> {
        trace("open");
        let session = accessor
            .with(|mut access| {
                access
                    .get()
                    .sessions
                    .first()
                    .map(|standing| standing.credential.did())
            })
            .ok_or_else(|| Error::Denied("this worker has no session".into()))?;
        crate::chain::check(&chain, &subject, &session).await?;
        Ok(accessor.with(|mut access| access.get().layer(&subject)))
    }
}

impl<T: 'static> storage::HostLayerWithStore<T> for Sapling {
    async fn get(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        catalog: String,
        key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, Error> {
        trace("get");
        layers::get(&space(accessor, &layer).await?, &catalog, &key).await
    }

    async fn get_many(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        catalog: String,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Option<Vec<u8>>>, Error> {
        trace("get_many");
        let space = space(accessor, &layer).await?;
        let mut blocks = Vec::with_capacity(keys.len());
        for key in keys {
            blocks.push(layers::get(&space, &catalog, &key).await?);
        }
        Ok(blocks)
    }

    async fn put(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        catalog: String,
        key: Vec<u8>,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        trace("put");
        layers::put(&space(accessor, &layer).await?, &catalog, &key, bytes).await
    }

    async fn resolve(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
        cell: String,
    ) -> Result<Option<Edition>, Error> {
        trace("resolve");
        layers::resolve(&space(accessor, &layer).await?, &namespace, &cell).await
    }

    async fn publish(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
        cell: String,
        content: Vec<u8>,
        when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        trace("publish");
        let space = space(accessor, &layer).await?;
        layers::publish(&space, &namespace, &cell, content, when).await
    }

    async fn retract(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
        cell: String,
        when: Vec<u8>,
    ) -> Result<(), Error> {
        trace("retract");
        layers::retract(&space(accessor, &layer).await?, &namespace, &cell, when).await
    }

    async fn list(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
    ) -> Result<Vec<String>, Error> {
        trace("list");
        layers::list(&space(accessor, &layer).await?, &namespace).await
    }

    async fn watch(
        _accessor: &Accessor<T, Self>,
        _layer: Resource<Layer>,
        _namespace: String,
        _cell: String,
    ) -> Result<StreamReader<Edition>, Error> {
        trace("watch");
        Err(Error::Unavailable("this host keeps no watches".into()))
    }

    async fn size(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        key: Vec<u8>,
    ) -> Result<Option<u64>, Error> {
        trace("size");
        layers::size(&space(accessor, &layer).await?, &key).await
    }

    async fn read(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        key: Vec<u8>,
        range: ByteRange,
        _size: Option<u64>,
        _encoding: Encoding,
        _proof: bool,
    ) -> Result<BlobRead, Error> {
        trace("read");
        let space = space(accessor, &layer).await?;
        let bytes = layers::read(&space, &key, range.start, range.end).await?;
        accessor.with(|mut access| {
            let body = StreamReader::new(access.store(), bytes)
                .map_err(|error| Error::Unavailable(error.to_string()))?;
            Ok(BlobRead {
                proof: Vec::new(),
                body,
            })
        })
    }

    async fn write(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        bytes: StreamReader<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("write");
        let (collected, done) = futures::channel::oneshot::channel();
        accessor
            .with(|mut access| {
                bytes.pipe(
                    access.store(),
                    Collect {
                        bytes: Vec::new(),
                        done: Some(collected),
                    },
                )
            })
            .map_err(|error| Error::Unavailable(error.to_string()))?;
        let blob = done
            .await
            .map_err(|_| Error::BrokeOff("the stream was dropped".into()))?;
        layers::write(&space(accessor, &layer).await?, &blob).await
    }
}

/// Collects a stream of bytes, and hands them over when the guest ends it.
struct Collect {
    bytes: Vec<u8>,
    done: Option<futures::channel::oneshot::Sender<Vec<u8>>>,
}

impl<T: 'static> StreamConsumer<T> for Collect {
    type Item = u8;

    fn poll_consume(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        store: &mut StoreContext<'_, T>,
        mut source: Source<'_, u8>,
        _finish: bool,
    ) -> Poll<wcmp::Result<StreamResult>> {
        let count = source.remaining();
        let mut read = Vec::new();
        source.read(store, &mut read, count)?;
        self.bytes.extend(read);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl Drop for Collect {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            let _ = done.send(core::mem::take(&mut self.bytes));
        }
    }
}

impl application::Host for KernelHost {}

impl<T: 'static> application::HostWithStore<T> for Sapling {
    async fn send(accessor: &Accessor<T, Self>, message: Vec<u8>) -> Result<(), Error> {
        trace("send");
        accessor.with(|mut access| {
            let host = access.get();
            match &host.outbox {
                Some(outbox) => outbox
                    .unbounded_send(message)
                    .map_err(|_| Error::BrokeOff("no one takes the worker's messages".into())),
                None => {
                    host.sent.push(message);
                    Ok(())
                }
            }
        })?;
        Ok(())
    }
}

impl network::Host for KernelHost {}

/// What every call on a peer answers until this host reaches the network.
fn offline() -> Error {
    Error::Unavailable("this host has no network".into())
}

impl network::HostPeer for KernelHost {
    fn did(&mut self, _peer: Resource<network::Peer>) -> String {
        String::new()
    }

    fn drop(&mut self, _peer: Resource<network::Peer>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl network::HostReplica for KernelHost {
    fn subject(&mut self, _replica: Resource<network::Replica>) -> String {
        String::new()
    }

    fn drop(&mut self, _replica: Resource<network::Replica>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> network::HostWithStore<T> for Sapling {
    async fn open(
        _accessor: &Accessor<T, Self>,
        _peer: String,
        _routes: Vec<String>,
    ) -> Result<Resource<network::Peer>, Error> {
        trace("network.open");
        Err(offline())
    }

    async fn home(_accessor: &Accessor<T, Self>) -> Result<String, Error> {
        trace("network.home");
        Err(offline())
    }
}

impl<T: 'static> network::HostPeerWithStore<T> for Sapling {
    async fn open(
        _accessor: &Accessor<T, Self>,
        _peer: Resource<network::Peer>,
        _subject: String,
        _chain: Vec<u8>,
    ) -> Result<Resource<network::Replica>, Error> {
        trace("peer.open");
        Err(offline())
    }
}

impl<T: 'static> network::HostReplicaWithStore<T> for Sapling {
    async fn get(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _catalog: String,
        _key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, Error> {
        Err(offline())
    }

    async fn get_many(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _catalog: String,
        _keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Option<Vec<u8>>>, Error> {
        Err(offline())
    }

    async fn put(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _catalog: String,
        _key: Vec<u8>,
        _bytes: Vec<u8>,
    ) -> Result<(), Error> {
        Err(offline())
    }

    async fn resolve(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
        _cell: String,
    ) -> Result<Option<Edition>, Error> {
        Err(offline())
    }

    async fn publish(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
        _cell: String,
        _content: Vec<u8>,
        _when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        Err(offline())
    }

    async fn retract(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
        _cell: String,
        _when: Vec<u8>,
    ) -> Result<(), Error> {
        Err(offline())
    }

    async fn list(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
    ) -> Result<Vec<String>, Error> {
        Err(offline())
    }

    async fn size(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _key: Vec<u8>,
    ) -> Result<Option<u64>, Error> {
        Err(offline())
    }

    async fn read(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _key: Vec<u8>,
        _range: ByteRange,
        _size: Option<u64>,
        _encoding: Encoding,
        _proof: bool,
    ) -> Result<BlobRead, Error> {
        Err(offline())
    }

    async fn write(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _bytes: StreamReader<u8>,
    ) -> Result<Vec<u8>, Error> {
        Err(offline())
    }
}

/// The `did:key` of an Ed25519 key: base58btc of the multicodec `ed25519-pub`
/// and the public key.
pub fn did_key(key: &SigningKey) -> String {
    let mut bytes = vec![0xed, 0x01];
    bytes.extend_from_slice(&key.verifying_key().to_bytes());
    format!("did:key:z{}", bs58::encode(bytes).into_string())
}

/// Note a call into the host, with the time since the first note, when
/// `KERNEL_TRACE` is set (natively) or always on the web when asked.
pub fn trace(what: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ON: AtomicBool = AtomicBool::new(false);
    static CHECKED: AtomicBool = AtomicBool::new(false);
    if !CHECKED.swap(true, Ordering::Relaxed) {
        #[cfg(not(target_arch = "wasm32"))]
        ON.store(
            std::env::var_os("KERNEL_TRACE").is_some(),
            Ordering::Relaxed,
        );
    }
    if ON.load(Ordering::Relaxed) {
        crate::platform::log(
            "trace",
            &format!("{:9.1} {what}", crate::platform::now_millis()),
            false,
        );
    }
}
