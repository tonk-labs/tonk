//! The interfaces of `sapling:kernel` that are Sapling's own, served from
//! memory: each subject's blocks, cells, and blobs, the session key, and the
//! catch-ups a guest kernel sends. What a browser host would back with
//! IndexedDB or OPFS, and with the profile's keys.

use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::HashMap;

use ed25519_dalek::{Signer, SigningKey};
use wcmp::{
    Accessor, HasData, Resource, Source, StoreContext, StreamConsumer, StreamReader, StreamResult,
};
use zeroize::Zeroizing;

use crate::bindings::sapling::kernel::{application, network, session, storage, types};
use crate::credentials::Held;
use crate::http_types::{HttpHost, HttpTable};
use storage::{BlobRead, ByteRange, Edition, Encoding, Layer};
use types::Error;

/// What one subject holds.
#[derive(Default)]
pub struct Subject {
    blocks: HashMap<(String, Vec<u8>), Vec<u8>>,
    cells: HashMap<(String, String), Edition>,
    blobs: HashMap<Vec<u8>, Vec<u8>>,
    /// The seed of each key of the subject, by its address.
    pub(crate) keys: HashMap<String, Zeroizing<[u8; 32]>>,
    /// The secret of each site the subject reaches.
    pub(crate) secrets: HashMap<String, Vec<u8>>,
}

/// The host data of a store that runs a guest kernel.
pub struct KernelHost {
    pub http: HttpTable,
    /// The subject of each layer handle, by its representation.
    layers: Vec<String>,
    subjects: HashMap<String, Subject>,
    key: SigningKey,
    /// The credential each `credential` handle names, by its representation.
    pub(crate) credentials: Vec<Held>,
    /// The catch-ups the guest kernel sent with `application.send`.
    pub sent: Vec<Vec<u8>>,
    /// Each write since the last drain, for the host to keep durably.
    pub journal: Vec<Change>,
}

/// One write to a subject, as a host keeps it durably. Its `Debug` leaves out
/// the seed of a key and the bytes of a secret.
#[derive(Clone)]
pub enum Change {
    Block {
        subject: String,
        catalog: String,
        key: Vec<u8>,
        bytes: Vec<u8>,
    },
    Cell {
        subject: String,
        namespace: String,
        cell: String,
        edition: Option<Edition>,
    },
    Blob {
        subject: String,
        key: Vec<u8>,
        bytes: Vec<u8>,
    },
    /// The key of a subject at an address: its seed, or none once forgotten.
    Key {
        subject: String,
        address: String,
        seed: Option<Vec<u8>>,
    },
    /// The secret of a site, or none once retracted.
    Secret {
        subject: String,
        site: String,
        secret: Option<Vec<u8>>,
    },
}

impl core::fmt::Debug for Change {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Change::Block {
                subject,
                catalog,
                key,
                ..
            } => f
                .debug_struct("Block")
                .field("subject", subject)
                .field("catalog", catalog)
                .field("key", key)
                .finish_non_exhaustive(),
            Change::Cell {
                subject,
                namespace,
                cell,
                edition,
            } => f
                .debug_struct("Cell")
                .field("subject", subject)
                .field("namespace", namespace)
                .field("cell", cell)
                .field("edition", edition)
                .finish(),
            Change::Blob { subject, key, .. } => f
                .debug_struct("Blob")
                .field("subject", subject)
                .field("key", key)
                .finish_non_exhaustive(),
            Change::Key {
                subject,
                address,
                seed,
            } => f
                .debug_struct("Key")
                .field("subject", subject)
                .field("address", address)
                .field("forgotten", &seed.is_none())
                .finish_non_exhaustive(),
            Change::Secret {
                subject,
                site,
                secret,
            } => f
                .debug_struct("Secret")
                .field("subject", subject)
                .field("site", site)
                .field("retracted", &secret.is_none())
                .finish_non_exhaustive(),
        }
    }
}

impl KernelHost {
    pub fn new(seed: [u8; 32]) -> Self {
        KernelHost {
            http: HttpTable::default(),
            layers: Vec::new(),
            subjects: HashMap::new(),
            credentials: Vec::new(),
            key: SigningKey::from_bytes(&seed),
            sent: Vec::new(),
            journal: Vec::new(),
        }
    }

