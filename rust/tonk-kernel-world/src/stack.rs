//! The layer of a worker's own: the one its stack holds under `memory:space`,
//! which keeps everything the worker keeps. Dialog DB keeps each part of a
//! space at a location of its own, so each location the worker opens other
//! than its subject's is a part of that layer ([`Within`]): its catalogs and
//! namespaces under the location's name. A blob is the layer's own, since its
//! key is its hash.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use dialog_varsig::Did;

use crate::layer::{BlobRead, BlobStream, Edition, Layer, LayerError, SharedLayer};

struct Held(SharedLayer);

// SAFETY: a component on WASI has no threads, so the layer is never reached
// from another thread.
unsafe impl Send for Held {}

static STACK: Mutex<Option<Held>> = Mutex::new(None);

/// Keep `layer`, the layer the boot hands this worker under `memory:space`,
/// as the one it keeps everything in.
pub fn keep(layer: SharedLayer) {
    *STACK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held(layer));
}

/// The layer this worker keeps everything in, if its boot handed it one.
pub fn layer() -> Option<SharedLayer> {
    STACK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| Arc::clone(&held.0))
}

/// The part of `layer` that a location of its own is: its catalogs and
/// namespaces under `prefix`.
pub struct Within {
    layer: SharedLayer,
    prefix: String,
}

impl Within {
    pub fn new(layer: SharedLayer, prefix: impl Into<String>) -> Self {
        Self {
            layer,
            prefix: prefix.into(),
        }
    }

    fn named(&self, name: &str) -> String {
        format!("{}/{name}", self.prefix)
    }
}

#[async_trait(?Send)]
impl Layer for Within {
    fn subject(&self) -> Did {
        self.layer.subject()
    }

    async fn get(&self, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, LayerError> {
        self.layer.get(&self.named(catalog), key).await
    }

    async fn put(&self, catalog: &str, key: &[u8], bytes: Vec<u8>) -> Result<(), LayerError> {
        self.layer.put(&self.named(catalog), key, bytes).await
    }

    async fn resolve(&self, namespace: &str, cell: &str) -> Result<Option<Edition>, LayerError> {
        self.layer.resolve(&self.named(namespace), cell).await
    }

    async fn publish(
        &self,
        namespace: &str,
        cell: &str,
        content: Vec<u8>,
        when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, LayerError> {
        self.layer
            .publish(&self.named(namespace), cell, content, when)
            .await
    }

    async fn retract(&self, namespace: &str, cell: &str, when: Vec<u8>) -> Result<(), LayerError> {
        self.layer.retract(&self.named(namespace), cell, when).await
    }

    async fn list(&self, namespace: &str) -> Result<Vec<String>, LayerError> {
        self.layer.list(&self.named(namespace)).await
    }

    async fn size(&self, key: &[u8]) -> Result<Option<u64>, LayerError> {
        self.layer.size(key).await
    }

    async fn read(&self, key: &[u8], start: u64, end: Option<u64>) -> Result<Vec<u8>, LayerError> {
        self.layer.read(key, start, end).await
    }

    async fn write(&self, bytes: Vec<u8>) -> Result<Vec<u8>, LayerError> {
        self.layer.write(bytes).await
    }

    async fn write_stream(self: Arc<Self>) -> Result<Option<Box<dyn BlobStream>>, LayerError> {
        Arc::clone(&self.layer).write_stream().await
    }

    async fn read_stream(
        &self,
        key: &[u8],
        range: (u64, Option<u64>),
        size: Option<u64>,
    ) -> Result<Option<BlobRead>, LayerError> {
        self.layer.read_stream(key, range, size).await
    }
}
