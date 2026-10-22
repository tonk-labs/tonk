//! The providers of Dialog DB over a [`Layer`]: the archive, the memory, and
//! the blobs of a space. See PDD007, "Storage".
//!
//! [`Store`] serves each effect with one call of its layer. The word
//! "namespace" of the world is a memory space of Dialog DB. A layer that
//! streams reads and writes a blob a chunk at a time, and any other layer
//! reads and writes it whole. The watch of a cell is left out: Dialog DB
//! reaches it through a fork, not through a space.
//!
//! Each `error` of the world becomes the error of Dialog DB that means the
//! same, so that Dialog DB acts on it as on its own:
//!
//! | `error`       | Dialog DB                                             |
//! | ------------- | ----------------------------------------------------- |
//! | `denied`      | `AuthorizeError::Declined`, with no recourse          |
//! | `conflict`    | `MemoryError::VersionMismatch` (a storage error for blocks and blobs) |
//! | `expired`     | `AuthorizeError::Expired`                             |
//! | `invalid`     | a storage error                                       |
//! | `unavailable` | `Rejection::Unavailable`, which a caller can retry    |
//! | `unreachable` | `Rejection::Unavailable`                              |
//! | `broke-off`   | `Rejection::Unavailable`                              |
//! | `full`        | a storage error                                       |
//!
//! Dialog DB keeps the error of a provider as text in places, so each store
//! also keeps the last [`LayerError`] that it saw, in its [`Witness`]: the
//! helpers of `repository` read it to tell a caller what the layer said. The
//! witness also keeps whether a get found no block: on the replica of a peer,
//! a block that the peer does not serve.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use dialog_capability::access::{AuthorizeError, Recourse};
use dialog_capability::{Capability, Provider};
use dialog_common::Blake3Hash;
use dialog_effects::Rejection;
use dialog_effects::archive::prelude::{GetExt as _, ImportExt as _, PutExt as _};
use dialog_effects::archive::{self, ArchiveError};
use dialog_effects::blob::prelude::{BlobImportExt as _, BlobReadExt as _, BlobSizeExt as _};
use dialog_effects::blob::{self, BlobError, BlobReader, BlobSink, BlobSource, BlobWriter};
use dialog_effects::memory::prelude::{
    ListExt as _, PublishExt as _, ResolveExt as _, RetractExt as _,
};
use dialog_effects::memory::{self, MemoryError, Version};

use crate::layer::{LayerError, SharedLayer};

/// The last error that the layer of a store gave, if any, and whether a get
/// found no block since the witness was cleared. Clones share it.
#[derive(Clone, Default)]
pub struct Witness(Arc<Mutex<Option<LayerError>>>, Arc<AtomicBool>);

impl Witness {
    /// Forgets the last error, and each get that found no block.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.1.store(false, Ordering::SeqCst);
    }

    /// The last error, if any.
    pub fn last(&self) -> Option<LayerError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn saw(&self, error: &LayerError) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(error.clone());
    }
}

/// The archive, the memory, and the blobs of a space, over one layer.
#[derive(Clone)]
pub struct Store {
    layer: SharedLayer,
    witness: Witness,
}

impl Store {
    /// A store over `layer`, which keeps each error of the layer in `witness`.
    pub fn new(layer: SharedLayer, witness: Witness) -> Self {
        Self { layer, witness }
    }

    /// The layer of the store.
    pub fn layer(&self) -> &SharedLayer {
        &self.layer
    }

    /// `result`, with its error kept in the witness.
    fn seen<T>(&self, result: Result<T, LayerError>) -> Result<T, LayerError> {
        if let Err(error) = &result {
            self.witness.saw(error);
        }
        result
    }
}

