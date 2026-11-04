//! The guest side, in a guest kernel built for the world: a [`Layer`] over a
//! handle of `storage.layer`. See PDD007, "Storage".
//!
//! Each layer reads a block through the cache of blocks of the guest kernel
//! (see `cache`), so a read of the same block again makes no call of the world.
//! A layer of a handle sends the gets of one catalog that wait at once as
//! one call: each get waits for one turn of the tasks of the guest kernel, and
//! the gets that came in that turn go as one `get-many`. See PDD007, "The
//! Cost of a Read".

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use crate::world::sapling::kernel::storage;
use crate::world::sapling::kernel::types::{
    ByteRange, Encoding as WireEncoding, Error as WorldError,
};
use crate::world::wit_stream;
use async_trait::async_trait;
use dialog_varsig::Did;
use wit_bindgen::StreamResult;

use crate::cache::{Blocks, LONGEST, MOST_BYTES};
use crate::layer::{Edition, Layer, LayerError};
use crate::signal::Signal;

/// The most keys of one `get-many`, as the world says.
const MOST_KEYS: usize = 256;

/// The blocks that the guest kernel read or wrote. A static, not a thread
/// local: a thread local of `wasm32-wasip3` does not last from one task of the
/// component model to the next.
static BLOCKS: std::sync::Mutex<Option<Blocks>> = std::sync::Mutex::new(None);

/// Runs `then` with the cache of blocks.
fn with_blocks<T>(then: impl FnOnce(&mut Blocks) -> T) -> T {
    let mut blocks = BLOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    then(blocks.get_or_insert_with(|| Blocks::new(MOST_BYTES)))
}

/// The number of the next layer, for its part of the cache of blocks: never
/// the number of a layer before, as the number of a handle can be.
fn next_layer() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The block of `key` in `catalog` of the layer `layer`, if the cache keeps
/// it.
fn cached(layer: u64, catalog: &str, key: &[u8]) -> Option<Vec<u8>> {
    with_blocks(|blocks| blocks.get(layer, catalog, key))
}

/// Keeps `block` of `key` in `catalog` of the layer `layer` in the cache.
fn keep(layer: u64, catalog: &str, key: &[u8], block: &[u8]) {
    with_blocks(|blocks| blocks.put(layer, catalog, key, block));
}

/// Forgets the blocks of the layer `layer`.
fn forget(layer: u64) {
    with_blocks(|blocks| blocks.forget(layer));
}

/// The error of the world, as a [`LayerError`].
pub(crate) fn layer_error(error: WorldError) -> LayerError {
    match error {
        WorldError::Denied(reason) => LayerError::Denied(reason),
        WorldError::Conflict => LayerError::Conflict,
        WorldError::Expired => LayerError::Expired,
        WorldError::Invalid(reason) => LayerError::Invalid(reason),
        WorldError::Unavailable(reason) => LayerError::Unavailable(reason),
        WorldError::Unreachable(reason) => LayerError::Unreachable(reason),
        WorldError::BrokeOff(reason) => LayerError::BrokeOff(reason),
        WorldError::Full(reason) => LayerError::Full(reason),
    }
}

/// A layer of the world, by its handle.
pub struct HandleLayer {
    layer: storage::Layer,
    subject: Did,
    /// The number of the layer in the cache of blocks.
    id: u64,
    /// The batch of each catalog that takes gets still.
    open: Open,
}

/// The batch of each catalog of a layer that takes gets still.
type Open = Rc<RefCell<Vec<(String, Rc<Batch>)>>>;

/// The gets of one catalog of a layer that go as one call: their keys, and
/// then the answer.
struct Batch {
    keys: RefCell<Vec<Vec<u8>>>,
    answer: RefCell<Answer>,
    /// Moves once the batch has an answer, or its sender went away.
    done: Signal,
}

/// The answer to a [`Batch`].
enum Answer {
    /// The batch waits for its call.
    Waiting,
    /// The block of each key, in order.
    Done(Result<Vec<Option<Vec<u8>>>, LayerError>),
    /// The get that sends the batch was dropped before the answer came: each
    /// other get of the batch reads its block by itself.
    Abandoned,
}

