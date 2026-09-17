//! The bucket as a provider of the effects the access layer performs.
//!
//! [`dialog_remote_ucan::Access`] decodes and verifies an invocation and
//! hands the operation it authorizes here as a capability. Every object
//! lives at the key the permit route would have addressed it by, so a
//! block written through one path is read through the other: blocks at
//! `{subject}/{catalog}/{digest}`, cells at `{subject}/{space}/{cell}`.
//! A cell's version is the object's `ETag`, unquoted, which is what a
//! client reads off the permit route's answer and echoes back as a
//! precondition. Blobs live at `{subject}/blob/{digest}`, read as the
//! bucket streams them and written whole once their bytes have been
//! checked against the declared digest.

use base58::ToBase58;
use dialog_capability::{Capability, Policy, Provider};
use dialog_common::Blake3Hash;
use dialog_effects::archive::prelude::{GetExt, PutExt};
use dialog_effects::archive::{self, ArchiveError};
use dialog_effects::blob::prelude::{BlobImportExt as _, BlobReadExt as _};
use dialog_effects::blob::{self, BlobError, BlobReader, BlobSink, BlobSource, BlobWriter};
use dialog_effects::memory::prelude::{PublishExt, RetractExt};
use dialog_effects::memory::{self, Cell, Edition, MemoryError, Space, Version};
use futures_util::StreamExt as _;
use sha2_0_10::{Digest, Sha256};
use worker::js_sys::Uint8Array;

use worker::wasm_bindgen::JsValue;
use worker::{Bucket, ByteStream, Range};

use crate::cached::{blob_key, block_key};
use crate::handlers::object::store;
use crate::permit::{Claims, Method, Precondition};

/// The bucket behind the access service, as a provider.
#[derive(Clone)]
pub struct Objects {
    bucket: Bucket,
}

impl Objects {
    /// The provider over `bucket`.
    pub fn new(bucket: Bucket) -> Self {
        Self { bucket }
    }
}

fn cell_key<Fx>(capability: &Capability<Fx>) -> String
where
    Fx: Policy<Of = Cell>,
{
    format!(
        "{}/{}/{}",
        capability.subject(),
        Space::of(capability).space,
        Cell::of(capability).cell
    )
}

fn claims(method: Method, key: String, body: Option<&[u8]>, precondition: Precondition) -> Claims {
    Claims {
        method,
        key,
        expires: 0,
        sha256: body.map(|body| Sha256::digest(body).to_vec()),
        precondition,
    }
}

/// A version as the precondition an object store compares: the `ETag`
/// the client was given, which the store quotes on the wire.
fn version_text(version: &Version) -> String {
    String::from_utf8_lossy(version.as_bytes()).into_owned()
}

/// The `ETag` the store answered with, as the version a client holds.
fn version_of(etag: &str) -> Version {
    Version::from(etag.trim_matches('"'))
}

async fn read(bucket: &Bucket, key: &str) -> Result<Option<(Vec<u8>, String)>, String> {
    let object = bucket
        .get(key)
        .execute()
        .await
        .map_err(|error| format!("get {key}: {error}"))?;
    let Some(object) = object else {
        return Ok(None);
    };
    let etag = object.http_etag();
    let bytes = object
        .body()
        .ok_or_else(|| format!("get {key}: the object came without a body"))?
        .bytes()
        .await
        .map_err(|error| format!("get {key}: {error}"))?;
    Ok(Some((bytes, etag)))
}