    /// Put back what an earlier store wrote, before the guest kernel boots.
    pub fn restore(&mut self, changes: impl IntoIterator<Item = Change>) {
        for change in changes {
            match change {
                Change::Block {
                    subject,
                    catalog,
                    key,
                    bytes,
                } => {
                    self.subjects
                        .entry(subject)
                        .or_default()
                        .blocks
                        .insert((catalog, key), bytes);
                }
                Change::Cell {
                    subject,
                    namespace,
                    cell,
                    edition,
                } => {
                    let cells = &mut self.subjects.entry(subject).or_default().cells;
                    match edition {
                        Some(edition) => cells.insert((namespace, cell), edition),
                        None => cells.remove(&(namespace, cell)),
                    };
                }
                Change::Blob {
                    subject,
                    key,
                    bytes,
                } => {
                    self.subjects
                        .entry(subject)
                        .or_default()
                        .blobs
                        .insert(key, bytes);
                }
                Change::Key {
                    subject,
                    address,
                    seed,
                } => {
                    let keys = &mut self.subjects.entry(subject).or_default().keys;
                    match seed.and_then(|seed| <[u8; 32]>::try_from(seed).ok()) {
                        Some(seed) => keys.insert(address, Zeroizing::new(seed)),
                        None => keys.remove(&address),
                    };
                }
                Change::Secret {
                    subject,
                    site,
                    secret,
                } => {
                    let secrets = &mut self.subjects.entry(subject).or_default().secrets;
                    match secret {
                        Some(secret) => secrets.insert(site, secret),
                        None => secrets.remove(&site),
                    };
                }
            }
        }
    }

    pub(crate) fn subject_name(&self, layer: &Resource<Layer>) -> String {
        self.layers[layer.rep() as usize].clone()
    }

    /// A handle on the layer of `subject`, for the boot or for `open`.
    pub fn layer(&mut self, subject: &str) -> Resource<Layer> {
        self.subjects.entry(subject.to_owned()).or_default();
        self.layers.push(subject.to_owned());
        Resource::new_own((self.layers.len() - 1) as u32)
    }

    pub(crate) fn subject(&mut self, layer: &Resource<Layer>) -> &mut Subject {
        let name = self.layers[layer.rep() as usize].clone();
        self.subjects.entry(name).or_default()
    }

