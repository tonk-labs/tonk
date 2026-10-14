//! Per-operation env trait aliases.
//!
//! Dialog's `perform` methods take `Env: Provider<X> + Provider<Y> + …`
//! soups that vary by operation. We collect each soup behind a
//! short trait alias so the reactor's leaf effects don't repeat
//! the union at every call site.
//!
//! One alias per dialog operation we wrap. If a leaf needs the
//! union of two (e.g. `acquire` uses both `LoadProvider` for the
//! repo load and `BranchOpenProvider` for the branch open), the
//! bound is `LoadProvider + BranchOpenProvider`.

use dialog_artifacts::{Preload, Speculation};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::{Attest, Identify};
use dialog_effects::blob::{Import as BlobImport, Read as BlobRead};
use dialog_effects::memory::{List, Publish, Resolve};
use dialog_effects::space::Load;
use dialog_repository::registry::RegistryEnv;
use dialog_repository::{Hydrate, PeersEnv, RemoteSite, ResolveEnv};

/// Bound needed to load a repository via the profile: loading opens the
/// repository's branch registry and upgrades its storage.
pub trait LoadProvider: Provider<Load> + Provider<List> + RegistryEnv + PeersEnv {}
impl<T> LoadProvider for T where T: Provider<Load> + Provider<List> + RegistryEnv + PeersEnv {}

/// Bound needed to open a branch on a repository.
pub trait BranchOpenProvider: Provider<Resolve> + ConditionalSync + 'static {}
impl<T> BranchOpenProvider for T where T: Provider<Resolve> + ConditionalSync + 'static {}

/// Bound needed for raw content-addressed block access — a
/// `LocalIndex` over the branch archive, reading tree nodes by
/// hash. `Put` is part of the `StorageBackend` bound even though
/// tree introspection only reads.
pub trait GetPutProvider: Provider<Get> + Provider<Put> + ConditionalSync + 'static {}
impl<T> GetPutProvider for T where T: Provider<Get> + Provider<Put> + ConditionalSync + 'static {}

/// Bound needed to run a query (`branch.query().select(q).perform`).
pub trait SelectProvider:
    Provider<Get>
    + Provider<BlobRead>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Identify>
    + Provider<Hydrate>
    + Provider<Preload>
    + Provider<Speculation>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}
impl<T> SelectProvider for T where
    T: Provider<Get>
        + Provider<dialog_effects::blob::Read>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Hydrate>
        + Provider<Preload>
        + Provider<Speculation>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Bound needed to commit a transaction (`branch.transaction()...commit().publish().perform`).
pub trait CommitProvider:
    Provider<Get>
    + Provider<BlobRead>
    + Provider<BlobImport>
    + Provider<dialog_effects::blob::Size>
    + Provider<Put>
    + Provider<Import>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Attest>
    + Provider<Hydrate>
    + Provider<Preload>
    + Provider<Speculation>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}
impl<T> CommitProvider for T where
    T: Provider<Get>
        + Provider<BlobRead>
        + Provider<BlobImport>
        + Provider<dialog_effects::blob::Size>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Attest>
        + Provider<Hydrate>
        + Provider<Preload>
        + Provider<Speculation>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Bound needed to pull from upstream (`branch.pull().perform`): the
/// registry the branch's upstreams resolve from, and the host's
/// connections to the peers they live at.
pub trait PullProvider:
    ResolveEnv
    + Provider<Fork<RemoteSite, Get>>
    + Provider<dialog_effects::blob::Read>
    + Provider<dialog_effects::blob::Import>
    + Provider<Fork<RemoteSite, dialog_effects::blob::Read>>
{
}
impl<T> PullProvider for T where
    T: ResolveEnv
        + Provider<Fork<RemoteSite, Get>>
        + Provider<dialog_effects::blob::Read>
        + Provider<dialog_effects::blob::Import>
        + Provider<Fork<RemoteSite, dialog_effects::blob::Read>>
{
}

/// Bound needed to push to upstream (`branch.push().perform`).
pub trait PushProvider:
    ResolveEnv
    + Provider<BlobRead>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Put>>
    + Provider<Fork<RemoteSite, Publish>>
    + Provider<Fork<RemoteSite, BlobImport>>
    + Provider<Fork<RemoteSite, BlobRead>>
{
}
impl<T> PushProvider for T where
    T: ResolveEnv
        + Provider<BlobRead>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Put>>
        + Provider<Fork<RemoteSite, Publish>>
        + Provider<Fork<RemoteSite, BlobImport>>
        + Provider<Fork<RemoteSite, BlobRead>>
{
}
