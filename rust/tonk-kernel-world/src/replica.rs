//! The replicas of the peers a worker reaches through its embedder
//! (`network`), as Dialog DB's UCAN site reaches them.
//!
//! A worker on WASI has no network of its own. Its UCAN site proves each
//! operation as any other does, and hands it to [`KernelRelay`], which asks
//! the embedder to reach the peer at the operation's address, opens the
//! replica of its subject there under the invocation's proofs, and performs
//! the operation on that replica as it would on a layer of its own stack.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use async_trait::async_trait;
use dialog_capability::{Effect, ForkInvocation};
use dialog_effects::archive::{ArchiveError, Get, Put};
use dialog_effects::blob::prelude::BlobReadExt as _;
use dialog_effects::blob::{BlobError, BlobReader, BlobSource, BlobWriter, Import, Read};
use dialog_effects::memory::{
    Edition as MemoryEdition, MemoryError, Publish, Resolve, Retract, Version,
};
use dialog_remote_ucan::{Relay, UcanAuthorization, UcanSite};
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;

use crate::guest::{layer_error, read_all};
use crate::layer::{Edition, Layer, LayerError, share};
use crate::store::{Store, Witness, archive_error, blob_error, memory_error};
use crate::world::sapling::kernel::network;
use crate::world::sapling::kernel::types::{ByteRange, Encoding as WireEncoding};
use crate::world::wit_stream;

/// A replica of a subject on a peer, by its handle, as a layer.
pub struct HandleReplica {
    replica: network::Replica,
    subject: Did,
}

impl HandleReplica {
    /// The replica of the handle `replica`.
    ///
    /// # Errors
    ///
    /// If the subject of the replica is not a DID.
    pub fn new(replica: network::Replica) -> Result<Self, LayerError> {
        let subject = replica
            .subject()
            .parse()
            .map_err(|_| LayerError::Invalid("the subject of the replica is not a DID".into()))?;
        Ok(Self { replica, subject })
    }
}

#[async_trait(?Send)]
impl Layer for HandleReplica {
    fn subject(&self) -> Did {
        self.subject.clone()
    }

    async fn get(&self, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, LayerError> {
        self.replica
            .get(catalog.to_owned(), key.to_vec())
            .await
            .map_err(layer_error)
    }

    async fn put(&self, catalog: &str, key: &[u8], bytes: Vec<u8>) -> Result<(), LayerError> {
        self.replica
            .put(catalog.to_owned(), key.to_vec(), bytes)
            .await
            .map_err(layer_error)
    }

    async fn resolve(&self, namespace: &str, cell: &str) -> Result<Option<Edition>, LayerError> {
        let edition = self
            .replica
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
        self.replica
            .publish(namespace.to_owned(), cell.to_owned(), content, when)
            .await
            .map_err(layer_error)
    }

    async fn retract(&self, namespace: &str, cell: &str, when: Vec<u8>) -> Result<(), LayerError> {
        self.replica
            .retract(namespace.to_owned(), cell.to_owned(), when)
            .await
            .map_err(layer_error)
    }

    async fn list(&self, namespace: &str) -> Result<Vec<String>, LayerError> {
        self.replica
            .list(namespace.to_owned())
            .await
            .map_err(layer_error)
    }

    async fn size(&self, key: &[u8]) -> Result<Option<u64>, LayerError> {
        self.replica.size(key.to_vec()).await.map_err(layer_error)
    }

    async fn read(&self, key: &[u8], start: u64, end: Option<u64>) -> Result<Vec<u8>, LayerError> {
        let read = self
            .replica
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
        self.replica.write(reader).await.map_err(layer_error)
    }
}

/// What an embedder answers a read of a blob the replica does not hold.
const NO_SUCH_BLOB: &str = "no such blob";

/// A blob read whole, as one chunk.
struct Whole(Option<Vec<u8>>);

#[async_trait(?Send)]
impl BlobSource for Whole {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        Ok(self.0.take())
    }
}

/// Performs a UCAN site's operations on the replicas the embedder reaches,
/// keeping each replica it opened, by the address and the proofs it was
/// opened with.
#[derive(Default)]
pub struct KernelRelay {
    opened: RefCell<HashMap<(String, Vec<u8>), Store>>,
}

// SAFETY: a component on WASI has no threads, so the replicas the relay
// keeps are never reached from another thread.
unsafe impl Send for KernelRelay {}
// SAFETY: as above, nothing reaches the relay from another thread.
unsafe impl Sync for KernelRelay {}

impl Debug for KernelRelay {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelRelay")
            .field("opened", &self.opened.borrow().len())
            .finish()
    }
}

impl KernelRelay {
    /// A site whose operations the embedder performs.
    pub fn site() -> UcanSite {
        UcanSite::relayed(Arc::new(Self::default()))
    }