/// The seconds since the Unix epoch, now.
fn now() -> u64 {
    dialog_common::time::now()
        .duration_since(dialog_common::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The decision of Dialog DB for `denied` and `expired`, if `error` is one.
fn decision(error: &LayerError) -> Option<AuthorizeError> {
    match error {
        LayerError::Denied(reason) => Some(AuthorizeError::Declined {
            recourse: Recourse::None,
            reason: reason.clone(),
        }),
        LayerError::Expired => Some(AuthorizeError::Expired {
            expiration: now(),
            at: now(),
        }),
        _ => None,
    }
}

fn archive_error(error: LayerError) -> ArchiveError {
    if let Some(decision) = decision(&error) {
        return ArchiveError::Authorization(decision);
    }
    match error {
        LayerError::Unavailable(reason)
        | LayerError::Unreachable(reason)
        | LayerError::BrokeOff(reason) => ArchiveError::Rejected(Rejection::unavailable(reason)),
        other => ArchiveError::Storage(other.to_string()),
    }
}

fn memory_error(error: LayerError, when: Option<&Version>) -> MemoryError {
    if let Some(decision) = decision(&error) {
        return MemoryError::Authorization(decision);
    }
    match error {
        LayerError::Conflict => MemoryError::VersionMismatch {
            expected: when.cloned(),
            current: memory::Current::Unknown,
        },
        LayerError::Unavailable(reason)
        | LayerError::Unreachable(reason)
        | LayerError::BrokeOff(reason) => MemoryError::Rejected(Rejection::unavailable(reason)),
        other => MemoryError::Storage(other.to_string()),
    }
}

fn blob_error(error: LayerError) -> BlobError {
    if let Some(decision) = decision(&error) {
        return BlobError::Authorization(decision);
    }
    match error {
        LayerError::Unavailable(reason)
        | LayerError::Unreachable(reason)
        | LayerError::BrokeOff(reason) => BlobError::Rejected(Rejection::unavailable(reason)),
        other => BlobError::Storage(other.to_string()),
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<archive::Get> for Store {
    async fn execute(
        &self,
        effect: Capability<archive::Get>,
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        let result = self
            .layer
            .get(effect.catalog(), effect.digest().as_bytes())
            .await;
        if matches!(result, Ok(None)) {
            self.witness.1.store(true, Ordering::SeqCst);
        }
        self.seen(result).map_err(archive_error)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<archive::Put> for Store {
    async fn execute(&self, effect: Capability<archive::Put>) -> Result<(), ArchiveError> {
        let result = self
            .layer
            .put(
                effect.catalog(),
                effect.digest().as_bytes(),
                effect.content().to_vec(),
            )
            .await;
        self.seen(result).map_err(archive_error)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<archive::Import> for Store {
    async fn execute(&self, effect: Capability<archive::Import>) -> Result<(), ArchiveError> {
        let catalog = effect.catalog();
        for block in effect.blocks() {
            let result = self
                .layer
                .put(
                    catalog,
                    block.blake3_hash().as_bytes(),
                    block.as_ref().to_vec(),
                )
                .await;
            self.seen(result).map_err(archive_error)?;
        }
        Ok(())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<memory::Resolve> for Store {
    async fn execute(
        &self,
        effect: Capability<memory::Resolve>,
    ) -> Result<Option<memory::Edition<Vec<u8>>>, MemoryError> {
        let result = self.layer.resolve(effect.space(), effect.cell()).await;
        let edition = self
            .seen(result)
            .map_err(|error| memory_error(error, None))?;
        Ok(edition.map(|edition| memory::Edition {
            content: edition.content,
            version: Version::from(edition.version),
        }))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<memory::Publish> for Store {
    async fn execute(&self, effect: Capability<memory::Publish>) -> Result<Version, MemoryError> {
        let when = effect.when();
        let result = self
            .layer
            .publish(
                effect.space(),
                effect.cell(),
                effect.content().to_vec(),
                when.map(|version| version.as_bytes().to_vec()),
            )
            .await;
        let version = self
            .seen(result)
            .map_err(|error| memory_error(error, when))?;
        Ok(Version::from(version))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<memory::Retract> for Store {
    async fn execute(&self, effect: Capability<memory::Retract>) -> Result<(), MemoryError> {
        let when = effect.when();
        let result = self
            .layer
            .retract(effect.space(), effect.cell(), when.as_bytes().to_vec())
            .await;
        self.seen(result)
            .map_err(|error| memory_error(error, Some(when)))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<memory::List> for Store {
    async fn execute(&self, effect: Capability<memory::List>) -> Result<Vec<String>, MemoryError> {
        let result = self.layer.list(effect.space()).await;
        self.seen(result).map_err(|error| memory_error(error, None))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<blob::Size> for Store {
    async fn execute(&self, effect: Capability<blob::Size>) -> Result<Option<u64>, BlobError> {
        let result = self.layer.size(effect.digest().as_bytes()).await;
        self.seen(result).map_err(blob_error)
    }
}

/// The bytes of a blob, read whole, as one chunk.
struct Whole(Option<Vec<u8>>);

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BlobSource for Whole {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        Ok(self.0.take())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<blob::Read> for Store {
    async fn execute(&self, effect: Capability<blob::Read>) -> Result<BlobReader, BlobError> {
        // The daemon serves the verified ranges of a blob to peers; a layer of
        // the world reads raw bytes alone. See PDD013, "A Peer Serves What It
        // Holds".
        let key = effect.digest().as_bytes().to_vec();
        // A blob that the layer does not hold is not found, as Dialog DB reads
        // it, not a failure of the layer.
        let size = self.layer.size(&key).await;
        let Some(size) = self.seen(size).map_err(blob_error)? else {
            return Err(BlobError::NotFound(effect.digest().to_string()));
        };
        let (start, end) = match effect.range() {
            None => (0, None),
            Some(range) => (
                range.offset,
                range
                    .length
                    .map(|length| range.offset.saturating_add(length)),
            ),
        };
        // A layer that streams gives the blob a chunk at a time.
        let streamed = self.layer.read_stream(&key, (start, end), None).await;
        if let Some(crate::layer::BlobRead { chunks, .. }) =
            self.seen(streamed).map_err(blob_error)?
        {
            let stop = end.map_or(size, |end| end.min(size));
            return Ok(Box::new(Chunked {
                chunks,
                left: stop.saturating_sub(start),
            }));
        }
        let result = self.layer.read(&key, start, end).await;
        let bytes = self.seen(result).map_err(blob_error)?;
        Ok(Box::new(Whole(Some(bytes))))
    }
}

/// A blob read a chunk at a time, as a layer that streams gives it, with the
/// bytes of the range still to come: a stream that ends before them, as one
/// whose layer fails, is an error, not a short read.
struct Chunked {
    chunks: Box<dyn crate::layer::BlobChunks>,
    left: u64,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BlobSource for Chunked {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        match self.chunks.next().await {
            Some(chunk) => {
                self.left = self.left.saturating_sub(chunk.len() as u64);
                Ok(Some(chunk))
            }
            None if self.left > 0 => Err(BlobError::Storage(format!(
                "the read of the blob ended {} bytes short",
                self.left
            ))),
            None => Ok(None),
        }
    }
}

/// A blob on its way to a layer that streams, a chunk at a time, with its
/// hash as it goes, to check against the hash that an import declares.
struct Streamed {
    store: Store,
    stream: Box<dyn crate::layer::BlobStream>,
    hasher: blake3::Hasher,
    expected: Option<Blake3Hash>,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BlobSink for Streamed {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), BlobError> {
        self.hasher.update(bytes);
        let written = self.stream.write_all(bytes).await;
        self.store.seen(written).map_err(blob_error)
    }

    async fn finish(self: Box<Self>) -> Result<Blake3Hash, BlobError> {
        let Streamed {
            store,
            stream,
            hasher,
            expected,
        } = *self;
        let hash = Blake3Hash::from(*hasher.finalize().as_bytes());
        // The layer keeps the blob under the hash of its bytes in any case:
        // an import of other bytes than it declares fails, and what the
        // layer kept names no fact.
        let written = stream.finish().await;
        let written = store.seen(written).map_err(blob_error)?;
        if let Some(expected) = expected
            && expected != hash
        {
            return Err(BlobError::DigestMismatch {
                expected: expected.to_string(),
                actual: hash.to_string(),
            });
        }
        if written != hash.as_bytes() {
            return Err(BlobError::Storage(
                "the layer wrote the blob under another hash".into(),
            ));
        }
        Ok(hash)
    }
}

impl Store {
    /// A sink for a blob into the layer: a chunk at a time, if the layer
    /// streams, or else buffered and written whole.
    async fn sink(&self, expected: Option<Blake3Hash>) -> Result<BlobWriter, BlobError> {
        let streamed = std::sync::Arc::clone(&self.layer).write_stream().await;
        if let Some(stream) = self.seen(streamed).map_err(blob_error)? {
            return Ok(Box::new(Streamed {
                store: self.clone(),
                stream,
                hasher: blake3::Hasher::new(),
                expected,
            }));
        }
        Ok(Box::new(Buffered {
            store: self.clone(),
            bytes: Vec::new(),
            expected,
        }))
    }
}

impl Store {}

/// A blob on its way to the layer: buffered, and written whole at
/// [`finish`](BlobSink::finish).
struct Buffered {
    store: Store,
    bytes: Vec<u8>,
    /// The hash that an import declares, checked at `finish`.
    expected: Option<Blake3Hash>,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BlobSink for Buffered {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), BlobError> {
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<Blake3Hash, BlobError> {
        let Buffered {
            store,
            bytes,
            expected,
        } = *self;
        let hash = Blake3Hash::hash(&bytes);
        if let Some(expected) = expected
            && expected != hash
        {
            return Err(BlobError::DigestMismatch {
                expected: expected.to_string(),
                actual: hash.to_string(),
            });
        }
        let result = store.layer.write(bytes).await;
        let written = store.seen(result).map_err(blob_error)?;
        if written != hash.as_bytes() {
            return Err(BlobError::Storage(
                "the layer wrote the blob under another hash".into(),
            ));
        }
        Ok(hash)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<blob::Write> for Store {
    async fn execute(&self, _effect: Capability<blob::Write>) -> Result<BlobWriter, BlobError> {
        self.sink(None).await
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Provider<blob::Import> for Store {
    async fn execute(&self, effect: Capability<blob::Import>) -> Result<BlobWriter, BlobError> {
        self.sink(Some(effect.digest().clone())).await
    }
}
