//! Content-addressed objects served from a cache in front of the store.
//!
//! Blocks and blobs are named by their content, so a copy anywhere is
//! as good as the bucket's: a read looks in the cache first and serves
//! what it finds, a miss goes to the store and fills the cache behind
//! the answer, and a write fills it too, so a reader that follows in
//! the same place never reaches the bucket. Cells are mutable and never
//! cached.
//!
//! The policy is written over any provider and any [`ObjectCache`], so
//! it runs natively against the in-memory store in tests; the worker
//! plugs in the Cache API ([`worker::WorkerCache`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use base58::ToBase58;
use dialog_capability::{Capability, Policy, Provider};
use dialog_common::{Blake3Hash, ConditionalSend, ConditionalSync};
use dialog_effects::archive::prelude::{GetExt, PutExt};
use dialog_effects::archive::{self, ArchiveError, Catalog};
use dialog_effects::blob::prelude::{BlobImportExt as _, BlobReadExt as _};
use dialog_effects::blob::{self, BlobError, BlobReader, BlobSink, BlobWriter, ByteRange};
use dialog_effects::memory::{self, Edition, MemoryError, Version};

#[cfg(target_arch = "wasm32")]
pub mod worker;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod test;

/// The key a block is stored under: `{subject}/{catalog}/{digest}`.
pub fn block_key<Fx>(capability: &Capability<Fx>, digest: &Blake3Hash) -> String
where
    Fx: Policy<Of = Catalog>,
{
    format!(
        "{}/{}/{}",
        capability.subject(),
        Catalog::of(capability).catalog,
        digest.as_bytes().to_base58()
    )
}

/// The key a blob is stored under: `{subject}/blob/{digest}`.
pub fn blob_key<Fx>(capability: &Capability<Fx>, digest: &Blake3Hash) -> String
where
    Fx: Policy<Of = blob::Blob>,
{
    format!(
        "{}/blob/{}",
        capability.subject(),
        digest.as_bytes().to_base58()
    )
}

/// Somewhere content-addressed objects are kept near the reader.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait ObjectCache: ConditionalSend + ConditionalSync + 'static {
    /// The object under `key`, or the `range` of it, if the cache holds
    /// it. A cache that cannot answer a range from what it holds
    /// answers `None` and the store serves it.
    async fn read(&self, key: &str, range: Option<ByteRange>) -> Option<BlobReader>;

    /// Keep `bytes` under `key`. Content-addressed objects never change,
    /// so there is nothing to invalidate.
    async fn write(&self, key: &str, bytes: Vec<u8>);
}

/// Whether reads consult the cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    /// Serve from the cache when it holds the object.
    #[default]
    ReadThrough,
    /// Never serve from the cache; still fill it. For seeing what the
    /// store answers when the cache would have answered instead.
    Bypass,
}

/// What the cache did for the last operation, for `Server-Timing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Served from the cache.
    Hit,
    /// Served from the store; the cache is being filled behind the
    /// answer when there was something to fill it with.
    Miss,
    /// Served from the store because reads bypass the cache.
    Bypass,
    /// A write; the cache is being filled behind the answer.
    Fill,
    /// An operation the cache takes no part in.
    None,
}

impl Outcome {
    /// The word `Server-Timing` carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Hit => "hit",
            Outcome::Miss => "miss",
            Outcome::Bypass => "bypass",
            Outcome::Fill => "fill",
            Outcome::None => "none",
        }
    }
}

/// A cache fill, run after the answer has gone out.
pub trait Deferred: Future<Output = ()> + ConditionalSend {}
impl<T: Future<Output = ()> + ConditionalSend> Deferred for T {}

/// A fill the embedder runs once the response is on its way.
pub type Fill = Pin<Box<dyn Deferred>>;

/// A provider whose content-addressed reads are answered from `cache`
/// when it can, and whose writes fill it.
pub struct Cached<P, C> {
    store: P,
    cache: Arc<C>,
    mode: Mode,
    outcome: Mutex<Outcome>,
    /// Shared with a blob sink, which finishes after the call that made
    /// it returned and hands its fill over through the same list.
    fills: Arc<Mutex<Vec<Fill>>>,
}

