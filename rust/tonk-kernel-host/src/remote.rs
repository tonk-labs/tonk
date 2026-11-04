//! The replicas a guest kernel reaches through this host (`network`): a
//! subject on an access service, acted on under the chain the guest kernel
//! opened it with.
//!
//! The guest kernel proves each operation itself, and hands over only what
//! the operation is and the chain that grants it. This host signs the
//! invocation with the key the chain was delegated to, which it holds, and
//! performs it at the service the way Dialog DB's UCAN site performs it for
//! any other worker.

use dialog_capability::access::{
    Authorization as _, FromCapability, Proof as _, Protocol, TimeRange,
};
use dialog_capability::{
    Ability, Capability, Constraint, Effect, ForkInvocation, Provider, Subject,
};
use dialog_common::Blake3Hash;
use dialog_common::time::{self, UNIX_EPOCH};
use dialog_credentials::Signer;
use dialog_effects::archive::prelude::{
    ArchiveExt as _, CatalogExt as _, GetBlockExt as _, PutBlockExt as _,
};
use dialog_effects::blob::prelude::{ArchiveBlobExt as _, WriteBlobExt as _};
use dialog_effects::blob::{self, BlobError};
use dialog_effects::memory::prelude::{
    CellExt as _, MemoryExt as _, PublishCellExt as _, ResolveCellExt as _, RetractCellExt as _,
    SpaceExt as _,
};
use dialog_effects::memory::{MemoryError, Version};
use dialog_effects::{MethodExt as _, UseExt as _, archive};
use dialog_remote_ucan::{Ucan, UcanAddress, UcanAuthorization, UcanSite};
use dialog_ucan::UcanProof;
use dialog_ucan_core::DelegationChain;
use dialog_varsig::Did;

use crate::bindings::sapling::kernel::storage::Edition;
use crate::bindings::sapling::kernel::types::Error;

/// A replica of a subject on an access service.
#[derive(Clone)]
pub struct Replica {
    subject: Did,
    chain: DelegationChain,
    signer: Signer,
    address: UcanAddress,
    site: UcanSite,
}

impl Replica {
    /// The replica of `subject` at `endpoint`, acted on under `chain` by
    /// `signer`, the key it was delegated to.
    pub fn new(subject: Did, chain: DelegationChain, signer: Signer, endpoint: &str) -> Self {
        Self {
            subject,
            chain,
            signer,
            address: UcanAddress::new(endpoint),
            site: UcanSite::default(),
        }
    }

    /// The subject the replica is of.
    pub fn subject(&self) -> &Did {
        &self.subject
    }

    fn of(&self) -> Subject {
        Subject::from(self.subject.clone())
    }

    /// Perform `capability` at the service: invoked now, under the chain,
    /// signed by the key it was delegated to.
    async fn perform<Fx>(&self, capability: Capability<Fx>) -> Result<Fx::Output, Error>
    where
        Fx: Effect + Clone + 'static,
        Fx::Of: Constraint,
        Capability<Fx>: Ability,
        UcanSite: Provider<ForkInvocation<UcanSite, Fx>>,
    {
        let scope = <Ucan as Protocol>::Access::from_capability(&capability);
        let mut proof = UcanProof::from_chain(&self.chain, scope);
        let at = now();
        proof.set_duration(TimeRange {
            not_before: Some(at),
            expiration: Some(at),
        });
        let invocation = proof
            .claim(self.signer.clone())
            .map_err(denied)?
            .invoke()
            .await
            .map_err(denied)?;
        let authorization = UcanAuthorization::from(invocation);
        Ok(self
            .site
            .execute(ForkInvocation::new(
                capability,
                self.address.clone(),
                authorization,
            ))
            .await)
    }

    /// The block of `catalog` under `key`.
    pub async fn get(&self, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let get = self
            .of()
            .reader()
            .archive()
            .catalog(catalog)
            .get(hash(key)?);
        self.perform(get).await?.map_err(archived)
    }