/// Leaves a batch [`Answer::Abandoned`] when the get that sends it is
/// dropped before it answers.
struct Sending {
    batch: Rc<Batch>,
    open: Open,
    answered: bool,
}

impl Drop for Sending {
    fn drop(&mut self) {
        if self.answered {
            return;
        }
        self.open
            .borrow_mut()
            .retain(|(_, batch)| !Rc::ptr_eq(batch, &self.batch));
        *self.batch.answer.borrow_mut() = Answer::Abandoned;
        self.batch.done.notify();
    }
}

/// Waits for one turn of the tasks of the guest kernel: the other gets that a
/// task starts at once join the batch in that turn.
struct Turn(bool);

impl Future for Turn {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl HandleLayer {
    /// The layer of the handle `layer`.
    ///
    /// # Errors
    ///
    /// If the subject of the layer is not a DID.
    pub fn new(layer: storage::Layer) -> Result<Self, LayerError> {
        let subject = layer
            .subject()
            .parse()
            .map_err(|_| LayerError::Invalid("the subject of the layer is not a DID".into()))?;
        Ok(Self {
            layer,
            subject,
            id: next_layer(),
            open: Rc::default(),
        })
    }

    /// The batch of `catalog` that takes gets, with `key` in it: the batch,
    /// the place of the key in it, and whether this get sends it.
    fn join(&self, catalog: &str, key: &[u8]) -> (Rc<Batch>, usize, bool) {
        let mut open = self.open.borrow_mut();
        let (batch, sends) = match open.iter().find(|(of, _)| of == catalog) {
            Some((_, batch)) => (Rc::clone(batch), false),
            None => {
                let batch = Rc::new(Batch {
                    keys: RefCell::default(),
                    answer: RefCell::new(Answer::Waiting),
                    done: Signal::new(),
                });
                open.push((catalog.to_owned(), Rc::clone(&batch)));
                (batch, true)
            }
        };
        let mut keys = batch.keys.borrow_mut();
        let at = match keys.iter().position(|kept| kept == key) {
            Some(at) => at,
            None => {
                keys.push(key.to_vec());
                keys.len() - 1
            }
        };
        drop(keys);
        (batch, at, sends)
    }

    /// The blocks of `keys` in `catalog`, in order: with one get for one
    /// key, and else with as many calls of `get-many` as they need. Each
    /// block goes into the cache.
    async fn fetch(
        &self,
        catalog: &str,
        keys: &[Vec<u8>],
    ) -> Result<Vec<Option<Vec<u8>>>, LayerError> {
        if let [key] = keys {
            let block = self
                .layer
                .get(catalog.to_owned(), key.clone())
                .await
                .map_err(layer_error)?;
            if let Some(block) = &block {
                keep(self.id, catalog, key, block);
            }
            return Ok(vec![block]);
        }
        let mut blocks: Vec<Option<Vec<u8>>> = Vec::with_capacity(keys.len());
        while blocks.len() < keys.len() {
            let asked: Vec<Vec<u8>> = keys[blocks.len()..]
                .iter()
                .take(MOST_KEYS)
                .cloned()
                .collect();
            let count = asked.len();
            let answered = self
                .layer
                .get_many(catalog.to_owned(), asked)
                .await
                .map_err(layer_error)?;
            if answered.is_empty() || answered.len() > count {
                return Err(LayerError::Unavailable(format!(
                    "the daemon answered a get-many of {count} keys with {} blocks",
                    answered.len()
                )));
            }
            for block in answered {
                if let Some(block) = &block {
                    keep(self.id, catalog, &keys[blocks.len()], block);
                }
                blocks.push(block);
            }
        }
        Ok(blocks)
    }
}

impl Drop for HandleLayer {
    fn drop(&mut self) {
        forget(self.id);
    }
}

#[async_trait(?Send)]
impl Layer for HandleLayer {
    fn subject(&self) -> Did {
        self.subject.clone()
    }

