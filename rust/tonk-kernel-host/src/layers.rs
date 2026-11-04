//! Where a host keeps the layers a guest kernel opens: each one a space of
//! Dialog DB, at the location the layer's name was made for
//! ([`naming::location`]). A layer is kept where Dialog DB keeps that
//! location, so what a worker running Dialog DB kept there before opens as
//! the layer it was: in a browser, the IndexedDB database of the location,
//! with its blobs in the origin's file system.

use dialog_capability::{Capability, Effect, Provider, Subject, did};
use dialog_common::Blake3Hash;
use dialog_effects::archive::prelude::{
    ArchiveExt as _, CatalogExt as _, GetBlockExt as _, PutBlockExt as _,
};
use dialog_effects::blob::prelude::{ArchiveBlobExt as _, ReadBlobExt as _, WriteBlobExt as _};
use dialog_effects::blob::{self, BlobError};
use dialog_effects::memory::prelude::{
    CellExt as _, ListSpaceExt as _, MemoryExt as _, PublishCellExt as _, ResolveCellExt as _,
    RetractCellExt as _, SpaceExt as _,
};
use dialog_effects::memory::{MemoryError, Version};
use dialog_effects::storage::{Directory, Location};
use dialog_effects::{MethodExt as _, UseExt as _, archive};
use dialog_storage::resource::Resource as _;
use tonk_kernel_world::naming;

use crate::bindings::sapling::kernel::storage::Edition;
use crate::bindings::sapling::kernel::types::Error;

/// The space of Dialog DB a layer is: IndexedDB, with blobs in the origin's
/// file system, in a browser; the file system everywhere else.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub type LayerSpace = dialog_storage::provider::storage::WebSpace;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub type LayerSpace = dialog_storage::provider::storage::NativeSpace;

/// Where layers are kept: where Dialog DB keeps each location, or, for a
/// host of its own such as a test's, all under one directory.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    under: Option<String>,
}

impl Layers {
    /// Layers kept under the directory `path`, each in a directory named for
    /// where Dialog DB would keep it.
    pub fn under(path: impl Into<String>) -> Self {
        Self {
            under: Some(path.into()),
        }
    }

    /// The location the layer `name` is kept at.
    pub fn location(&self, name: &str) -> Location {
        let location = naming::location(name);
        let Some(root) = &self.under else {
            return location;
        };
        let kept = match &location.directory {
            Directory::Profile => "profile".to_owned(),
            Directory::Current => "current".to_owned(),
            Directory::Temp => "temp".to_owned(),
            Directory::At(path) => format!("at/{}", path.trim_start_matches('/')),
        };
        Location::new(Directory::At(format!("{root}/{kept}")), location.name)
    }

    /// The subjects whose layers are kept: each a space's DID, which a
    /// space's layer is named by.
    pub async fn subjects(&self) -> Result<Vec<String>, Error> {
        let names = self.kept().await?;
        Ok(names
            .into_iter()
            .filter(|name| name.starts_with("did:") && !name.starts_with("did:local:"))
            .collect())
    }

    /// The name of each store kept: in a browser, each IndexedDB database of
    /// the origin.
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    async fn kept(&self) -> Result<Vec<String>, Error> {
        use js_sys::{Array, Function, Promise, Reflect};
        use wasm_bindgen::{JsCast as _, JsValue};

        let unavailable = |error: JsValue| Error::Unavailable(format!("{error:?}"));
        let factory = Reflect::get(&js_sys::global(), &"indexedDB".into()).map_err(unavailable)?;
        let databases: Function = Reflect::get(&factory, &"databases".into())
            .map_err(unavailable)?
            .dyn_into()
            .map_err(unavailable)?;
        let listed: Promise = databases
            .call0(&factory)
            .map_err(unavailable)?
            .dyn_into()
            .map_err(unavailable)?;
        let listed = wasm_bindgen_futures::JsFuture::from(listed)
            .await
            .map_err(unavailable)?;
        Ok(Array::from(&listed)
            .iter()
            .filter_map(|database| Reflect::get(&database, &"name".into()).ok()?.as_string())
            .collect())
    }

    /// The name of each store kept under the current directory, where a
    /// space's layer is.
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    async fn kept(&self) -> Result<Vec<String>, Error> {
        let Some(root) = &self.under else {
            return Ok(Vec::new());
        };
        let Ok(entries) = std::fs::read_dir(std::path::Path::new(root).join("current")) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect())
    }

    /// The space the layer `name` is kept in.
    pub async fn open(&self, name: &str) -> Result<LayerSpace, Error> {
        LayerSpace::open(&self.location(name))
            .await
            .map_err(|error| Error::Unavailable(format!("the layer {name} did not open: {error}")))
    }
}

/// What every effect on a layer's space is addressed to. A space serves only
/// its own location, whatever subject an effect names.
fn subject() -> Subject {
    Subject::from(did!("local:storage"))
}