#[async_trait::async_trait(?Send)]
impl Provider<archive::Get> for Objects {
    async fn execute(
        &self,
        capability: Capability<archive::Get>,
    ) -> Result<Option<Vec<u8>>, ArchiveError> {
        let key = block_key(&capability, capability.digest());
        read(&self.bucket, &key)
            .await
            .map(|found| found.map(|(bytes, _)| bytes))
            .map_err(ArchiveError::Storage)
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<archive::Put> for Objects {
    async fn execute(&self, capability: Capability<archive::Put>) -> Result<(), ArchiveError> {
        let content = capability.content();
        let key = block_key(&capability, &Blake3Hash::hash(content));
        let claims = claims(Method::Put, key, Some(content), Precondition::None);
        let value: JsValue = Uint8Array::from(content).into();
        store::put(&self.bucket, &claims, value)
            .await
            .map(|_| ())
            .map_err(|error| ArchiveError::Storage(format!("put {}: {error}", claims.key)))
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<memory::Resolve> for Objects {
    async fn execute(
        &self,
        capability: Capability<memory::Resolve>,
    ) -> Result<Option<Edition<Vec<u8>>>, MemoryError> {
        let key = cell_key(&capability);
        read(&self.bucket, &key)
            .await
            .map(|found| {
                found.map(|(content, etag)| Edition {
                    content,
                    version: version_of(&etag),
                })
            })
            .map_err(MemoryError::Storage)
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<memory::Publish> for Objects {
    async fn execute(
        &self,
        capability: Capability<memory::Publish>,
    ) -> Result<Version, MemoryError> {
        let key = cell_key(&capability);
        let precondition = match capability.when() {
            Some(version) => Precondition::IfMatch(version_text(version)),
            None => Precondition::IfNoneMatch,
        };
        let content = capability.content();
        let claims = claims(Method::Put, key, Some(content), precondition);
        let value: JsValue = Uint8Array::from(content).into();
        match store::put(&self.bucket, &claims, value).await {
            Ok(Some(etag)) => Ok(version_of(&etag)),
            Ok(None) => Err(MemoryError::VersionMismatch {
                expected: capability.when().cloned(),
                actual: None,
            }),
            Err(error) => Err(MemoryError::Storage(format!("put {}: {error}", claims.key))),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<memory::Retract> for Objects {
    async fn execute(&self, capability: Capability<memory::Retract>) -> Result<(), MemoryError> {
        let key = cell_key(&capability);
        let claims = claims(
            Method::Delete,
            key,
            None,
            Precondition::IfMatch(version_text(capability.when())),
        );
        match store::delete(&self.bucket, &claims).await {
            Ok(true) => Ok(()),
            Ok(false) => Err(MemoryError::VersionMismatch {
                expected: Some(capability.when().clone()),
                actual: None,
            }),
            Err(error) => Err(MemoryError::Storage(format!(
                "delete {}: {error}",
                claims.key
            ))),
        }
    }
}

/// The bucket's stream of an object's bytes, as the source a blob read
/// answers with.
pub(crate) struct Streamed {
    stream: ByteStream,
}

impl Streamed {
    /// The source over `stream`.
    pub(crate) fn new(stream: ByteStream) -> Self {
        Self { stream }
    }
}

#[async_trait::async_trait(?Send)]
impl BlobSource for Streamed {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        self.stream
            .next()
            .await
            .transpose()
            .map_err(|error| BlobError::Storage(error.to_string()))
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<blob::Read> for Objects {
    async fn execute(&self, capability: Capability<blob::Read>) -> Result<BlobReader, BlobError> {
        let digest = capability.digest().clone();
        let key = blob_key(&capability, &digest);
        let mut request = self.bucket.get(&key);
        if let Some(range) = capability.range() {
            request = request.range(match range.length {
                Some(length) => Range::OffsetWithLength {
                    offset: range.offset,
                    length,
                },
                None => Range::OffsetToEnd {
                    offset: range.offset,
                },
            });
        }
        let object = request
            .execute()
            .await
            .map_err(|error| BlobError::Storage(format!("get {key}: {error}")))?
            .ok_or_else(|| BlobError::NotFound(digest.as_bytes().to_base58()))?;
        let stream = object
            .body()
            .ok_or_else(|| {
                BlobError::Storage(format!("get {key}: the object came without a body"))
            })?
            .stream()
            .map_err(|error| BlobError::Storage(format!("get {key}: {error}")))?;
        Ok(Box::new(Streamed { stream }))
    }
}

/// Gathers an import's bytes and stores them under the declared digest
/// once they have been checked against it.
struct Importing {
    bucket: Bucket,
    key: String,
    expected: Blake3Hash,
    buffer: Vec<u8>,
}

#[async_trait::async_trait(?Send)]
impl BlobSink for Importing {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), BlobError> {
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> Result<Blake3Hash, BlobError> {
        let Importing {
            bucket,
            key,
            expected,
            buffer,
        } = *self;
        let hash = Blake3Hash::hash(&buffer);
        if hash != expected {
            return Err(BlobError::DigestMismatch {
                expected: expected.as_bytes().to_base58(),
                actual: hash.as_bytes().to_base58(),
            });
        }
        let claims = claims(Method::Put, key, Some(&buffer), Precondition::None);
        let value: JsValue = Uint8Array::from(buffer.as_slice()).into();
        store::put(&bucket, &claims, value)
            .await
            .map(|_| hash)
            .map_err(|error| BlobError::Storage(format!("put {}: {error}", claims.key)))
    }
}

#[async_trait::async_trait(?Send)]
impl Provider<blob::Import> for Objects {
    async fn execute(&self, capability: Capability<blob::Import>) -> Result<BlobWriter, BlobError> {
        let expected = capability.digest().clone();
        Ok(Box::new(Importing {
            bucket: self.bucket.clone(),
            key: blob_key(&capability, &expected),
            expected,
            buffer: Vec::new(),
        }))
    }
}