    fn sign(&self, tag: &str, payload: &[u8]) -> Vec<u8> {
        let mut message = tag.as_bytes().to_vec();
        message.extend_from_slice(payload);
        self.key.sign(&message).to_bytes().to_vec()
    }
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
        self.layers[layer.rep() as usize].clone()
    }

    fn drop(&mut self, _layer: Resource<Layer>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> storage::HostWithStore<T> for Sapling {
    async fn open(
        accessor: &Accessor<T, Self>,
        subject: String,
        _chain: Vec<u8>,
    ) -> Result<Resource<Layer>, Error> {
        trace("open");
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
        Ok(accessor.with(|mut access| {
            access
                .get()
                .subject(&layer)
                .blocks
                .get(&(catalog, key))
                .cloned()
        }))
    }

    async fn get_many(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        catalog: String,
        keys: Vec<Vec<u8>>,
    ) -> Result<Vec<Option<Vec<u8>>>, Error> {
        trace("get_many");
        Ok(accessor.with(|mut access| {
            let subject = access.get().subject(&layer);
            keys.into_iter()
                .map(|key| subject.blocks.get(&(catalog.clone(), key)).cloned())
                .collect()
        }))
    }

    async fn put(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        catalog: String,
        key: Vec<u8>,
        bytes: Vec<u8>,
    ) -> Result<(), Error> {
        trace("put");
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&layer);
            host.journal.push(Change::Block {
                subject,
                catalog: catalog.clone(),
                key: key.clone(),
                bytes: bytes.clone(),
            });
            host.subject(&layer).blocks.insert((catalog, key), bytes);
        });
        Ok(())
    }

    async fn resolve(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
        cell: String,
    ) -> Result<Option<Edition>, Error> {
        trace("resolve");
        Ok(accessor.with(|mut access| {
            access
                .get()
                .subject(&layer)
                .cells
                .get(&(namespace, cell))
                .cloned()
        }))
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
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&layer);
            let cells = &mut host.subject(&layer).cells;
            let name = (namespace, cell);
            if cells.get(&name).map(|held| &held.version) != when.as_ref() {
                return Err(Error::Conflict);
            }
            let version = blake3::hash(&content).as_bytes().to_vec();
            let edition = Edition {
                content,
                version: version.clone(),
            };
            cells.insert(name.clone(), edition.clone());
            host.journal.push(Change::Cell {
                subject,
                namespace: name.0,
                cell: name.1,
                edition: Some(edition),
            });
            Ok(version)
        })
    }

    async fn retract(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
        cell: String,
        when: Vec<u8>,
    ) -> Result<(), Error> {
        trace("retract");
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&layer);
            let cells = &mut host.subject(&layer).cells;
            let name = (namespace, cell);
            match cells.get(&name) {
                Some(held) if held.version == when => {
                    cells.remove(&name);
                    host.journal.push(Change::Cell {
                        subject,
                        namespace: name.0,
                        cell: name.1,
                        edition: None,
                    });
                    Ok(())
                }
                _ => Err(Error::Conflict),
            }
        })
    }

    async fn list(
        accessor: &Accessor<T, Self>,
        layer: Resource<Layer>,
        namespace: String,
    ) -> Result<Vec<String>, Error> {
        trace("list");
        Ok(accessor.with(|mut access| {
            access
                .get()
                .subject(&layer)
                .cells
                .keys()
                .filter(|(held, _)| *held == namespace)
                .map(|(_, cell)| cell.clone())
                .collect()
        }))
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
        Ok(accessor.with(|mut access| {
            access
                .get()
                .subject(&layer)
                .blobs
                .get(&key)
                .map(|blob| blob.len() as u64)
        }))
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
        accessor.with(|mut access| {
            let bytes = {
                let Some(blob) = access.get().subject(&layer).blobs.get(&key) else {
                    return Err(Error::Invalid("no such blob".into()));
                };
                let start = (range.start as usize).min(blob.len());
                let end = range
                    .end
                    .map_or(blob.len(), |end| (end as usize).min(blob.len()));
                blob[start..end.max(start)].to_vec()
            };
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
        let key = blake3::hash(&blob).as_bytes().to_vec();
        accessor.with(|mut access| {
            let host = access.get();
            let subject = host.subject_name(&layer);
            host.journal.push(Change::Blob {
                subject,
                key: key.clone(),
                bytes: blob.clone(),
            });
            host.subject(&layer).blobs.insert(key.clone(), blob);
        });
        Ok(key)
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

impl session::Host for KernelHost {
    fn key(&mut self) -> String {
        did_key(&self.key)
    }

    fn chain(&mut self) -> Vec<u8> {
        Vec::new()
    }
}

impl<T: 'static> session::HostWithStore<T> for Sapling {
    async fn sign_head(accessor: &Accessor<T, Self>, payload: Vec<u8>) -> Result<Vec<u8>, Error> {
        trace("sign_head");
        Ok(accessor.with(|mut access| access.get().sign("dialog/head@1\n", &payload)))
    }

    async fn sign_revision_record(
        accessor: &Accessor<T, Self>,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        trace("sign_revision_record");
        Ok(accessor.with(|mut access| access.get().sign("dialog/revision-record@1\n", &payload)))
    }
}

impl application::Host for KernelHost {}

impl<T: 'static> application::HostWithStore<T> for Sapling {
    async fn send(accessor: &Accessor<T, Self>, catchup: Vec<u8>) -> Result<(), Error> {
        trace("send");
        accessor.with(|mut access| access.get().sent.push(catchup));
        Ok(())
    }
}

impl network::Host for KernelHost {}

impl network::HostReplica for KernelHost {
    fn peer(&mut self, _replica: Resource<network::Replica>) -> String {
        String::new()
    }

    fn subject(&mut self, _replica: Resource<network::Replica>) -> String {
        String::new()
    }

    fn drop(&mut self, _replica: Resource<network::Replica>) -> wcmp::Result<()> {
        Ok(())
    }
}

impl<T: 'static> network::HostWithStore<T> for Sapling {
    async fn open_remote(
        _accessor: &Accessor<T, Self>,
        _peer: String,
        _routes: Vec<String>,
        _subject: String,
    ) -> Result<Resource<network::Replica>, Error> {
        trace("open_remote");
        Err(Error::Unavailable("this host has no network".into()))
    }
}

impl<T: 'static> network::HostReplicaWithStore<T> for Sapling {
    async fn get(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _catalog: String,
        _key: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, Error> {
        trace("get");
        Err(Error::Unavailable("this host has no network".into()))
    }

    async fn resolve(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
        _cell: String,
    ) -> Result<Option<Edition>, Error> {
        trace("resolve");
        Err(Error::Unavailable("this host has no network".into()))
    }

    async fn list(
        _accessor: &Accessor<T, Self>,
        _replica: Resource<network::Replica>,
        _namespace: String,
    ) -> Result<Vec<String>, Error> {
        trace("list");
        Err(Error::Unavailable("this host has no network".into()))
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