fn hash(key: &[u8]) -> Result<Blake3Hash, Error> {
    <[u8; 32]>::try_from(key)
        .map(Blake3Hash::from)
        .map_err(|_| Error::Invalid("a key is a 32-byte BLAKE3 hash".into()))
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

/// Perform `effect` on `space`. In a browser, on the browser's own executor,
/// whose wake-ups are microtasks: an IndexedDB transaction stays open only
/// while what continues it runs in the task it started in, and the polyfill
/// resumes a host's futures in a later one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
async fn perform<E>(space: &LayerSpace, effect: Capability<E>) -> E::Output
where
    E: Effect + 'static,
    E::Output: 'static,
    LayerSpace: Provider<E>,
{
    let space = space.clone();
    let (done, answer) = futures::channel::oneshot::channel();
    wasm_bindgen_futures::spawn_local(async move {
        let _ = done.send(effect.perform(&space).await);
    });
    answer
        .await
        .expect("the browser's executor runs each task it is given to its end")
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
async fn perform<E>(space: &LayerSpace, effect: Capability<E>) -> E::Output
where
    E: Effect,
    LayerSpace: Provider<E>,
{
    effect.perform(space).await
}

/// The block of `catalog` under `key`.
pub async fn get(space: &LayerSpace, catalog: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    perform(
        space,
        subject()
            .reader()
            .archive()
            .catalog(catalog)
            .get(hash(key)?),
    )
    .await
    .map_err(archived)
}

/// Keep `bytes` as the block of `catalog` under `key`, which is their hash.
pub async fn put(
    space: &LayerSpace,
    catalog: &str,
    key: &[u8],
    bytes: Vec<u8>,
) -> Result<(), Error> {
    if blake3::hash(&bytes).as_bytes() != key {
        return Err(Error::Invalid(
            "a block's key is the hash of its bytes".into(),
        ));
    }
    perform(
        space,
        subject().writer().archive().catalog(catalog).put(bytes),
    )
    .await
    .map_err(archived)
}

/// The edition of the cell `cell` of `namespace`, if it holds one.
pub async fn resolve(
    space: &LayerSpace,
    namespace: &str,
    cell: &str,
) -> Result<Option<Edition>, Error> {
    let edition = perform(
        space,
        subject()
            .reader()
            .memory()
            .space(namespace)
            .cell(cell)
            .resolve(),
    )
    .await
    .map_err(remembered)?;
    Ok(edition.map(|edition| Edition {
        content: edition.content,
        version: edition.version.as_bytes().to_vec(),
    }))
}

/// Publish `content` to the cell, if it is still at `when`. Answers its new
/// version.
pub async fn publish(
    space: &LayerSpace,
    namespace: &str,
    cell: &str,
    content: Vec<u8>,
    when: Option<Vec<u8>>,
) -> Result<Vec<u8>, Error> {
    let version = perform(
        space,
        subject()
            .writer()
            .memory()
            .space(namespace)
            .cell(cell)
            .publish(content, when.map(Version::from)),
    )
    .await
    .map_err(remembered)?;
    Ok(version.as_bytes().to_vec())
}

/// Empty the cell, if it is still at `when`.
pub async fn retract(
    space: &LayerSpace,
    namespace: &str,
    cell: &str,
    when: Vec<u8>,
) -> Result<(), Error> {
    perform(
        space,
        subject()
            .user()
            .delete()
            .memory()
            .space(namespace)
            .cell(cell)
            .retract(Version::from(when)),
    )
    .await
    .map_err(remembered)
}

/// The cells of `namespace` that hold an edition.
pub async fn list(space: &LayerSpace, namespace: &str) -> Result<Vec<String>, Error> {
    perform(space, subject().reader().memory().space(namespace).list())
        .await
        .map_err(remembered)
}

/// The size of the blob under `key`, if the space holds it.
pub async fn size(space: &LayerSpace, key: &[u8]) -> Result<Option<u64>, Error> {
    perform(space, subject().reader().archive().blob().size(hash(key)?))
        .await
        .map_err(blobbed)
}

/// The bytes of the blob under `key`, from `start` up to `end`, or to its end.
pub async fn read(
    space: &LayerSpace,
    key: &[u8],
    start: u64,
    end: Option<u64>,
) -> Result<Vec<u8>, Error> {
    let length = end.map(|end| end.saturating_sub(start));
    let read: Capability<blob::Read> = subject()
        .reader()
        .archive()
        .blob()
        .invoke(blob::Read::range(hash(key)?, start, length));
    let mut reader = match perform(space, read).await {
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
pub async fn write(space: &LayerSpace, bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut sink = perform(space, subject().writer().archive().blob().write())
        .await
        .map_err(blobbed)?;
    sink.write_all(bytes).await.map_err(blobbed)?;
    let hash = sink.finish().await.map_err(blobbed)?;
    Ok(hash.as_bytes().to_vec())
}