    async fn get(&self, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, LayerError> {
        if let Some(block) = cached(self.id, catalog, key) {
            return Ok(Some(block));
        }
        let (batch, at, sends) = self.join(catalog, key);
        if sends {
            let mut sending = Sending {
                batch: Rc::clone(&batch),
                open: Rc::clone(&self.open),
                answered: false,
            };
            Turn(false).await;
            self.open
                .borrow_mut()
                .retain(|(_, open)| !Rc::ptr_eq(open, &batch));
            let keys = batch.keys.borrow().clone();
            let answer = self.fetch(catalog, &keys).await;
            *batch.answer.borrow_mut() = Answer::Done(answer);
            sending.answered = true;
            batch.done.notify();
        } else if matches!(*batch.answer.borrow(), Answer::Waiting) {
            batch.done.changed(0).await;
        }
        // The daemon fails a whole `get-many` for the error of one key. So
        // each get of a batch of several keys that failed asks again alone,
        // as for a batch that its sender abandoned, and gets the error of its
        // own key only.
        let several = batch.keys.borrow().len() > 1;
        let answered = match &*batch.answer.borrow() {
            Answer::Done(Ok(blocks)) => Some(Ok(blocks.get(at).cloned().flatten())),
            Answer::Done(Err(_)) if several => None,
            Answer::Done(Err(error)) => Some(Err(error.clone())),
            Answer::Abandoned | Answer::Waiting => None,
        };
        match answered {
            Some(answered) => answered,
            None => {
                let mut blocks = self.fetch(catalog, &[key.to_vec()]).await?;
                Ok(blocks.pop().flatten())
            }
        }
    }

    async fn put(&self, catalog: &str, key: &[u8], bytes: Vec<u8>) -> Result<(), LayerError> {
        // The daemon takes a block under its own hash alone, so the cache
        // keeps a block that a put wrote, if it keeps a block that long.
        let block = (bytes.len() <= LONGEST).then(|| bytes.clone());
        self.layer
            .put(catalog.to_owned(), key.to_vec(), bytes)
            .await
            .map_err(layer_error)?;
        if let Some(block) = block {
            keep(self.id, catalog, key, &block);
        }
        Ok(())
    }

    async fn resolve(&self, namespace: &str, cell: &str) -> Result<Option<Edition>, LayerError> {
        let edition = self
            .layer
            .resolve(namespace.to_owned(), cell.to_owned())
            .await
            .map_err(layer_error)?;
        Ok(edition.map(|edition| Edition {
            content: edition.content,
            version: edition.version,
        }))
    }

    async fn publish(
        &self,
        namespace: &str,
        cell: &str,
        content: Vec<u8>,
        when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, LayerError> {
        self.layer
            .publish(namespace.to_owned(), cell.to_owned(), content, when)
            .await
            .map_err(layer_error)
    }

    async fn retract(&self, namespace: &str, cell: &str, when: Vec<u8>) -> Result<(), LayerError> {
        self.layer
            .retract(namespace.to_owned(), cell.to_owned(), when)
            .await
            .map_err(layer_error)
    }

    async fn list(&self, namespace: &str) -> Result<Vec<String>, LayerError> {
        self.layer
            .list(namespace.to_owned())
            .await
            .map_err(layer_error)
    }

    async fn size(&self, key: &[u8]) -> Result<Option<u64>, LayerError> {
        self.layer.size(key.to_vec()).await.map_err(layer_error)
    }

    async fn read(&self, key: &[u8], start: u64, end: Option<u64>) -> Result<Vec<u8>, LayerError> {
        let read = self
            .layer
            .read(
                key.to_vec(),
                ByteRange { start, end },
                None,
                WireEncoding::Raw,
                false,
            )
            .await
            .map_err(layer_error)?;
        Ok(read_all(read.body).await)
    }

    async fn write(&self, bytes: Vec<u8>) -> Result<Vec<u8>, LayerError> {
        let (mut writer, reader) = wit_stream::new();
        wit_bindgen::spawn_local(async move {
            let _ = writer.write_all(bytes).await;
        });
        self.layer.write(reader).await.map_err(layer_error)
    }

    async fn write_stream(
        self: std::sync::Arc<Self>,
    ) -> Result<Option<Box<dyn crate::layer::BlobStream>>, LayerError> {
        let (writer, reader) = wit_stream::new();
        let call: Writing = Box::pin(async move { self.layer.write(reader).await });
        Ok(Some(Box::new(HandleStream {
            call: Some(call),
            writer,
        })))
    }

    async fn read_stream(
        &self,
        key: &[u8],
        (start, end): (u64, Option<u64>),
        size: Option<u64>,
    ) -> Result<Option<crate::layer::BlobRead>, LayerError> {
        let read = self
            .layer
            .read(
                key.to_vec(),
                ByteRange { start, end },
                size,
                WireEncoding::Raw,
                false,
            )
            .await
            .map_err(layer_error)?;
        Ok(Some(crate::layer::BlobRead {
            chunks: Box::new(HandleChunks(read.body)),
        }))
    }
}

/// The call `storage.layer.write` of a blob that streams, on its way.
type Writing = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<Vec<u8>, crate::world::sapling::kernel::types::Error>,
            >,
    >,
>;

/// A blob that streams to a handle: the stream that the call `write` reads,
/// and the call. Each chunk is written while the call runs beside it, so the
/// daemon takes the blob a chunk at a time.
struct HandleStream {
    /// Declared first, so that a stream dropped before its end drops the
    /// call, which cancels the write, before the end of its bytes would end
    /// it whole. `None` once the call has ended, before the stream.
    call: Option<Writing>,
    writer: wit_bindgen::StreamWriter<u8>,
}

#[async_trait(?Send)]
impl crate::layer::BlobStream for HandleStream {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), LayerError> {
        use futures_util::future::Either;
        let Some(call) = self.call.as_mut() else {
            return Err(ended());
        };
        let written = std::pin::pin!(self.writer.write_all(bytes.to_vec()));
        // The call that ends first, as one that the daemon refuses, gives
        // the error: the stream that it no longer reads only says it ended.
        let answered = match futures_util::future::select(written, call.as_mut()).await {
            Either::Left((rest, _)) if rest.is_empty() => return Ok(()),
            Either::Left((_, call)) => call.await,
            Either::Right((ended, _)) => ended,
        };
        self.call = None;
        Err(match answered {
            Err(error) => layer_error(error),
            Ok(_) => ended(),
        })
    }

    async fn finish(self: Box<Self>) -> Result<Vec<u8>, LayerError> {
        let HandleStream { writer, call } = *self;
        let call = call.ok_or_else(ended)?;
        // The end of the stream ends the write.
        drop(writer);
        call.await.map_err(layer_error)
    }
}

/// The error of a write of a blob whose call ended before its last chunk.
fn ended() -> LayerError {
    LayerError::Unavailable("the write of the blob ended before its last chunk".into())
}

/// A blob read from a handle a chunk at a time: the stream of `read`.
struct HandleChunks(wit_bindgen::StreamReader<u8>);

#[async_trait(?Send)]
impl crate::layer::BlobChunks for HandleChunks {
    async fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            let (status, chunk) = self.0.read(Vec::with_capacity(CHUNK)).await;
            match status {
                StreamResult::Complete(_) if chunk.is_empty() => continue,
                StreamResult::Complete(_) => return Some(chunk),
                StreamResult::Dropped | StreamResult::Cancelled => return None,
            }
        }
    }
}

/// The most bytes that one read of a stream of a blob takes, as the daemon
/// sends a blob a chunk at a time.
const CHUNK: usize = 1024 * 1024;

/// The bytes of `stream`, read to its end.
pub(crate) async fn read_all(mut stream: wit_bindgen::StreamReader<u8>) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = Vec::with_capacity(64 * 1024);
    loop {
        let (status, buffer) = stream.read(chunk).await;
        chunk = buffer;
        match status {
            StreamResult::Complete(_) => {
                bytes.extend_from_slice(&chunk);
                chunk.clear();
            }
            StreamResult::Dropped | StreamResult::Cancelled => break,
        }
    }
    bytes
}