impl<P, C> Cached<P, C> {
    /// `store` behind `cache`.
    pub fn new(store: P, cache: C) -> Self {
        Self {
            store,
            cache: Arc::new(cache),
            mode: Mode::default(),
            outcome: Mutex::new(Outcome::None),
            fills: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The same provider, consulting the cache as `mode` says.
    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// What the cache did for the last operation.
    pub fn outcome(&self) -> Outcome {
        *self.outcome.lock().expect("outcome lock")
    }

    /// The fills the last operations left to run, for the embedder to
    /// run once the response is on its way.
    pub fn take_fills(&self) -> Vec<Fill> {
        std::mem::take(&mut *self.fills.lock().expect("fills lock"))
    }

    fn note(&self, outcome: Outcome) {
        *self.outcome.lock().expect("outcome lock") = outcome;
    }

    fn defer(&self, fill: impl Future<Output = ()> + ConditionalSend + 'static) {
        self.fills.lock().expect("fills lock").push(Box::pin(fill));
    }
}

/// A source read to its end.
async fn collect(mut source: BlobReader) -> Result<Vec<u8>, BlobError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = source.next().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<archive::Get> for Cached<P, C>
where
    P: Provider<archive::Get> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(
        &self,
        capability: Capability<archive::Get>,
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        let key = block_key(&capability, capability.digest());
        if self.mode == Mode::ReadThrough
            && let Some(source) = self.cache.read(&key, None).await
            && let Ok(bytes) = collect(source).await
        {
            self.note(Outcome::Hit);
            return Ok(Some(bytes));
        }
        self.note(match self.mode {
            Mode::ReadThrough => Outcome::Miss,
            Mode::Bypass => Outcome::Bypass,
        });
        let found = Provider::<archive::Get>::execute(&self.store, capability).await?;
        if let Some(bytes) = &found {
            let cache = self.cache.clone();
            let bytes = bytes.clone();
            self.defer(async move { cache.write(&key, bytes).await });
        }
        Ok(found)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<archive::Put> for Cached<P, C>
where
    P: Provider<archive::Put> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(&self, capability: Capability<archive::Put>) -> Result<(), ArchiveError> {
        let content = capability.content().to_vec();
        let key = block_key(&capability, &Blake3Hash::hash(&content));
        Provider::<archive::Put>::execute(&self.store, capability).await?;
        self.note(Outcome::Fill);
        let cache = self.cache.clone();
        self.defer(async move { cache.write(&key, content).await });
        Ok(())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<blob::Read> for Cached<P, C>
where
    P: Provider<blob::Read> + Clone + ConditionalSync + 'static,
    C: ObjectCache,
{
    async fn execute(&self, capability: Capability<blob::Read>) -> Result<BlobReader, BlobError> {
        let key = blob_key(&capability, capability.digest());
        let range = capability.range();
        if self.mode == Mode::ReadThrough
            && let Some(source) = self.cache.read(&key, range).await
        {
            self.note(Outcome::Hit);
            return Ok(source);
        }
        self.note(match self.mode {
            Mode::ReadThrough => Outcome::Miss,
            Mode::Bypass => Outcome::Bypass,
        });
        // A whole blob's bytes stream out to the reader and cannot be
        // kept as they pass; the fill reads the blob again, behind the
        // answer, once per cache. A range is not kept: the cache serves
        // ranges out of a whole it holds.
        let whole = range.is_none().then(|| capability.clone());
        let source = Provider::<blob::Read>::execute(&self.store, capability).await?;
        if let Some(capability) = whole {
            let store = self.store.clone();
            let cache = self.cache.clone();
            self.defer(async move {
                if let Ok(source) = Provider::<blob::Read>::execute(&store, capability).await
                    && let Ok(bytes) = collect(source).await
                {
                    cache.write(&key, bytes).await;
                }
            });
        }
        Ok(source)
    }
}

/// A sink that keeps a copy of what it is given and fills the cache
/// with it once the store has accepted the blob.
struct Filling<C> {
    inner: BlobWriter,
    key: String,
    buffer: Vec<u8>,
    cache: Arc<C>,
    fills: Arc<Mutex<Vec<Fill>>>,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<C: ObjectCache> BlobSink for Filling<C> {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), BlobError> {
        self.inner.write_all(bytes).await?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<Blake3Hash, BlobError> {
        let Filling {
            inner,
            key,
            buffer,
            cache,
            fills,
        } = *self;
        let hash = inner.finish().await?;
        fills
            .lock()
            .expect("fills lock")
            .push(Box::pin(async move { cache.write(&key, buffer).await }));
        Ok(hash)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<blob::Import> for Cached<P, C>
where
    P: Provider<blob::Import> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(&self, capability: Capability<blob::Import>) -> Result<BlobWriter, BlobError> {
        let key = blob_key(&capability, capability.digest());
        let inner = Provider::<blob::Import>::execute(&self.store, capability).await?;
        self.note(Outcome::Fill);
        Ok(Box::new(Filling {
            inner,
            key,
            buffer: Vec::new(),
            cache: self.cache.clone(),
            fills: self.fills.clone(),
        }))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<memory::Resolve> for Cached<P, C>
where
    P: Provider<memory::Resolve> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(
        &self,
        capability: Capability<memory::Resolve>,
    ) -> Result<Option<Edition<Vec<u8>>>, MemoryError> {
        self.note(Outcome::None);
        Provider::<memory::Resolve>::execute(&self.store, capability).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<memory::Publish> for Cached<P, C>
where
    P: Provider<memory::Publish> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(
        &self,
        capability: Capability<memory::Publish>,
    ) -> Result<Version, MemoryError> {
        self.note(Outcome::None);
        Provider::<memory::Publish>::execute(&self.store, capability).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<P, C> Provider<memory::Retract> for Cached<P, C>
where
    P: Provider<memory::Retract> + ConditionalSync,
    C: ObjectCache,
{
    async fn execute(&self, capability: Capability<memory::Retract>) -> Result<(), MemoryError> {
        self.note(Outcome::None);
        Provider::<memory::Retract>::execute(&self.store, capability).await
    }
}
