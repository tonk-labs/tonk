//! Peers and workers over the CLI's identity.
//!
//! The CLI opens its peer once and hands `(peer, worker)` pairs around.
//! Its data lives in more than one place, so a worker is often rebased:
//! the same credential and storage as the peer, with a different base
//! directory for its space names.

use anyhow::{Context as _, Result};
use dialog_capability::Subject;
use dialog_effects::storage::Directory;
use dialog_peer::Peer;
use dialog_storage::provider::storage::NativeSpace;

/// The CLI's peer: its identity over the native storage.
pub type NativePeer = Peer<NativeSpace>;

/// A worker of the CLI's peer.
pub type NativeSession = Peer<NativeSpace>;

/// A peer over the same credential, home, storage and runtime as `peer`,
/// with `base` as the directory its space names resolve against.
pub async fn peer_for(peer: &NativePeer, base: Directory) -> Result<NativePeer> {
    let builder = Peer::open(peer.home().clone())
        .credential(peer.credential().clone())
        .storage(peer.storage().clone())
        .network(peer.network().clone())
        .runtime(peer.runtime().clone())
        .base(base);
    match peer.branch() {
        Some(name) => builder.branch(name),
        None => builder.ephemeral(),
    }
    .await
    .context("failed to build peer")
}

/// A worker of `peer` rebased at `base`, under `context`, allowed
/// everything the peer holds.
pub async fn session_for(
    peer: &NativePeer,
    base: Directory,
    context: &[u8],
) -> Result<NativeSession> {
    derive_session(&peer_for(peer, base).await?, context).await
}

/// A worker of `peer` under `context`, allowed everything the peer holds.
pub async fn derive_session(peer: &NativePeer, context: &[u8]) -> Result<NativeSession> {
    peer.worker(context)
        .allow(Subject::any())
        .await
        .context("failed to build worker")
}
