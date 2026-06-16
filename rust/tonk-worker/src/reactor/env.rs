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

use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Import, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::{Publish, Resolve};
use dialog_effects::space::Load;
use dialog_repository::RemoteSite;

/// Bound needed to load a repository via the profile.
pub trait LoadProvider: Provider<Load> + ConditionalSync + 'static {}
impl<T> LoadProvider for T where T: Provider<Load> + ConditionalSync + 'static {}

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
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}
impl<T> SelectProvider for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Bound needed to commit (`branch.commit(stream).perform`).
pub trait CommitProvider:
    Provider<Get>
    + Provider<Put>
    + Provider<Import>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}
impl<T> CommitProvider for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Bound needed to pull from upstream (`branch.pull().perform`).
pub trait PullProvider:
    Provider<Get>
    + Provider<Put>
    + Provider<Import>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Identify>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Resolve>>
    + ConditionalSync
    + 'static
{
}
impl<T> PullProvider for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Import>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static
{
}

/// Bound needed to push to upstream (`branch.push().perform`).
pub trait PushProvider:
    Provider<Get>
    + Provider<Put>
    + Provider<Resolve>
    + Provider<Publish>
    + Provider<Fork<RemoteSite, Get>>
    + Provider<Fork<RemoteSite, Put>>
    + Provider<Fork<RemoteSite, Resolve>>
    + Provider<Fork<RemoteSite, Publish>>
    + ConditionalSync
    + 'static
{
}
impl<T> PushProvider for T where
    T: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Publish>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Put>>
        + Provider<Fork<RemoteSite, Resolve>>
        + Provider<Fork<RemoteSite, Publish>>
        + ConditionalSync
        + 'static
{
}