    /// Keep `bytes` as the block of `catalog` under `key`, their hash.
    pub async fn put(&self, catalog: &str, key: &[u8], bytes: Vec<u8>) -> Result<(), Error> {
        if blake3::hash(&bytes).as_bytes() != key {
            return Err(Error::Invalid(
                "a block's key is the hash of its bytes".into(),
            ));
        }
        let put = self.of().writer().archive().catalog(catalog).put(bytes);
        self.perform(put).await?.map_err(archived)
    }

    /// The edition of the cell `cell` of `namespace`, if it holds one.
    pub async fn resolve(&self, namespace: &str, cell: &str) -> Result<Option<Edition>, Error> {
        let resolve = self
            .of()
            .reader()
            .memory()
            .space(namespace)
            .cell(cell)
            .resolve();
        let edition = self.perform(resolve).await?.map_err(remembered)?;
        Ok(edition.map(|edition| Edition {
            content: edition.content,
            version: edition.version.as_bytes().to_vec(),
        }))
    }

    /// Publish `content` to the cell, if it is still at `when`. Answers its
    /// new version.
    pub async fn publish(
        &self,
        namespace: &str,
        cell: &str,
        content: Vec<u8>,
        when: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        let publish = self
            .of()
            .writer()
            .memory()
            .space(namespace)
            .cell(cell)
            .publish(content, when.map(Version::from));
        let version = self.perform(publish).await?.map_err(remembered)?;
        Ok(version.as_bytes().to_vec())
    }

    /// Empty the cell, if it is still at `when`.
    pub async fn retract(&self, namespace: &str, cell: &str, when: Vec<u8>) -> Result<(), Error> {
        let retract = self
            .of()
            .user()
            .delete()
            .memory()
            .space(namespace)
            .cell(cell)
            .retract(Version::from(when));
        self.perform(retract).await?.map_err(remembered)
    }

    /// The bytes of the blob under `key`, from `start` up to `end`, or to its
    /// end.
    pub async fn read(&self, key: &[u8], start: u64, end: Option<u64>) -> Result<Vec<u8>, Error> {
        let length = end.map(|end| end.saturating_sub(start));
        let read: Capability<blob::Read> = self
            .of()
            .reader()
            .archive()
            .blob()
            .invoke(blob::Read::range(hash(key)?, start, length));
        let mut reader = match self.perform(read).await? {
            Ok(reader) => reader,
            Err(BlobError::NotFound(_)) => return Err(Error::Invalid("no such blob".into())),
            Err(error) => return Err(blobbed(error)),
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = reader.next().await.map_err(blobbed)? {
            bytes.extend(chunk);
        }
        Ok(bytes)
    }

    /// Keep `bytes` as a blob. Answers its hash.
    pub async fn write(&self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let digest = Blake3Hash::hash(bytes);
        let import = self
            .of()
            .writer()
            .archive()
            .blob()
            .import(digest, bytes.len() as u64);
        let mut sink = self.perform(import).await?.map_err(blobbed)?;
        sink.write_all(bytes).await.map_err(blobbed)?;
        let hash = sink.finish().await.map_err(blobbed)?;
        Ok(hash.as_bytes().to_vec())
    }
}

fn now() -> u64 {
    time::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn hash(key: &[u8]) -> Result<Blake3Hash, Error> {
    <[u8; 32]>::try_from(key)
        .map(Blake3Hash::from)
        .map_err(|_| Error::Invalid("a key is a 32-byte BLAKE3 hash".into()))
}

fn denied(error: dialog_capability::access::AuthorizeError) -> Error {
    Error::Denied(error.to_string())
}

fn archived(error: archive::ArchiveError) -> Error {
    Error::Unavailable(error.to_string())
}

fn remembered(error: MemoryError) -> Error {
    match error {
        MemoryError::VersionMismatch { .. } => Error::Conflict,
        other => Error::Unavailable(other.to_string()),
    }
}

fn blobbed(error: BlobError) -> Error {
    Error::Unavailable(error.to_string())
}