    /// The replica `invocation` is for, as a store: the replica of its
    /// subject on the peer at its address, opened under its proofs.
    async fn replica<Fx: Effect>(
        &self,
        invocation: &ForkInvocation<UcanSite, Fx>,
    ) -> Result<Store, LayerError>
    where
        Fx::Of: dialog_capability::Constraint,
    {
        let endpoint = invocation.address.endpoint().to_owned();
        let chain = proofs(&invocation.authorization)?;
        let key = (endpoint.clone(), chain.clone());
        if let Some(store) = self.opened.borrow().get(&key) {
            return Ok(store.clone());
        }
        let subject = invocation
            .authorization
            .invocation()
            .chain()
            .subject()
            .to_string();
        let peer = network::open(peer_of(&endpoint), vec![endpoint])
            .await
            .map_err(layer_error)?;
        let replica = peer.open(subject, chain).await.map_err(layer_error)?;
        let store = Store::new(share(HandleReplica::new(replica)?), Witness::default());
        self.opened.borrow_mut().insert(key, store.clone());
        Ok(store)
    }
}

/// The delegations an invocation is proven by, as a chain from its subject
/// to its invoker, encoded.
fn proofs(authorization: &UcanAuthorization) -> Result<Vec<u8>, LayerError> {
    let chain = authorization.invocation().chain();
    let delegations = chain
        .proofs()
        .iter()
        .map(|cid| {
            chain
                .delegation(cid)
                .map(|delegation| (**delegation).clone())
                .ok_or_else(|| LayerError::Invalid(format!("the invocation lacks its proof {cid}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    DelegationChain::try_from(delegations)
        .and_then(|chain| chain.to_bytes())
        .map_err(|error| LayerError::Invalid(format!("the invocation's proofs: {error}")))
}

/// The DID a peer at `endpoint` goes by: the `did:web` of its host.
fn peer_of(endpoint: &str) -> String {
    let host = endpoint
        .split("://")
        .nth(1)
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or_default();
    format!("did:web:{}", host.replace(':', "%3A"))
}

#[async_trait(?Send)]
impl Relay for KernelRelay {
    async fn get(
        &self,
        invocation: ForkInvocation<UcanSite, Get>,
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        let store = self.replica(&invocation).await.map_err(archive_error)?;
        invocation.capability.perform(&store).await
    }

    async fn put(&self, invocation: ForkInvocation<UcanSite, Put>) -> Result<(), ArchiveError> {
        let store = self.replica(&invocation).await.map_err(archive_error)?;
        invocation.capability.perform(&store).await
    }

    async fn resolve(
        &self,
        invocation: ForkInvocation<UcanSite, Resolve>,
    ) -> Result<Option<MemoryEdition<Vec<u8>>>, MemoryError> {
        let store = self
            .replica(&invocation)
            .await
            .map_err(|error| memory_error(error, None))?;
        invocation.capability.perform(&store).await
    }

    async fn publish(
        &self,
        invocation: ForkInvocation<UcanSite, Publish>,
    ) -> Result<Version, MemoryError> {
        let store = self
            .replica(&invocation)
            .await
            .map_err(|error| memory_error(error, None))?;
        invocation.capability.perform(&store).await
    }

    async fn retract(
        &self,
        invocation: ForkInvocation<UcanSite, Retract>,
    ) -> Result<(), MemoryError> {
        let store = self
            .replica(&invocation)
            .await
            .map_err(|error| memory_error(error, None))?;
        invocation.capability.perform(&store).await
    }

    /// A replica reads a blob whole or in part, without asking its size
    /// first, as a layer is asked: a service answers that only by sending
    /// the blob.
    async fn read(
        &self,
        invocation: ForkInvocation<UcanSite, Read>,
    ) -> Result<BlobReader, BlobError> {
        let store = self.replica(&invocation).await.map_err(blob_error)?;
        let digest = invocation.capability.digest();
        let (start, end) = match invocation.capability.range() {
            None => (0, None),
            Some(range) => (
                range.offset,
                range
                    .length
                    .map(|length| range.offset.saturating_add(length)),
            ),
        };
        match store.layer().read(digest.as_bytes(), start, end).await {
            Ok(bytes) => Ok(Box::new(Whole(Some(bytes)))),
            Err(LayerError::Invalid(reason)) if reason == NO_SUCH_BLOB => {
                Err(BlobError::NotFound(digest.to_string()))
            }
            Err(error) => Err(blob_error(error)),
        }
    }

    async fn import(
        &self,
        invocation: ForkInvocation<UcanSite, Import>,
    ) -> Result<BlobWriter, BlobError> {
        let store = self.replica(&invocation).await.map_err(blob_error)?;
        invocation.capability.perform(&store).await
    }
}
