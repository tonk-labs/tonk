//! A layer: the cells, the blocks, and the blobs of one subject, as the world
//! `sapling:kernel` serves them. See PDD007, "Storage".
//!
//! [`Layer`] has the calls of `storage.layer`, and [`LayerError`] the cases of
//! the `error` of the world. A guest kernel implements it over its handle (see
//! `guest`), and the daemon can implement it over the cells of a store.
//! [`MemoryLayer`] holds one in memory, for tests.
//!
//! [`Layer::watch`] follows one cell: a [`Watch`] gives each new edition of
//! it. A layer that serves no watch refuses it as unavailable, and its reader
//! reads the cell again at each look instead.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use dialog_common::{ConditionalSend, ConditionalSync};
use dialog_varsig::Did;

/// The content of a cell, and its version, for compare and set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edition {
    pub content: Vec<u8>,
    pub version: Vec<u8>,
}

/// Why a layer did not do a call: the `error` of the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerError {
    /// The scope of the handle, or the chain, does not allow this.
    Denied(String),
    /// A compare and set lost a race. The cell has a new version.
    Conflict,
    /// The chain of the handle expired. Open the layer again.
    Expired,
    /// The input is not valid.
    Invalid(String),
    /// A peer, or the storage, did not answer.
    Unavailable(String),
    /// No route to a peer answered.
    Unreachable(String),
    /// An exchange with a peer broke off, or got no answer in time.
    BrokeOff(String),
    /// The blob store has no room for the bytes, as the quota of the device
    /// says.
    Full(String),
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(reason) => write!(f, "denied: {reason}"),
            Self::Conflict => f.write_str("conflict: the cell has a new version"),
            Self::Expired => f.write_str("expired: open the layer again"),
            Self::Invalid(reason) => write!(f, "invalid: {reason}"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
            Self::Unreachable(reason) => write!(f, "unreachable: {reason}"),
            Self::BrokeOff(reason) => write!(f, "broke off: {reason}"),
            Self::Full(reason) => write!(f, "full: {reason}"),
        }
    }
}

impl std::error::Error for LayerError {}

/// One open layer of one subject. Each call is the call of `storage.layer`
/// of the same name. A hash is a BLAKE3 hash of 32 bytes; a version is opaque.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait Layer: ConditionalSend + ConditionalSync + 'static {
    /// The DID of the subject of the layer.
    fn subject(&self) -> Did;

    /// The block of `key` in `catalog`, if any.
    async fn get(&self, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, LayerError>;
    /// Puts `bytes`, whose hash is `key`, in `catalog`.
    async fn put(&self, catalog: &str, key: &[u8], bytes: Vec<u8>) -> Result<(), LayerError>;

    /// The edition of `cell` in `namespace`, if any.
    async fn resolve(&self, namespace: &str, cell: &str) -> Result<Option<Edition>, LayerError>;
    /// Sets `cell` to `content`, if its version is `when` (none for a cell
    /// that does not exist), and gives the new version.
    async fn publish(
        &self,
        namespace: &str,
        cell: &str,
        content: Vec<u8>,
        when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, LayerError>;
    /// Removes `cell`, if its version is `when`.
    async fn retract(&self, namespace: &str, cell: &str, when: Vec<u8>) -> Result<(), LayerError>;
    /// The cells under `namespace`, by their paths from it: in the namespace
    /// itself, and in each namespace under it, as Dialog DB lists a memory
    /// space (`branch` lists `main/revision` of the namespace `branch/main`).
    async fn list(&self, namespace: &str) -> Result<Vec<String>, LayerError>;

    /// The size of the blob of `key`, if any.
    async fn size(&self, key: &[u8]) -> Result<Option<u64>, LayerError>;
    /// The bytes of the blob of `key`, from `start` to `end` or to its end.
    async fn read(&self, key: &[u8], start: u64, end: Option<u64>) -> Result<Vec<u8>, LayerError>;
    /// Writes a blob, and gives its hash.
    async fn write(&self, bytes: Vec<u8>) -> Result<Vec<u8>, LayerError>;

    /// Starts the write of a blob that takes its bytes a chunk at a time, if
    /// this layer streams: a layer that does not gives none, and the store
    /// writes the blob whole. See PDD007, "Storage".
    async fn write_stream(self: Arc<Self>) -> Result<Option<Box<dyn BlobStream>>, LayerError> {
        Ok(None)
    }

    /// The bytes of the blob of `key`, from `start` to `end` or to its end, a
    /// chunk at a time, if this layer streams: a
    /// layer that does not gives none, and the store reads the blob whole.
    /// With no `size`, it reads only what the store holds. With the size of
    /// the blob, from the fact of its asset, the daemon also fetches what the
    /// store lacks from the peers of the subject.
    async fn read_stream(
        &self,
        key: &[u8],
        (start, end): (u64, Option<u64>),
        size: Option<u64>,
    ) -> Result<Option<BlobRead>, LayerError> {
        let _ = (key, start, end, size);
        Ok(None)
    }
}

/// The write of a blob a chunk at a time: [`Layer::write_stream`].
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait BlobStream: ConditionalSend {
    /// Writes the next chunk.
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), LayerError>;
    /// Ends the write, and gives the hash of the blob.
    async fn finish(self: Box<Self>) -> Result<Vec<u8>, LayerError>;
}

/// A read of a blob that streams: its chunks. See [`Layer::read_stream`].
pub struct BlobRead {
    pub chunks: Box<dyn BlobChunks>,
}

/// The read of a blob a chunk at a time: [`Layer::read_stream`].
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait BlobChunks: ConditionalSend {
    /// The next chunk, or none at the end.
    async fn next(&mut self) -> Option<Vec<u8>>;
}

/// A layer that can be shared: across threads, off the web.
#[cfg(not(target_arch = "wasm32"))]
pub type SharedLayer = Arc<dyn Layer + Send + Sync>;
/// A layer that can be shared.
#[cfg(target_arch = "wasm32")]
pub type SharedLayer = Arc<dyn Layer>;

/// `layer`, to share.
#[cfg(not(target_arch = "wasm32"))]
pub fn share(layer: impl Layer + Send + Sync) -> SharedLayer {
    Arc::new(layer)
}

/// `layer`, to share.
#[cfg(target_arch = "wasm32")]
pub fn share(layer: impl Layer) -> SharedLayer {
    Arc::new(layer)
}
